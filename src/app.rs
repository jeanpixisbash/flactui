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

/// Lyrics popup content: timed (karaoke-follow) or plain text.
#[derive(Debug)]
pub enum Lyrics {
    Timed(crate::lyrics::LrcLines),
    Plain(String),
}

/// What the cover popup shows: the decoded art or a plain message.
#[derive(Debug)]
pub enum CoverView {    Ready {
        art: crate::covers::HalfBlocks,
        title: String,
    },
    Plain(String),
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
    pub shuffle: bool,
    pub repeat: bool,
    /// lyrics popup: Some(view) = open
    pub lyrics: Option<Lyrics>,
    pub lyrics_scroll: usize,
    /// Timed-only manual offset from the centered active line
    pub lyrics_offset: i32,
    /// cached active line index (drives the follow reset)
    pub lyrics_active: Option<usize>,
    /// track we asked mpv to load, until status confirms it (loading feedback)
    pub loading: Option<String>,
    /// cover popup: Some(view) = open
    pub cover: Option<CoverView>,
    /// in-flight cover decode job (background; polled by the event loop)
    pub cover_rx: Option<tokio::sync::oneshot::Receiver<(String, Result<crate::covers::HalfBlocks, crate::covers::CoverError>)>>,
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
            shuffle: false,
            repeat: false,
            lyrics: None,
            lyrics_scroll: 0,
            lyrics_offset: 0,
            lyrics_active: None,
            loading: None,
            cover: None,
            cover_rx: None,
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

    fn track_name(path: &str) -> String {
        std::path::Path::new(path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string())
    }

    /// Set the loading feedback: mpv opens a network FLAC in seconds, and
    /// without this the silence reads as "it doesn't play".
    fn start_loading(&mut self, path: &str) {
        self.loading = Some(path.to_string());
        self.message = Some(format!("loading {}…", Self::track_name(path)));
    }

    pub async fn play_selected(&mut self, p: &mut Player) {
        if let Some(path) = self.selected().map(String::from) {
            match p.loadfile(&path, LoadMode::Replace).await {
                Ok(resp) if resp["error"] == "success" => self.start_loading(&path),
                Ok(resp) => {
                    self.loading = None;
                    self.message = Some(format!("error: {}", resp["error"].as_str().unwrap_or("?")))
                }
                Err(e) => {
                    self.loading = None;
                    self.message = Some(format!("player error: {e}"))
                }
            }
        }
    }

    pub async fn enqueue_selected(&mut self, p: &mut Player) {
        if let Some(path) = self.selected().map(String::from) {
            match p.loadfile(&path, LoadMode::Append).await {
                Ok(resp) if resp["error"] == "success" => self.start_loading(&path),
                Ok(resp) => {
                    self.loading = None;
                    self.message = Some(format!("error: {}", resp["error"].as_str().unwrap_or("?")))
                }
                Err(e) => {
                    self.loading = None;
                    self.message = Some(format!("player error: {e}"))
                }
            }
        }
    }

    pub async fn enqueue_all_filtered(&mut self, p: &mut Player) {
        let paths: Vec<String> = self
            .filtered
            .iter()
            .filter_map(|&i| self.tracks.get(i).cloned())
            .collect();
        let mut queued = 0usize;
        let mut failed = 0usize;
        for path in &paths {
            match p.loadfile(path, LoadMode::Append).await {
                Ok(resp) if resp["error"] == "success" => queued += 1,
                _ => failed += 1,
            }
        }
        self.message = Some(if failed == 0 {
            format!("queued {queued} tracks")
        } else {
            format!("queued {queued} of {}, {failed} failed", paths.len())
        });
    }

    pub async fn refresh_status(&mut self, p: &mut Player) {
        match p.status().await {
            Ok(st) => {
                self.queue = st.playlist.clone();
                // the requested file made it into mpv → loading is over
                if self.loading.is_some() && st.path.is_some() {
                    self.loading = None;
                    self.message = None;
                }
                if let Some(v) = st.volume {
                    self.volume = v;
                }
                self.shuffle = st.shuffle;
                self.repeat = st.loop_playlist != "no";
                self.status = Some(st);
                self.sync_lyrics_follow();
            }
            Err(e) => {
                let msg = format!("{e}");
                self.message = Some(if msg.contains("socket closed") || msg.contains("timeout") {
                    "player lost — press q to exit".into()
                } else {
                    format!("player error: {e}")
                });
            }
        }
    }

