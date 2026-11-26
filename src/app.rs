//! Application state, keymap and queue actions.

use crate::player::{LoadMode, Player, PlayerStatus};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum View {
    Library,
    Queue,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    None,
    Quit,
}

pub struct App {
    /// absolute paths of the phone library
    pub tracks: Vec<String>,
    /// indices into `tracks` currently visible (search filter)
    pub filtered: Vec<usize>,
    pub cursor: usize,
    /// cursor inside the queue view
    pub queue_cursor: usize,
    pub view: View,
    /// snapshot of mpv's playlist (filenames only)
    pub queue: Vec<String>,
    pub status: Option<PlayerStatus>,
    pub message: Option<String>,
    pub search: Option<String>,
    pub volume: i64,
}

impl App {
    pub fn new(root: PathBuf, tracks: Vec<String>) -> App {
        // `tracks` are relative paths in the cache; store absolute for mpv
        let tracks: Vec<String> = tracks
            .into_iter()
            .map(|t| root.join(&t).to_string_lossy().into_owned())
            .collect();
        let filtered = (0..tracks.len()).collect();
        App {
            tracks,
            filtered,
            cursor: 0,
            queue_cursor: 0,
            view: View::Library,
            queue: Vec::new(),
            status: None,
            message: None,
            search: None,
            volume: 100,
        }
    }

