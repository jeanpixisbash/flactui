//! flactui — spotify-tui-style TUI for your phone's music library (KDE Connect + mpv).
use anyhow::Context;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use flactui::app::App;
use flactui::cache;
use flactui::kde;
use flactui::player::Player;
use flactui::run::{farewell_prompt, run_app, EventPump};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{stdout, IsTerminal, Write};
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("flactui {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!("flactui {} — spotify-tui-style TUI for your phone's music", env!("CARGO_PKG_VERSION"));
                println!("\nUsage: flactui [--keep-mounted]");
                println!("  --keep-mounted  skip the unmount prompt on exit");
                return Ok(());
            }
            "--keep-mounted" => {}
            other => anyhow::bail!("unknown argument: {other}\ntry --help"),
        }
    }

    println!("Connecting to your phone (KDE Connect)...");
    // the phone often flaps (Android doze) — a couple of quick retries with a
    // refresh in between make starting up feel reliable instead of a coin flip
    let phone = {
        let mut phone = None;
        for attempt in 1..=3 {
            match kde::connect().await {
                Ok(p) => {
                    phone = Some(p);
                    break;
                }
                Err(e) if attempt < 3 => {
                    eprintln!("  attempt {attempt}/3 failed ({e}); retrying...");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                Err(e) => anyhow::bail!(
                    "flactui: {e}\n  - open the KDE Connect app on your phone (Android kills it in the background)\n  - check phone and computer are on the same network"
                ),
            }
        }
        phone.expect("loop ran at least once")
    };
    let music = kde::music_dir(&phone);

    // cached library listing (rebuild when missing or empty); a FAILED build
    // is never cached — a partial index would otherwise persist forever
    let cache_file = cache::cache_path();
    let tracks = match cache::load(&cache_file) {
        Ok(t) if !t.is_empty() => t,
        _ => {
            println!("Listing your phone library (a few seconds)...");
            let t = cache::build(&music).map_err(|e| {
                anyhow::anyhow!("could not read the phone library ({e}) — try again")
            })?;
            cache::save(&cache_file, &t)?;
            t
        }
    };
    anyhow::ensure!(
        !tracks.is_empty(),
        "no audio files found in {}\n  set FLACAST_PHONE_MUSIC_DIR if your phone keeps music elsewhere",
        music.display()
    );

    let mut player = Player::spawn().await.context("failed to start mpv")?;
    let mut app = App::new(music, tracks);
    app.phone_mount = Some(phone.mount_point);

    // terminal setup (restore guaranteed below)
    enable_raw_mode()?;
    let mut out = stdout();
    out.execute(EnterAlternateScreen)?;
    let result = {
        let backend = CrosstermBackend::new(&mut out);
        let mut terminal = Terminal::new(backend)?;
        run_app(
            &mut terminal,
            EventPump::spawn_real(),
            &mut player,
            &mut app,
            Duration::from_millis(500),
        )
        .await
    };

    // restore terminal no matter what
    disable_raw_mode()?;
    out.execute(LeaveAlternateScreen)?;

    let _ = player.quit_player().await;
    result?;

    // farewell: offer to unmount, like flacast
    if !std::env::args().any(|a| a == "--keep-mounted") && std::io::stdin().is_terminal() {
        if let Some(prompt) = farewell_prompt(true) {
            print!("{prompt}");
            let _ = stdout().flush();
            let mut answer = String::new();
            let _ = std::io::stdin().read_line(&mut answer);
            if !answer.trim().starts_with('n') {
                match kde::unmount(&phone.device_id).await {
                    Ok(()) => println!("Phone unmounted. Bye!"),
                    Err(_) => println!(
                        "Could not unmount from here — leave it with: flacast --unmount"
                    ),
                }
            }
        }
    }
    Ok(())
}