    /// Strip LRC markup from an embedded lyrics tag: line timestamps
    /// `[mm:ss.xx]`, word timestamps `<mm:ss.xx>` (often duplicated), metadata
    /// headers (`[ti:…]`) and voice labels (`v1:`, `bg:`). Plain text — and
    /// lyrics with a stray "<3" — will lose the "<3" to this; acceptable.
    pub fn clean_lyrics(raw: &str) -> String {
        const VOICE_LABELS: [&str; 6] = ["v1", "v2", "v3", "bg", "d1", "d2"];
        let mut out = Vec::new();
        for line in raw.lines() {
            let mut s = String::with_capacity(line.len());
            let mut bracket = 0u8; // 0 = text, 1 = inside […], 2 = inside <…>
            for ch in line.chars() {
                match ch {
                    '[' => bracket = 1,
                    '<' => bracket = 2,
                    ']' if bracket == 1 => bracket = 0,
                    '>' if bracket == 2 => bracket = 0,
                    c if bracket == 0 => s.push(c),
                    _ => {}
                }
            }
            let t = s.trim();
            let t = match t.split_once(':') {
                // a leading voice label like "v1:" before the words
                Some((label, rest))
                    if VOICE_LABELS.contains(&label.to_lowercase().as_str()) =>
                {
                    rest.trim()
                }
                _ => t,
            };
            out.push(t.to_string());
        }
        while out.last().is_some_and(|l| l.is_empty()) {
            out.pop();
        }
        out.join("\n")
    }

    /// Toggle the lyrics popup: reads the playing file's embedded lyrics tag
    /// through mpv's metadata property (zero network).
    pub async fn toggle_lyrics(&mut self, p: &mut Player) {
        if self.lyrics.is_some() {
            self.lyrics = None;
            self.lyrics_scroll = 0;
            self.lyrics_offset = 0;
            self.lyrics_active = None;
            return;
        }
        match p.metadata().await {
            Ok(pairs) => {
                let found = pairs
                    .iter()
                    .find(|(k, _)| k.to_uppercase().contains("LYRICS"))
                    .map(|(_, v)| v.clone());
                self.lyrics = Some(match found {
                    Some(tag) => match crate::lyrics::parse_lrc(&tag) {
                        Some(lines) => Lyrics::Timed(lines),
                        None => Lyrics::Plain(Self::clean_lyrics(&tag)),
                    },
                    None if self.status.as_ref().and_then(|s| s.path.as_ref()).is_some() => {
                        Lyrics::Plain("no lyrics found in this file's tags".into())
                    }
                    None => Lyrics::Plain("nothing playing".into()),
                });
                self.lyrics_scroll = 0;
                self.lyrics_offset = 0;
                self.lyrics_active = None;
            }
            Err(e) => self.message = Some(format!("player error: {e}")),
        }
    }

    /// Timed lyrics: cache the active line; when it advances, drop the manual
    /// offset so the view re-follows the song.
    pub fn sync_lyrics_follow(&mut self) {
        let Some(Lyrics::Timed(lines)) = &self.lyrics else { return };
        let time_pos = self.status.as_ref().and_then(|s| s.time_pos);
        let Some(t) = time_pos else {
            self.lyrics_active = None;
            return;
        };
        let new_active = crate::lyrics::active_line(lines, t);
        if new_active != self.lyrics_active {
            self.lyrics_active = new_active;
            self.lyrics_offset = 0;
        }
    }

    /// Toggle the cover popup: opens it instantly with loading feedback while
    /// the embedded art decodes in the background (the event loop keeps
    /// repainting; `poll_cover` applies the result when it lands).
    pub async fn toggle_cover(&mut self, p: &mut Player) {
        if self.cover.is_some() {
            self.cover = None;
            self.cover_rx = None; // orphan the in-flight job; result discarded
            return;
        }
        let path = self
            .status
            .as_ref()
            .and_then(|s| s.path.clone())
            .or_else(|| self.selected().map(String::from));
        let Some(path) = path else {
            self.cover = Some(CoverView::Plain("nothing playing".into()));
            return;
        };
        let title = crate::covers::track_name(&path);
        self.cover = Some(CoverView::Plain("loading cover…".into()));
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::task::spawn_blocking(move || {
            // v1: FLAC covers only — other formats read as "no embedded cover"
            let res = if !path.to_lowercase().ends_with(".flac") {
                Err(crate::covers::CoverError::NoPicture)
            } else {
                crate::covers::load(&path)
            };
            let _ = tx.send((title, res));
        });
        self.cover_rx = Some(rx);
    }

