//! Event pump + main application loop (headless-testable).
use crate::app::{Action, App};
use crate::player::Player;
use crate::ui;
use crossterm::event::{Event, KeyEventKind};
use ratatui::backend::Backend;
use ratatui::Terminal;
use std::time::Duration;
use tokio::sync::mpsc;

/// Delivers terminal events (or scripted ones in tests).
pub struct EventPump {
    rx: mpsc::UnboundedReceiver<Event>,
    _keepalive: Option<Box<mpsc::UnboundedSender<Event>>>,
}

impl EventPump {
    /// Real input: a thread reading crossterm events (blocking reads are fine
    /// on their own thread; we bridge to async with an unbounded channel).
    pub fn spawn_real() -> EventPump {
        let (tx, rx) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            while let Some(ev) =
                read_event_with_retry(|| crossterm::event::read().map_err(Into::into))
            {
                if tx.send(ev).is_err() {
                    break;
                }
            }
        });
        EventPump { rx, _keepalive: None }
    }

    /// Scripted events for tests; the channel closes after the last one.
    pub fn from_events(events: Vec<Event>) -> EventPump {
        let (tx, rx) = mpsc::unbounded_channel();
        for ev in events {
            let _ = tx.send(ev);
        }
        EventPump { rx, _keepalive: None }
    }

    /// Scripted events that do NOT close the channel afterwards — lets the
    /// tick branch run between events (needed for the player-lost test).
    pub fn from_events_pending(events: Vec<Event>) -> EventPump {
        let (tx, rx) = mpsc::unbounded_channel();
        for ev in events {
            let _ = tx.send(ev);
        }
        EventPump { rx, _keepalive: Some(Box::new(tx)) }
    }

    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}

/// Bounded retry around ONE crossterm read: transient errors (EINTR and
/// friends) are retried, but a persistent failure (terminal gone) gives up
/// after `MAX_READ_ERRORS` consecutive attempts (returns None) instead of
/// hanging or exiting the whole TUI without a trace.
const MAX_READ_ERRORS: u32 = 5;

fn read_event_with_retry<F>(mut read: F) -> Option<Event>
where
    F: FnMut() -> std::io::Result<Event>,
{
    let mut errors = 0u32;
    loop {
        match read() {
            Ok(ev) => return Some(ev),
            Err(_) => {
                errors += 1;
                if errors >= MAX_READ_ERRORS {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// Poll-model main loop: refresh status every iteration, render, then wait
/// for an event or the tick.
pub async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut pump: EventPump,
    player: &mut Player,
    app: &mut App,
    tick: Duration,
) -> anyhow::Result<()> {
    loop {
        // bound the poll: a stuck mpv must not freeze the loop (keys keep working)
        if tokio::time::timeout(tick + Duration::from_secs(1), app.refresh_status(player))
            .await
            .is_err()
        {
            app.message = Some("player not responding — press q to exit".into());
        }
        if !player.is_alive() {
            app.message = Some("player lost — press q to exit".into());
        }
        // apply a finished background cover decode, if any
        app.poll_cover();
        terminal
            .draw(|f| ui::draw(f, app))
            .map_err(|e| anyhow::anyhow!("draw error: {e}"))?;

        tokio::select! {
            ev = pump.next() => match ev {
                Some(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                    if app.on_key(k, player).await == Action::Quit {
                        break;
                    }
                }
                Some(_) => {}
                None => break,
            },
            _ = tokio::time::sleep(tick) => {}
        }
    }
    Ok(())
}

/// The farewell prompt text (None = nothing to ask; e.g. unmounted already).
pub fn farewell_prompt(mounted: bool) -> Option<String> {
    mounted.then(|| "Your phone is still mounted. Unmount it now? [Y/n] ".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn farewell_prompt_only_when_mounted() {
        assert!(farewell_prompt(true)
            .unwrap()
            .contains("still mounted"));
        assert!(farewell_prompt(false).is_none());
    }

    #[test]
    fn read_events_retries_transient_errors() {
        // crossterm read errors (EINTR etc.) happen; the pump thread must
        // retry instead of silently killing the TUI
        let mut attempts = 0;
        let ev = read_event_with_retry(|| {
            attempts += 1;
            if attempts < 3 {
                Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "eintr"))
            } else {
                Ok(Event::Key(crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('q'),
                    crossterm::event::KeyModifiers::NONE,
                )))
            }
        });
        assert!(ev.is_some(), "transient errors retried until the event");
    }

    #[test]
    fn read_events_gives_up_after_streak_of_errors() {
        // a persistent failure (terminal gone) ends the thread after a bounded
        // retry streak — it must not spin forever either
        let ev = read_event_with_retry(|| Err(std::io::Error::other("terminal gone")));
        assert!(ev.is_none());
    }
}