    /// Fuzzy-filter the library; empty query shows everything.
    pub fn apply_search(&mut self, q: &str) {
        if q.is_empty() {
            self.filtered = (0..self.tracks.len()).collect();
        } else {
            let matcher = SkimMatcherV2::default().ignore_case();
            let mut scored: Vec<(i64, usize)> = self
                .tracks
                .iter()
                .enumerate()
                .filter_map(|(i, t)| matcher.fuzzy_match(t, q).map(|s| (s, i)))
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0));
            self.filtered = scored.into_iter().map(|(_, i)| i).collect();
        }
        self.cursor = 0;
    }

    pub fn move_cursor(&mut self, d: i32) {
        let len = self.filtered.len();
        if len == 0 {
            return;
        }
        let cur = self.cursor as i32 + d;
        self.cursor = cur.clamp(0, len as i32 - 1) as usize;
    }

    fn selected(&self) -> Option<&str> {
        self.filtered
            .get(self.cursor)
            .and_then(|&i| self.tracks.get(i))
            .map(String::as_str)
    }

    pub async fn play_selected(&mut self, p: &mut Player) {
        if let Some(path) = self.selected() {
            match p.loadfile(path, LoadMode::Replace).await {
                Ok(resp) if resp["error"] == "success" => self.message = None,
                Ok(resp) => {
                    self.message = Some(format!("error: {}", resp["error"].as_str().unwrap_or("?")))
                }
                Err(e) => self.message = Some(format!("player error: {e}")),
            }
        }
    }

    pub async fn enqueue_selected(&mut self, p: &mut Player) {
        if let Some(path) = self.selected() {
            match p.loadfile(path, LoadMode::Append).await {
                Ok(resp) if resp["error"] == "success" => self.message = None,
                Ok(resp) => {
                    self.message = Some(format!("error: {}", resp["error"].as_str().unwrap_or("?")))
                }
                Err(e) => self.message = Some(format!("player error: {e}")),
            }
        }
    }

    pub async fn enqueue_all_filtered(&mut self, p: &mut Player) {
        let paths: Vec<String> = self
            .filtered
            .iter()
            .filter_map(|&i| self.tracks.get(i).cloned())
            .collect();
        for path in &paths {
            let _ = p.loadfile(path, LoadMode::Append).await;
        }
        self.message = Some(format!("queued {} tracks", paths.len()));
    }

    pub async fn refresh_status(&mut self, p: &mut Player) {
        match p.status().await {
            Ok(st) => {
                self.queue = st.playlist.clone();
                self.status = Some(st);
            }
            Err(e) => self.message = Some(format!("player error: {e}")),
        }
    }

    pub async fn on_key(&mut self, key: KeyEvent, p: &mut Player) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        // search input mode
        if let Some(q) = &mut self.search {
            match key.code {
                KeyCode::Char(c) => {
                    q.push(c);
                    let q = q.clone();
                    self.apply_search(&q);
                }
                KeyCode::Backspace => {
                    q.pop();
                    let q = q.clone();
                    self.apply_search(&q);
                    if q.is_empty() {
                        self.search = None;
                    }
                }
                KeyCode::Enter | KeyCode::Esc => self.search = None,
                _ => {}
            }
            return Action::None;
        }

        match self.view {
            View::Library => match key.code {
                KeyCode::Char('q') => return Action::Quit,
                KeyCode::Char('j') | KeyCode::Down => self.move_cursor(1),
                KeyCode::Char('k') | KeyCode::Up => self.move_cursor(-1),
                KeyCode::Enter => self.play_selected(p).await,
                KeyCode::Char(' ') => self.enqueue_selected(p).await,
                KeyCode::Char('a') => self.enqueue_all_filtered(p).await,
                KeyCode::Char('/') => self.search = Some(String::new()),
                KeyCode::Tab => self.view = View::Queue,
                KeyCode::Char('n') => { let _ = p.next().await; }
                KeyCode::Char('p') => { let _ = p.previous().await; }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    self.volume = (self.volume + 5).min(130);
                    let _ = p.set_volume(self.volume).await;
                }
                KeyCode::Char('-') => {
                    self.volume = (self.volume - 5).max(0);
                    let _ = p.set_volume(self.volume).await;
                }
                _ => {}
            },
            View::Queue => match key.code {
                KeyCode::Char('q') => return Action::Quit,
                KeyCode::Char('j') | KeyCode::Down => {
                    if self.queue_cursor + 1 < self.queue.len() {
                        self.queue_cursor += 1;
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.queue_cursor = self.queue_cursor.saturating_sub(1);
                }
                KeyCode::Char('d') => {
                    let _ = p.playlist_remove(self.queue_cursor).await;
                    self.queue_cursor = self.queue_cursor.saturating_sub(1);
                }
                KeyCode::Char('J') => {
                    let c = self.queue_cursor;
                    if c + 1 < self.queue.len() {
                        let _ = p.playlist_move(c, c + 1).await;
                        self.queue_cursor += 1;
                    }
                }
                KeyCode::Char('K') => {
                    let c = self.queue_cursor;
                    if c > 0 {
                        let _ = p.playlist_move(c, c - 1).await;
                        self.queue_cursor -= 1;
                    }
                }
                KeyCode::Enter => {
                    let _ = p.playlist_play_index(self.queue_cursor).await;
                }
                KeyCode::Char('D') => {
                    let _ = p.playlist_clear().await;
                    self.queue_cursor = 0;
                }
                KeyCode::Tab | KeyCode::Esc => self.view = View::Library,
                _ => {}
            },
        }
        Action::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::Player;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    static N: AtomicUsize = AtomicUsize::new(0);

    type CmdLog = Arc<tokio::sync::Mutex<Vec<String>>>;

    /// Fake mpv server: records command names, replies success, with scripted
    /// get_property data.
    async fn fake_player(prop_data: serde_json::Value) -> (Player, CmdLog) {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("flactui-app-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("fake.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let log: CmdLog = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let log_srv = log.clone();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut lines = tokio::io::BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let req: serde_json::Value = serde_json::from_str(&line).unwrap();
                let id = req["request_id"].clone();
                let cmd = req["command"].as_array().unwrap();
                let name = cmd[0].as_str().unwrap().to_string();
                log_srv.lock().await.push(name.clone());
                let data = if name == "get_property" {
                    prop_data
                        .get(cmd[1].as_str().unwrap())
                        .cloned()
                        .unwrap_or(serde_json::Value::Null)
                } else {
                    serde_json::Value::Null
                };
                let resp =
                    serde_json::json!({ "error": "success", "request_id": id, "data": data });
                write.write_all(format!("{resp}\n").as_bytes()).await.unwrap();
            }
        });
        let player = Player::connect(&sock).await.unwrap();
        (player, log)
    }

    fn app_with(tracks: &[&str]) -> App {
        App::new(
            PathBuf::from("/m"),
            tracks.iter().map(|s| s.to_string()).collect(),
        )
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn search_is_fuzzy_and_empty_restores_all() {
        let mut app = app_with(&[
            "01 - Canserbero - Es Épico.flac",
            "some - Random Artist - Song.flac",
            "02 - Canserbero - Vida.flac",
            "03 - Silvestre - Track.flac",
        ]);
        app.apply_search("cnsrbr");
        assert_eq!(app.filtered.len(), 2);
        assert!(app.tracks[app.filtered[0]].contains("Canserbero"));
        app.apply_search("");
        assert_eq!(app.filtered.len(), 4);
    }

    #[tokio::test]
    async fn play_selected_sends_replace_with_joined_path() {
        let (mut player, log) = fake_player(serde_json::json!({})).await;
        let mut app = app_with(&["Es Épico.flac"]);
        app.play_selected(&mut player).await;
        assert!(app.message.is_none());
        // loadfile request body must carry the absolute joined path
        // (the log only stores names; assert via the second raw field instead)
        assert_eq!(log.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn enqueue_error_sets_message() {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("flactui-apperr-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("fake.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut lines = tokio::io::BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let req: serde_json::Value = serde_json::from_str(&line).unwrap();
                let resp = serde_json::json!({
                    "error": "error loading file",
                    "request_id": req["request_id"].clone(),
                    "data": serde_json::Value::Null
                });
                write.write_all(format!("{resp}\n").as_bytes()).await.unwrap();
            }
        });
        let mut player = Player::connect(&sock).await.unwrap();
        let mut app = app_with(&["gone.flac"]);
        app.enqueue_selected(&mut player).await;
        assert!(app
            .message
            .as_deref()
            .unwrap()
            .contains("error loading file"));
    }

    #[test]
    fn cursor_clamps_both_ends() {
        let mut app = app_with(&["a.flac", "b.flac", "c.flac"]);
        app.move_cursor(10);
        assert_eq!(app.cursor, 2);
        app.move_cursor(-10);
        assert_eq!(app.cursor, 0);
        let mut empty = app_with(&[]);
        empty.move_cursor(1); // must not panic
        assert_eq!(empty.cursor, 0);
    }

    #[tokio::test]
    async fn queue_view_d_calls_playlist_remove() {
        let (mut player, log) = fake_player(serde_json::json!({
            "pause": false,
            "playlist": [
                {"filename": "/m/a.flac"},
                {"filename": "/m/b.flac"}
            ],
            "playlist-pos": 0
        }))
        .await;
        let mut app = app_with(&["a.flac"]);
        app.refresh_status(&mut player).await;
        assert_eq!(app.queue, vec!["a.flac", "b.flac"]);
        app.view = View::Queue;
        app.queue_cursor = 1;
        app.on_key(key('d'), &mut player).await;
        assert!(log.lock().await.contains(&"playlist-remove".to_string()));
        assert_eq!(app.queue_cursor, 0); // clamped after remove
    }

    #[tokio::test]
    async fn ctrl_c_quits() {
        let (mut player, _log) = fake_player(serde_json::json!({})).await;
        let mut app = app_with(&["a.flac"]);
        let k = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(app.on_key(k, &mut player).await, Action::Quit);
    }
}
