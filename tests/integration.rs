//! Headless integration tests: real Player over a fake mpv socket, real app
//! loop over scripted events, TestBackend rendering.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use flactui::app::App;
use flactui::player::Player;
use flactui::run::{farewell_prompt, run_app, EventPump};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

static N: AtomicUsize = AtomicUsize::new(0);

fn key(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

/// Fake mpv IPC server. `close_after` = close the stream after N commands
/// (simulates a dead player / EOF).
async fn fake_server(
    props: serde_json::Value,
    close_after: Option<usize>,
) -> (PathBuf, tokio::task::JoinHandle<()>) {
    let n = N.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("flactui-int-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("fake.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read).lines();
        let mut count = 0usize;
        loop {
            let line = match lines.next_line().await {
                Ok(Some(l)) => l,
                _ => break,
            };
            let req: serde_json::Value = serde_json::from_str(&line).unwrap();
            let id = req["request_id"].clone();
            let cmd = req["command"].as_array().unwrap();
            let name = cmd[0].as_str().unwrap();
            let data = if name == "get_property" {
                props.get(cmd[1].as_str().unwrap()).cloned().unwrap_or(json!(null))
            } else {
                json!(null)
            };
            let resp = json!({ "error": "success", "request_id": id, "data": data });
            if write.write_all(format!("{resp}\n").as_bytes()).await.is_err() {
                break;
            }
            count += 1;
            if let Some(max) = close_after {
                if count >= max {
                    break; // drop stream → EOF for the client
                }
            }
        }
    });
    (sock, handle)
}

#[tokio::test]
async fn enqueue_then_status_shows_queue_then_quit() {
    let (sock, _srv) = fake_server(
        json!({
            "pause": false,
            "playlist": [{"filename": "/m/Perdon.flac"}],
            "playlist-pos": 0,
            "path": "/m/Perdon.flac",
            "time-pos": 1.0,
            "duration": 200.0
        }),
        None,
    )
    .await;
    let mut player = Player::connect(&sock).await.unwrap();
    let mut app = App::new(
        PathBuf::from("/m"),
        vec!["Perdon.flac".into(), "Vida.flac".into()],
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    run_app(
        &mut terminal,
        EventPump::from_events(vec![key(' ')]),
        &mut player,
        &mut app,
        Duration::from_millis(1),
    )
    .await
    .unwrap();
    // the spacebar enqueued; the next poll picked the queue up
    assert_eq!(app.queue, vec!["Perdon.flac"]);
    assert!(app.message.is_none());
}

#[tokio::test]
async fn player_lost_surfaces_message_and_q_exits_cleanly() {
    let (sock, _srv) = fake_server(
        json!({
            "pause": false,
            "playlist": [{"filename": "/m/Perdon.flac"}],
            "playlist-pos": 0,
            "path": "/m/Perdon.flac",
            "time-pos": 1.0,
            "duration": 200.0
        }),
        Some(1), // dies after the first command (the enqueue)
    )
    .await;
    let mut player = Player::connect(&sock).await.unwrap();
    let mut app = App::new(
        PathBuf::from("/m"),
        vec!["Perdon.flac".into(), "Vida.flac".into()],
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    // pending pump: events fire, channel stays open so ticks keep coming
    run_app(
        &mut terminal,
        EventPump::from_events_pending(vec![key(' '), key('q')]),
        &mut player,
        &mut app,
        Duration::from_millis(1),
    )
    .await
    .unwrap();
    assert_eq!(
        app.message.as_deref(),
        Some("player lost — press q to exit")
    );
}

#[tokio::test]
async fn enqueue_missing_file_shows_message() {
    let (sock, _srv) = fake_server(
        json!({ "pause": false, "playlist": json!([]), "playlist-pos": json!(null), "path": json!(null), "time-pos": json!(null), "duration": json!(null) }),
        None,
    )
    .await;
    // server variant answering mpv-level errors:
    // (use a dedicated one instead)
    drop(sock);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("flactui-interr-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sock2 = dir.join("fake.sock");
    let listener = UnixListener::bind(&sock2).unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let req: serde_json::Value = serde_json::from_str(&line).unwrap();
            let resp = json!({
                "error": "error loading file '/m/gone.flac'",
                "request_id": req["request_id"].clone(),
                "data": null
            });
            write.write_all(format!("{resp}\n").as_bytes()).await.unwrap();
        }
    });
    let mut player = Player::connect(&sock2).await.unwrap();
    let mut app = App::new(PathBuf::from("/m"), vec!["gone.flac".into()]);
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    run_app(
        &mut terminal,
        EventPump::from_events(vec![key(' ')]),
        &mut player,
        &mut app,
        Duration::from_millis(1),
    )
    .await
    .unwrap();
    assert!(app.message.as_deref().unwrap().contains("error loading file"));
}

#[test]
fn farewell_prompt_offered_when_mounted() {
    let prompt = farewell_prompt(true).unwrap();
    assert!(prompt.contains("still mounted"));
    assert!(prompt.contains("[Y/n]"));
}
