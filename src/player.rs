//! Persistent mpv player controlled over its JSON IPC socket.

use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

#[derive(Debug, Clone, Default)]
pub struct PlayerStatus {
    pub paused: bool,
    pub time_pos: Option<f64>,
    pub duration: Option<f64>,
    pub path: Option<String>,
    pub playlist_pos: Option<usize>,
    pub playlist: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoadMode {
    Replace,
    Append,
}

impl LoadMode {
    fn mpv_arg(self) -> &'static str {
        match self {
            LoadMode::Replace => "replace",
            LoadMode::Append => "append-play",
        }
    }
}

pub struct Player {
    child: Option<tokio::process::Child>,
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    next_id: u64,
    sock_path: PathBuf,
}

impl Player {
    /// Spawn mpv as a persistent idle process and connect to its IPC socket.
    pub async fn spawn() -> anyhow::Result<Player> {
        let dir = std::env::temp_dir();
        let sock = dir.join(format!("flactui-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock);
        let child = tokio::process::Command::new("mpv")
            .args([
                "--no-video",
                "--no-terminal",
                "--idle=yes",
                "--keep-open=no",
                format!("--input-ipc-server={}", sock.display()).as_str(),
            ])
            .spawn()?;
        // wait for the socket to appear
        for _ in 0..50 {
            if sock.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        anyhow::ensure!(sock.exists(), "mpv did not create its IPC socket");
        Player::connect(&sock).await.map(|mut p| {
            p.child = Some(child);
            p
        })
    }

    /// Connect to an existing IPC socket (mpv's or a fake one in tests).
    pub async fn connect(sock: &Path) -> anyhow::Result<Player> {
        let stream = UnixStream::connect(sock).await?;
        let (read, write) = stream.into_split();
        Ok(Player {
            child: None,
            reader: BufReader::new(read),
            writer: write,
            next_id: 1,
            sock_path: sock.to_path_buf(),
        })
    }

    pub fn sock_path(&self) -> &Path {
        &self.sock_path
    }

    /// Whether the spawned mpv process is still running (None child = attached).
    pub fn is_alive(&mut self) -> bool {
        match &mut self.child {
            Some(c) => c.try_wait().map(|s| s.is_none()).unwrap_or(true),
            None => true,
        }
    }

    /// Send one command and wait for the response with a matching request_id.
    /// mpv-level errors (e.g. missing file) come back as Ok(resp) — the caller
    /// decides what to do with `resp["error"]`.
    async fn command(&mut self, cmd: Value) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        // mpv request format: {"command": [...], "request_id": N}
        let command = cmd.get("command").cloned().unwrap_or(cmd);
        let req = serde_json::json!({ "command": command, "request_id": id });
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        let send = async {
            self.writer.write_all(line.as_bytes()).await?;
            self.writer.flush().await
        };
        if send.await.is_err() {
            anyhow::bail!("mpv IPC socket closed");
        }

        let mut buf = String::new();
        loop {
            buf.clear();
            let n = match self.reader.read_line(&mut buf).await {
                Ok(n) => n,
                Err(_) => anyhow::bail!("mpv IPC socket closed"),
            };
            if n == 0 {
                anyhow::bail!("mpv IPC socket closed");
            }
            let v: Value = match serde_json::from_str(buf.trim()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("request_id").and_then(Value::as_u64) == Some(id) {
                return Ok(v);
            }
            // events without our request_id are skipped
        }
    }

    pub async fn loadfile(&mut self, path: &str, mode: LoadMode) -> anyhow::Result<Value> {
        self.command(serde_json::json!({ "command": ["loadfile", path, mode.mpv_arg()] }))
            .await
    }

    pub async fn status(&mut self) -> anyhow::Result<PlayerStatus> {
        let get = |name: &str| serde_json::json!({ "command": ["get_property", name] });
        let paused = self.command(get("pause")).await?;
        let time_pos = self.command(get("time-pos")).await?;
        let duration = self.command(get("duration")).await?;
        let path = self.command(get("path")).await?;
        let playlist_pos = self.command(get("playlist-pos")).await?;
        let playlist_raw = self.command(get("playlist")).await?;

        let prop = |v: &Value| v.get("data").cloned().unwrap_or(Value::Null);
        let opt_f = |v: &Value| prop(v).as_f64();
        let playlist = prop(&playlist_raw)
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| {
                        e.get("filename")
                            .and_then(Value::as_str)
                            .map(|f| {
                                std::path::Path::new(f)
                                    .file_name()
                                    .map(|s| s.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| f.to_string())
                            })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(PlayerStatus {
            paused: prop(&paused).as_bool().unwrap_or(false),
            time_pos: opt_f(&time_pos),
            duration: opt_f(&duration),
            path: prop(&path).as_str().map(String::from),
            playlist_pos: prop(&playlist_pos).as_u64().map(|v| v as usize),
            playlist,
        })
    }

    pub async fn toggle_pause(&mut self) -> anyhow::Result<Value> {
        self.command(serde_json::json!({ "command": ["cycle", "pause"] }))
            .await
    }

    pub async fn set_volume(&mut self, v: i64) -> anyhow::Result<Value> {
        self.command(serde_json::json!({ "command": ["set_property", "volume", v] }))
            .await
    }

    pub async fn seek(&mut self, secs: f64) -> anyhow::Result<Value> {
        self.command(serde_json::json!({ "command": ["seek", secs] }))
            .await
    }

    pub async fn next(&mut self) -> anyhow::Result<Value> {
        self.command(serde_json::json!({ "command": ["playlist-next", "weak"] }))
            .await
    }

    pub async fn previous(&mut self) -> anyhow::Result<Value> {
        self.command(serde_json::json!({ "command": ["playlist-prev", "weak"] }))
            .await
    }

    pub async fn playlist_remove(&mut self, index: usize) -> anyhow::Result<Value> {
        self.command(serde_json::json!({ "command": ["playlist-remove", index] }))
            .await
    }

    pub async fn playlist_move(&mut self, from: usize, to: usize) -> anyhow::Result<Value> {
        self.command(serde_json::json!({ "command": ["playlist-move", from, to] }))
            .await
    }

    pub async fn playlist_clear(&mut self) -> anyhow::Result<Value> {
        self.command(serde_json::json!({ "command": ["playlist-clear"] }))
            .await
    }

    pub async fn playlist_play_index(&mut self, index: usize) -> anyhow::Result<Value> {
        self.command(serde_json::json!({ "command": ["playlist-play-index", index] }))
            .await
    }

    pub async fn quit_player(&mut self) -> anyhow::Result<()> {
        let _ = self
            .command(serde_json::json!({ "command": ["quit"] }))
            .await;
        if let Some(c) = &mut self.child {
            let _ = c.wait().await;
        }
        let _ = std::fs::remove_file(&self.sock_path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::UnixListener;

    fn unique_dir(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "flactui-fake-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Fake mpv IPC server: replies `{"error":..,"request_id":N}` to each line.
    async fn fake_server(reply_data: Value) -> (PathBuf, tokio::task::JoinHandle<()>) {
        let sock = unique_dir("ok").join("fake.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let sock_path = sock.clone();
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let req: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                let id = req.get("request_id").cloned().unwrap_or(Value::Null);
                let resp = serde_json::json!({ "error": "success", "request_id": id, "data": reply_data });
                write
                    .write_all(format!("{resp}\n").as_bytes())
                    .await
                    .unwrap();
            }
        });
        (sock_path, handle)
    }

    #[tokio::test]
    async fn loadfile_append_sends_append_play() {
        let (sock, server) = fake_server(Value::Null).await;
        let mut p = Player::connect(&sock).await.unwrap();
        let resp = p.loadfile("/x.flac", LoadMode::Append).await.unwrap();
        assert_eq!(resp["error"], "success");
        server.abort();
    }

    #[tokio::test]
    async fn toggle_pause_sends_cycle() {
        let (sock, server) = fake_server(Value::Null).await;
        let mut p = Player::connect(&sock).await.unwrap();
        p.toggle_pause().await.unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn mpv_error_comes_back_as_ok_not_err() {
        // server that replies with an mpv-level error
        let sock = unique_dir("err").join("fake.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let req: Value = serde_json::from_str(&line).unwrap();
                let id = req["request_id"].clone();
                let resp = serde_json::json!({
                    "error": "error loading file", "request_id": id, "data": Value::Null
                });
                write.write_all(format!("{resp}\n").as_bytes()).await.unwrap();
            }
        });
        let mut p = Player::connect(&sock).await.unwrap();
        let resp = p.loadfile("/gone.flac", LoadMode::Replace).await.unwrap();
        assert_eq!(resp["error"], "error loading file");
        server.abort();
    }

    #[tokio::test]
    async fn status_parses_properties_and_playlist() {
        // server that replies per-command based on the requested property
        let sock = unique_dir("st").join("fake.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let req: Value = serde_json::from_str(&line).unwrap();
                let id = req["request_id"].clone();
                let cmd = req["command"].as_array().unwrap();
                let prop = cmd[1].as_str().unwrap();
                let data = match prop {
                    "pause" => Value::Bool(true),
                    "time-pos" => serde_json::json!(42.5),
                    "duration" => serde_json::json!(300.0),
                    "path" => serde_json::json!("/m/Es Epico.flac"),
                    "playlist-pos" => serde_json::json!(2),
                    "playlist" => serde_json::json!([
                        {"filename": "/m/Vida.flac"},
                        {"filename": "/m/Perdon.flac"},
                        {"filename": "/m/Es Epico.flac"}
                    ]),
                    _ => Value::Null,
                };
                let resp = serde_json::json!({ "error": "success", "request_id": id, "data": data });
                write.write_all(format!("{resp}\n").as_bytes()).await.unwrap();
            }
        });
        let mut p = Player::connect(&sock).await.unwrap();
        let st = p.status().await.unwrap();
        assert!(st.paused);
        assert_eq!(st.time_pos, Some(42.5));
        assert_eq!(st.duration, Some(300.0));
        assert_eq!(st.path.as_deref(), Some("/m/Es Epico.flac"));
        assert_eq!(st.playlist_pos, Some(2));
        assert_eq!(st.playlist, vec!["Vida.flac", "Perdon.flac", "Es Epico.flac"]);
        server.abort();
    }
}