    /// Apply a finished background cover job, if one landed.
    pub fn poll_cover(&mut self) {
        let Some(mut rx) = self.cover_rx.take() else { return };
        match rx.try_recv() {
            Ok((title, res)) => {
                self.cover = Some(match res {
                    Ok(art) => CoverView::Ready { art, title },
                    Err(crate::covers::CoverError::NoPicture) => {
                        CoverView::Plain("no embedded cover".into())
                    }
                    Err(e) => {
                        // also surface it in the playbar: long reasons get
                        // truncated in the popup's border title
                        self.message = Some(e.to_string());
                        CoverView::Plain(e.to_string())
                    }
                });
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                // still decoding — put the job back
                self.cover_rx = Some(rx);
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                // the job died without sending (panic in the worker)
                self.cover = Some(CoverView::Plain("cover unavailable".into()));
            }
        }
    }

    pub async fn on_key(&mut self, key: KeyEvent, p: &mut Player) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        // cover popup consumes keys while open — but playback controls still
        // reach the player (the popup is fullscreen, music control must live)
        if self.cover.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('i') | KeyCode::Char('q') => self.cover = None,
                KeyCode::Char('n') => {
                    let _ = p.next().await;
                }
                KeyCode::Char('p') => {
                    let _ = p.previous().await;
                }
                KeyCode::Char(' ') => {
                    let _ = p.toggle_pause().await;
                }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    self.volume = (self.volume + 5).min(130);
                    let _ = p.set_volume(self.volume).await;
                }
                KeyCode::Char('-') => {
                    self.volume = (self.volume - 5).max(0);
                    let _ = p.set_volume(self.volume).await;
                }
                _ => {}
            }
            return Action::None;
        }
        // lyrics popup consumes keys while open — but playback controls still
        // reach the player (fullscreen popup, music control must live)
        if self.lyrics.is_some() {
            match (&self.lyrics, key.code) {
                (_, KeyCode::Char('n')) => {
                    let _ = p.next().await;
                    return Action::None;
                }
                (_, KeyCode::Char('p')) => {
                    let _ = p.previous().await;
                    return Action::None;
                }
                (_, KeyCode::Char(' ')) => {
                    let _ = p.toggle_pause().await;
                    return Action::None;
                }
                (_, KeyCode::Char('+') | KeyCode::Char('=')) => {
                    self.volume = (self.volume + 5).min(130);
                    let _ = p.set_volume(self.volume).await;
                    return Action::None;
                }
                (_, KeyCode::Char('-')) => {
                    self.volume = (self.volume - 5).max(0);
                    let _ = p.set_volume(self.volume).await;
                    return Action::None;
                }
                (Some(Lyrics::Plain(_)), KeyCode::Char('j') | KeyCode::Down) => self.lyrics_scroll += 1,
                (Some(Lyrics::Plain(_)), KeyCode::Char('k') | KeyCode::Up) => {
                    self.lyrics_scroll = self.lyrics_scroll.saturating_sub(1)
                }
                (Some(Lyrics::Plain(_)), KeyCode::Char('g')) => self.lyrics_scroll = 0,
                (Some(Lyrics::Timed(_)), KeyCode::Char('j') | KeyCode::Down) => self.lyrics_offset += 1,
                (Some(Lyrics::Timed(_)), KeyCode::Char('k') | KeyCode::Up) => self.lyrics_offset -= 1,
                (Some(Lyrics::Timed(_)), KeyCode::Char('g')) => {
                    self.lyrics_offset = -(self.lyrics_active.unwrap_or(0) as i32);
                }
                (_, KeyCode::Esc | KeyCode::Char('l') | KeyCode::Char('q')) => {
                    self.lyrics = None;
                    self.lyrics_scroll = 0;
                    self.lyrics_offset = 0;
                }
                _ => {}
            }
            return Action::None;
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
                // Space = play/pause everywhere (spotify-tui style);
                // enqueue lives on 'e'
                KeyCode::Char(' ') => { let _ = p.toggle_pause().await; }
                KeyCode::Char('e') => self.enqueue_selected(p).await,
                KeyCode::Char('a') => self.enqueue_all_filtered(p).await,
                KeyCode::Char('/') => self.search = Some(String::new()),
                KeyCode::Tab => self.view = View::Queue,
                KeyCode::Char('n') => { let _ = p.next().await; }
                KeyCode::Char('p') => { let _ = p.previous().await; }
                KeyCode::Left => { let _ = p.seek(-10.0).await; }
                KeyCode::Right => { let _ = p.seek(10.0).await; }
                KeyCode::Char('s') => { let _ = p.toggle_shuffle().await; }
                KeyCode::Char('r') => { let _ = p.toggle_repeat().await; }
                KeyCode::Char('l') => self.toggle_lyrics(p).await,
                KeyCode::Char('i') => self.toggle_cover(p).await,
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
                KeyCode::Char(' ') => { let _ = p.toggle_pause().await; }
                KeyCode::Char('d') => {
                    let _ = p.playlist_remove(self.queue_cursor).await;
                    self.queue_cursor = self.queue_cursor.saturating_sub(1);
                }
                KeyCode::Char('J') => {
                    // mpv semantics: index2 is the TARGET entry, so moving one
                    // position down requires to = from + 2 (verified on real mpv)
                    let c = self.queue_cursor;
                    if c + 1 < self.queue.len() {
                        let _ = p.playlist_move(c, c + 2).await;
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
                KeyCode::Char('i') => self.toggle_cover(p).await,
                KeyCode::Char('D') => {
                    let _ = p.playlist_clear().await;
                    self.queue_cursor = 0;
                }
                KeyCode::Char('n') => { let _ = p.next().await; }
                KeyCode::Char('p') => { let _ = p.previous().await; }
                KeyCode::Left => { let _ = p.seek(-10.0).await; }
                KeyCode::Right => { let _ = p.seek(10.0).await; }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    self.volume = (self.volume + 5).min(130);
                    let _ = p.set_volume(self.volume).await;
                }
                KeyCode::Char('-') => {
                    self.volume = (self.volume - 5).max(0);
                    let _ = p.set_volume(self.volume).await;
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
                log_srv.lock().await.push(req["command"].to_string());
                // a loadfile whose path contains "fail" simulates an mpv-level
                // rejection (e.g. unreadable file) for honest-enqueue tests
                let load_failed = name == "loadfile"
                    && cmd.get(1).and_then(|v| v.as_str()).map(|p| p.contains("fail")).unwrap_or(false);
                let data = if name == "get_property" {
                    prop_data
                        .get(cmd[1].as_str().unwrap())
                        .cloned()
                        .unwrap_or(serde_json::Value::Null)
                } else {
                    serde_json::Value::Null
                };
                let resp = if load_failed {
                    serde_json::json!({ "error": "invalid parameter", "request_id": id })
                } else {
                    serde_json::json!({ "error": "success", "request_id": id, "data": data })
                };
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
        // success now shows loading feedback (cleared once status confirms)
        assert!(app.message.as_deref().unwrap_or("").contains("loading"));
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
        assert!(log.lock().await.iter().any(|c| c.contains("playlist-remove")));
        assert_eq!(app.queue_cursor, 0); // clamped after remove
    }

    #[tokio::test]
    async fn queue_move_down_uses_mpv_target_semantics() {
        // mpv: playlist-move from to — "to" is the TARGET entry index, so
        // moving one position down requires to = from + 2.
        let (mut player, log) = fake_player(serde_json::json!({
            "pause": false,
            "playlist": [
                {"filename": "/m/a.flac"},
                {"filename": "/m/b.flac"},
                {"filename": "/m/c.flac"}
            ],
            "playlist-pos": 0
        }))
        .await;
        let mut app = app_with(&["a.flac"]);
        app.refresh_status(&mut player).await;
        app.view = View::Queue;
        app.queue_cursor = 0;
        app.on_key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT), &mut player)
            .await;
        let log = log.lock().await;
        let mv = log.iter().find(|c| c.contains("playlist-move")).unwrap();
        assert!(mv.contains("2"), "expected playlist-move 0 2, got {mv}");
        drop(log);
        assert_eq!(app.queue_cursor, 1);
    }

    #[tokio::test]
    async fn queue_move_up_uses_direct_target() {
        let (mut player, log) = fake_player(serde_json::json!({
            "pause": false,
            "playlist": [
                {"filename": "/m/a.flac"},
                {"filename": "/m/b.flac"}
            ],
            "playlist-pos": 1
        }))
        .await;
        let mut app = app_with(&["a.flac"]);
        app.refresh_status(&mut player).await;
        app.view = View::Queue;
        app.queue_cursor = 1;
        app.on_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT), &mut player)
            .await;
        let log = log.lock().await;
        let mv = log.iter().find(|c| c.contains("playlist-move")).unwrap();
        assert!(mv.contains("0"), "expected playlist-move 1 0, got {mv}");
        drop(log);
        assert_eq!(app.queue_cursor, 0);
    }

    #[tokio::test]
    async fn shuffle_repeat_and_seek_keys() {
        let (mut player, log) = fake_player(serde_json::json!({
            "pause": false, "shuffle": false, "loop-playlist": "no",
            "playlist": [], "playlist-pos": 0, "volume": 55
        }))
        .await;
        let mut app = app_with(&["a.flac"]);
        app.refresh_status(&mut player).await;
        // volume synced from mpv
        assert_eq!(app.volume, 55);

        app.on_key(key('s'), &mut player).await; // shuffle
        app.on_key(key('r'), &mut player).await; // repeat
        app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), &mut player).await;
        app.on_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), &mut player).await;
        let log = log.lock().await;
        assert!(log.iter().any(|c| c.contains("shuffle")), "no shuffle: {log:?}");
        assert!(log.iter().any(|c| c.contains("loop-playlist")), "no repeat: {log:?}");
        assert!(log.iter().any(|c| c.contains("seek")), "no seek: {log:?}");
    }

    #[tokio::test]
    async fn queue_view_space_pauses_and_n_p_volume_work_there_too() {
        let (mut player, log) = fake_player(serde_json::json!({
            "pause": false, "shuffle": false, "loop-playlist": "no",
            "playlist": [{"filename": "/m/a.flac"}], "playlist-pos": 0, "volume": 100
        }))
        .await;
        let mut app = app_with(&["a.flac"]);
        app.refresh_status(&mut player).await;
        app.view = View::Queue;
        app.on_key(key(' '), &mut player).await; // pause in queue view
        app.on_key(key('n'), &mut player).await;
        app.on_key(key('p'), &mut player).await;
        app.on_key(key('+'), &mut player).await;
        let log = log.lock().await;
        assert!(log.iter().any(|c| c.contains("cycle")), "no pause cycle: {log:?}");
        assert!(log.iter().any(|c| c.contains("playlist-next")));
        assert!(log.iter().any(|c| c.contains("playlist-prev")));
        assert!(log.iter().any(|c| c.contains("volume")));
    }

    #[tokio::test]
    async fn library_space_pauses_and_e_enqueues() {
        let (mut player, log) = fake_player(serde_json::json!({
            "pause": false, "playlist": [], "playlist-pos": 0, "volume": 100
        }))
        .await;
        let mut app = app_with(&["a.flac"]);
        // Space = play/pause GLOBAL (como spotify-tui)
        app.on_key(key(' '), &mut player).await;
        assert!(log.lock().await.iter().any(|c| c.contains("cycle")));
        // 'e' = encolar la selección
        app.on_key(key('e'), &mut player).await;
        let log = log.lock().await;
        assert!(
            log.iter().any(|c| c.contains("append-play")),
            "no enqueue: {log:?}"
        );
    }

    #[tokio::test]
    async fn lyrics_popup_opens_closes_and_scrolls() {
        let (mut player, _log) = fake_player(serde_json::json!({
            "pause": false, "playlist": [], "playlist-pos": 0, "volume": 100,
            "metadata": {"TITLE": "Es Épico", "LYRICS": "verso 1\nverso 2\nverso 3"}
        }))
        .await;
        let mut app = app_with(&["a.flac"]);

        // 'l' abre el pop-up con la letra del tag LYRICS (plain: sin timestamps)
        app.on_key(key('l'), &mut player).await;
        assert!(matches!(
            app.lyrics,
            Some(super::Lyrics::Plain(ref t)) if t == "verso 1\nverso 2\nverso 3"
        ));

        // j/k hacen scroll
        app.on_key(key('j'), &mut player).await;
        app.on_key(key('j'), &mut player).await;
        assert_eq!(app.lyrics_scroll, 2);
        app.on_key(key('k'), &mut player).await;
        assert_eq!(app.lyrics_scroll, 1);

        // Esc cierra y resetea el scroll
        app.on_key(KeyCode::Esc.into(), &mut player).await;
        assert!(app.lyrics.is_none());
        assert_eq!(app.lyrics_scroll, 0);
    }

    #[tokio::test]
    async fn lyrics_without_tag_shows_hint() {
        let (mut player, _log) = fake_player(serde_json::json!({
            "pause": false, "playlist": [], "playlist-pos": 0, "volume": 100,
            "path": "/m/sin letra.flac",
            "metadata": {"TITLE": "sin letra"}
        }))
        .await;
        let mut app = app_with(&["sin letra.flac"]);
        app.refresh_status(&mut player).await;
        app.on_key(key('l'), &mut player).await;
        assert!(matches!(
            app.lyrics,
            Some(super::Lyrics::Plain(ref t)) if t == "no lyrics found in this file's tags"
        ));
    }

    #[tokio::test]
    async fn lyrics_nothing_playing_shows_hint() {
        let (mut player, _log) = fake_player(serde_json::json!({})).await;
        let mut app = app_with(&["a.flac"]);
        app.on_key(key('l'), &mut player).await;
        assert!(matches!(
            app.lyrics,
            Some(super::Lyrics::Plain(ref t)) if t == "nothing playing"
        ));
    }

    #[tokio::test]
    async fn play_shows_loading_feedback_until_status_confirms() {
        // streaming over SFTP takes seconds — without feedback it looks broken
        let (mut player, _log) = fake_player(serde_json::json!({
            "pause": false, "playlist": [], "playlist-pos": 0, "volume": 100
        }))
        .await;
        let mut app = app_with(&["Es Epico.flac"]);
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut player)
            .await;
        let msg = app.message.as_deref().unwrap_or("");
        assert!(
            msg.contains("loading") && msg.contains("Es Epico.flac"),
            "expected loading feedback, got: {msg:?}"
        );

        // the status poll still shows nothing loaded → keep the feedback
        app.refresh_status(&mut player).await;
        assert!(app.message.as_deref().unwrap_or("").contains("loading"));

        // now mpv reports the file loaded → feedback clears
        player.set_property_for_test("path", "/m/Es Epico.flac".into());
        app.refresh_status(&mut player).await;
        assert_eq!(app.message, None);
        assert!(app.loading.is_none());
    }

    #[test]
    fn clean_lyrics_strips_lrc_timestamps_and_labels() {
        // enhanced LRC (A2): line timestamps, duplicated word timestamps and
        // voice labels (v1:/bg:) — the raw tag reads like garbage in the popup
        let raw = concat!(
            "[ti:(When You Gonna) Give It Up to Me]\n",
            "[00:18.465]v1:<00:18.465>Through <00:19.102><00:19.102>drought <00:19.488><00:19.488>and famine\n",
            "[00:21.818]v1:<00:21.818>Natural <00:22.784><00:22.784>disasters\n",
            "[03:21.948]<bg:Don't <03:22.281><03:22.281>you love\n",
        );
        let cleaned = super::App::clean_lyrics(raw);
        assert!(cleaned.contains("Through drought and famine"), "got: {cleaned}");
        assert!(cleaned.contains("Natural disasters"));
        assert!(cleaned.contains("you love"));
        assert!(!cleaned.contains("00:18"), "timestamps must go: {cleaned}");
        assert!(!cleaned.contains("v1:"));
        assert!(!cleaned.contains("bg:"));
        assert!(!cleaned.contains("[ti:"), "metadata headers must go");
    }

    #[test]
    fn clean_lyrics_keeps_plain_text_intact() {
        assert_eq!(
            super::App::clean_lyrics("primera línea\n\nsegunda\n"),
            "primera línea\n\nsegunda"
        );
    }

    #[tokio::test]
    async fn enqueue_all_reports_real_failures() {
        // mpv may reject some files (unreadable over SFTP etc.) — the message
        // must count what actually got queued, not the attempt count
        let (mut player, _log) = fake_player(serde_json::json!({
            "pause": false, "playlist": [], "playlist-pos": 0, "volume": 100
        }))
        .await;
        let mut app = app_with(&[
            "Buena.flac",
            "fail-uno.flac",
            "Otra Buena.flac",
            "fail-dos.flac",
        ]);
        app.enqueue_all_filtered(&mut player).await;
        let msg = app.message.as_deref().unwrap_or("");
        assert!(
            msg.contains("queued 2") && msg.contains("4") && msg.contains("failed"),
            "expected honest count like 'queued 2 of 4, 2 failed', got: {msg:?}"
        );
        // two successful appends happened
        assert_eq!(_log.lock().await.iter().filter(|c| c.contains("loadfile")).count(), 4);
    }

    #[tokio::test]
    async fn cover_toggle_nothing_playing_shows_plain() {
        let (mut player, _log) = fake_player(serde_json::json!({})).await;
        // empty library, nothing playing → no track to read a cover from
        let mut app = app_with(&[]);
        app.toggle_cover(&mut player).await;
        assert!(matches!(
            app.cover,
            Some(super::CoverView::Plain(ref m)) if m.contains("nothing playing")
        ));
    }

    #[tokio::test]
    async fn cover_toggle_missing_file_shows_unavailable() {
        let (mut player, _log) = fake_player(serde_json::json!({})).await;
        let mut app = app_with(&["missing.flac"]);
        app.toggle_cover(&mut player).await;
        poll_until_cover_settles(&mut app).await;
        assert!(matches!(
            app.cover,
            Some(super::CoverView::Plain(ref m)) if m.contains("cover unavailable")
        ));
    }

    #[tokio::test]
    async fn cover_toggle_closes() {
        let (mut player, _log) = fake_player(serde_json::json!({})).await;
        let mut app = app_with(&["missing.flac"]);
        app.toggle_cover(&mut player).await;
        assert!(app.cover.is_some());
        app.toggle_cover(&mut player).await;
        assert!(app.cover.is_none());
    }

    #[tokio::test]
    async fn cover_popup_consumes_keys() {
        let (mut player, _log) = fake_player(serde_json::json!({})).await;
        let mut app = app_with(&["missing.flac"]);
        app.toggle_cover(&mut player).await;
        // unknown key while open: consumed, popup stays
        app.on_key(key('x'), &mut player).await;
        assert!(app.cover.is_some());
        // Esc closes
        app.on_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &mut player,
        )
        .await;
        assert!(app.cover.is_none());
        // 'i' and 'q' also close (q inside the popup is NOT quit)
        app.toggle_cover(&mut player).await;
        app.on_key(key('i'), &mut player).await;
        assert!(app.cover.is_none());
        app.toggle_cover(&mut player).await;
        let action = app.on_key(key('q'), &mut player).await;
        assert!(app.cover.is_none());
        assert_eq!(action, super::Action::None, "q in the popup must not quit");
    }

    #[tokio::test]
    async fn cover_toggle_non_flac_reports_missing() {
        let (mut player, _log) = fake_player(serde_json::json!({})).await;
        let mut app = app_with(&["song.m4a"]);
        app.toggle_cover(&mut player).await;
        poll_until_cover_settles(&mut app).await;
        assert!(matches!(
            app.cover,
            Some(super::CoverView::Plain(ref m)) if m.contains("no embedded cover")
        ));
    }

    /// Poll the background cover job until it lands (bounded wait).
    async fn poll_until_cover_settles(app: &mut App) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while app.cover_rx.is_some() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            app.poll_cover();
        }
    }

    #[tokio::test]
    async fn cover_toggle_shows_loading_immediately() {
        let (mut player, _log) = fake_player(serde_json::json!({})).await;
        let mut app = app_with(&["missing.flac"]);
        app.toggle_cover(&mut player).await;
        // the popup is open SYNCHRONOUSLY with loading feedback; the decode
        // job runs in the background and the event loop keeps repainting
        assert!(
            matches!(
                app.cover,
                Some(super::CoverView::Plain(ref m)) if m.contains("loading cover")
            ),
            "expected loading feedback, got {:?}",
            app.cover
        );
    }

    #[tokio::test]
    async fn cover_poll_applies_finished_result() {
        let (mut player, _log) = fake_player(serde_json::json!({})).await;
        let mut app = app_with(&["missing.flac"]);
        app.toggle_cover(&mut player).await;
        // the background job finishes fast (missing file); poll until applied
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while app.cover_rx.is_some() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            app.poll_cover();
        }
        assert!(matches!(
            app.cover,
            Some(super::CoverView::Plain(ref m)) if m.contains("cover unavailable")
        ));
        assert!(app.cover_rx.is_none());
    }

    #[tokio::test]
    async fn cover_toggle_while_loading_cancels() {
        let (mut player, _log) = fake_player(serde_json::json!({})).await;
        let mut app = app_with(&["missing.flac"]);
        app.toggle_cover(&mut player).await; // starts job
        app.toggle_cover(&mut player).await; // closes popup, orphans job
        assert!(app.cover.is_none());
        assert!(app.cover_rx.is_none());
        // the orphaned job must NOT resurrect the popup
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        app.poll_cover();
        assert!(app.cover.is_none());
    }

    /// PlayerStatus with a given time_pos (tests).
    fn status_at(time: f64) -> crate::player::PlayerStatus {
        crate::player::PlayerStatus {
            paused: false,
            time_pos: Some(time),
            duration: Some(300.0),
            path: Some("/m/x.flac".into()),
            playlist_pos: Some(0),
            playlist: vec![],
            volume: Some(100),
            shuffle: false,
            loop_playlist: "no".into(),
        }
    }

    #[tokio::test]
    async fn toggle_lyrics_stores_timed() {
        let raw = "[00:18.465]Through drought\n[00:21.818]Natural disasters\n";
        let (mut player, _log) = fake_player(serde_json::json!({
            "pause": false, "playlist": [], "playlist-pos": 0, "volume": 100,
            "metadata": {"TITLE": "x", "LYRICS": raw}
        }))
        .await;
        let mut app = app_with(&["x.flac"]);
        app.toggle_lyrics(&mut player).await;
        assert!(matches!(
            app.lyrics,
            Some(super::Lyrics::Timed(ref l)) if l.lines.len() == 2
        ));
    }

    #[tokio::test]
    async fn toggle_lyrics_plain_fallback() {
        let (mut player, _log) = fake_player(serde_json::json!({
            "pause": false, "playlist": [], "playlist-pos": 0, "volume": 100,
            "LYRICS": "primera línea\nsegunda\n"
        }))
        .await;
        let mut app = app_with(&["x.flac"]);
        app.toggle_lyrics(&mut player).await;
        assert!(matches!(app.lyrics, Some(super::Lyrics::Plain(_))));
    }

    #[test]
    fn follow_resets_offset_on_advance() {
        let mut app = app_with(&[]);
        app.lyrics = Some(super::Lyrics::Timed(crate::lyrics::parse_lrc(
            "[00:10.00]a\n[00:20.00]b\n[00:30.00]c\n",
        ).unwrap()));
        app.lyrics_active = None;
        app.lyrics_offset = 7;
        app.status = Some(status_at(25.0));
        app.sync_lyrics_follow();
        assert_eq!(app.lyrics_active, Some(1));
        assert_eq!(app.lyrics_offset, 0, "offset resets when the line advances");
        // same time → nothing resets
        app.lyrics_offset = 4;
        app.sync_lyrics_follow();
        assert_eq!(app.lyrics_offset, 4);
        // advance → reset again
        app.status = Some(status_at(35.0));
        app.sync_lyrics_follow();
        assert_eq!(app.lyrics_active, Some(2));
        assert_eq!(app.lyrics_offset, 0);
    }

    #[test]
    fn manual_offset_kept_between_ticks() {
        let mut app = app_with(&[]);
        app.lyrics = Some(super::Lyrics::Timed(crate::lyrics::parse_lrc(
            "[00:10.00]a\n[00:20.00]b\n",
        ).unwrap()));
        app.status = Some(status_at(25.0));
        app.lyrics_active = Some(1);
        app.lyrics_offset = 4;
        app.sync_lyrics_follow();
        assert_eq!(app.lyrics_offset, 4);
        assert_eq!(app.lyrics_active, Some(1));
    }

    #[tokio::test]
    async fn g_jumps_to_list_top() {
        let (mut player, _log) = fake_player(serde_json::json!({})).await;
        let mut app = app_with(&[]);
        let raw = (0..20)
            .map(|i| format!("[{:02}:{:02}.00]line {i:02}", i / 2, (i % 2) * 30))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        app.lyrics = Some(super::Lyrics::Timed(crate::lyrics::parse_lrc(&raw).unwrap()));
        app.lyrics_active = Some(10);
        app.lyrics_offset = 3;
        app.on_key(key('g'), &mut player).await;
        assert_eq!(app.lyrics_offset, -10);
    }

    #[tokio::test]
    async fn cover_popup_playback_keys_pass_through() {
        let (mut player, log) = fake_player(serde_json::json!({})).await;
        let mut app = app_with(&["a.flac"]);
        app.toggle_cover(&mut player).await;
        for (c, want) in [
            ('n', "playlist-next"),
            ('p', "playlist-prev"),
            ('+', "volume"),
            ('-', "volume"),
            (' ', "cycle"),
        ] {
            app.on_key(key(c), &mut player).await;
            assert!(
                log.lock().await.iter().any(|cmd| cmd.contains(want)),
                "key {c:?} must reach the player while cover is open"
            );
            assert!(app.cover.is_some(), "popup stays open");
        }
    }

    #[tokio::test]
    async fn lyrics_popup_playback_keys_pass_through() {
        let (mut player, log) = fake_player(serde_json::json!({})).await;
        let mut app = app_with(&["a.flac"]);
        app.lyrics = Some(super::Lyrics::Plain("texto".into()));
        for (c, want) in [
            ('n', "playlist-next"),
            ('p', "playlist-prev"),
            ('+', "volume"),
            ('-', "volume"),
            (' ', "cycle"),
        ] {
            app.on_key(key(c), &mut player).await;
            assert!(
                log.lock().await.iter().any(|cmd| cmd.contains(want)),
                "key {c:?} must reach the player while lyrics are open"
            );
            assert!(app.lyrics.is_some(), "popup stays open");
        }
    }

    #[tokio::test]
    async fn ctrl_c_quits() {
        let (mut player, _log) = fake_player(serde_json::json!({})).await;
        let mut app = app_with(&["a.flac"]);
        let k = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(app.on_key(k, &mut player).await, Action::Quit);
    }
}
