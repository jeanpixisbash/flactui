//! ratatui rendering: library pane, queue pane, playbar.
use crate::app::{App, View};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

const ACCENT: Color = Color::Cyan;
const ACCENT_2: Color = Color::Magenta;

fn file_display(path: &str) -> &str {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_str().unwrap_or(path))
        .unwrap_or(path)
}

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).split(f.area());

    // top area: library + queue (queue hidden below 100 cols)
    if chunks[0].width >= 100 {
        let top = Layout::horizontal([Constraint::Percentage(50), Constraint::Min(0)])
            .split(chunks[0]);
        draw_library(f, app, top[0]);
        draw_queue(f, app, top[1]);
    } else {
        draw_library(f, app, chunks[0]);
    }

    draw_playbar(f, app, chunks[1]);

    if app.cover.is_some() {
        draw_cover(f, app);
    }
    if app.lyrics.is_some() {
        draw_lyrics(f, app);
    }
}

/// Center-crop a grid coordinate into a popup cell area; None = skip cell.
fn grid_offset(grid: u16, area: u16) -> i32 {
    (i32::from(grid) - i32::from(area)) / 2
}

fn draw_cover(f: &mut Frame, app: &App) {
    let area = f.area(); // fullscreen: hide the library/queue behind
    f.render_widget(ratatui::widgets::Clear, area);
    let (title, art) = match &app.cover {
        Some(crate::app::CoverView::Ready { art, title }) => (title.as_str(), Some(art)),
        Some(crate::app::CoverView::Plain(msg)) => (msg.as_str(), None),
        None => return,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" cover · {title} "))
        .border_style(Style::default().fg(ACCENT_2));
    let inner = match block.inner(area) {
        // too small to draw anything meaningful
        i if i.width < 4 || i.height < 4 => return,
        i => i,
    };
    f.render_widget(block, area);
    let Some(art) = art else { return };
    let ox = grid_offset(art.w, inner.width);
    let oy = grid_offset(art.h, inner.height);
    for row in 0..inner.height {
        for col in 0..inner.width {
            let gx = col as i32 + ox;
            let gy = row as i32 + oy;
            if gx < 0 || gy < 0 || gx >= i32::from(art.w) || gy >= i32::from(art.h) {
                continue;
            }
            let cell = art.cells[gy as usize * art.w as usize + gx as usize];
            f.buffer_mut()[(inner.x + col, inner.y + row)]
                .set_symbol("▀")
                .set_fg(ratatui::style::Color::Rgb(cell.top[0], cell.top[1], cell.top[2]))
                .set_bg(ratatui::style::Color::Rgb(
                    cell.bottom[0], cell.bottom[1], cell.bottom[2],
                ));
        }
    }
}

fn draw_lyrics(f: &mut Frame, app: &App) {
    let area = f.area(); // fullscreen: hide the library/queue behind
    f.render_widget(ratatui::widgets::Clear, area); // wipe what's underneath

    match &app.lyrics {
        Some(crate::app::Lyrics::Timed(lines)) => {
            draw_timed_lyrics(f, app, area, lines);
        }
        other => {
            // Plain (or a stale None — nothing to draw then)
            let text = match other {
                Some(crate::app::Lyrics::Plain(t)) => t.as_str(),
                _ => return,
            };
            let n_lines = text.lines().count().max(1);
            let inner_h = area.height.saturating_sub(2) as usize;
            let max_off = n_lines.saturating_sub(inner_h);
            let off = app.lyrics_scroll.min(max_off) as u16;

            let para = Paragraph::new(text)
                .alignment(ratatui::layout::Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" lyrics ({n_lines} lines · Esc closes) "))
                        .border_style(Style::default().fg(ACCENT_2)),
                )
                .style(Style::default().fg(Color::White))
                .scroll((off, 0));
            f.render_widget(para, area);
        }
    }
}

/// Fade color for a line `d` rows away from the active one (pinned gradient:
/// `ch = round(255 - (d-1)*195/4)` → 255 at d=1, 206 at d=2, 60 at d=5, floor 60).
fn fade_fg(d: usize) -> Color {
    let ch = (255.0 - (d as f64 - 1.0) * 195.0 / 4.0).round();
    let ch = ch.max(60.0).min(255.0) as u8;
    Color::Rgb(ch, ch, ch)
}

fn draw_timed_lyrics(f: &mut Frame, app: &App, area: Rect, lines: &crate::lyrics::LrcLines) {
    let inner = {
        let b = Block::default()
            .borders(Borders::ALL)
            .title(format!(" lyrics · synced ({}) · j/k browse · g top · n/p skip · space pause · +/- vol · Esc closes ", lines.lines.len()))
            .border_style(Style::default().fg(ACCENT_2));
        let i = b.inner(area);
        f.render_widget(b, area);
        i
    };
    if inner.height == 0 {
        return;
    }
    let rows = inner.height as usize;
    let half = rows / 2;
    let active = app.lyrics_active.unwrap_or(0) as i32;
    let center = (active + app.lyrics_offset).max(0);
    // window: center - half .. center + half (clamped to the list)
    let start = (center - half as i32).max(0) as usize;
    let end = (center + half as i32).max(0) as usize;
    let end = end.min(lines.lines.len());
    let visible = &lines.lines[start..end];
    // pad so `center` lands on the middle row of the popup
    let center_row = (center - start as i32) as usize;
    let top_pad = rows.saturating_sub(center_row + 1).min(rows.saturating_sub(visible.len()));

    let mut rows_out: Vec<ratatui::text::Line> = Vec::with_capacity(rows);
    for _ in 0..top_pad {
        rows_out.push(ratatui::text::Line::from(""));
    }
    for (k, line) in visible.iter().enumerate() {
        let idx = start + k;
        let style = if Some(idx) == app.lyrics_active {
            Style::default().fg(ACCENT_2).add_modifier(Modifier::BOLD)
        } else {
            let d = idx.abs_diff(app.lyrics_active.unwrap_or(idx));
            Style::default().fg(fade_fg(d.max(1)))
        };
        rows_out.push(ratatui::text::Line::from(line.text.clone()).style(style));
    }
    let para = Paragraph::new(rows_out).alignment(ratatui::layout::Alignment::Center);
    f.render_widget(para, inner);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    let h = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(v[1]);
    h[1]
}

fn draw_library(f: &mut Frame, app: &App, area: Rect) {
    let title = match (&app.search, app.filtered.len()) {
        (Some(q), _) => format!(" library  /{q} ({}) ", app.filtered.len()),
        (None, n) => format!(" library ({n}) "),
    };
    let items: Vec<ListItem> = app
        .filtered
        .iter()
        .filter_map(|&i| app.tracks.get(i))
        .map(|t| ListItem::new(file_display(t)))
        .collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title).border_style(Style::default().fg(ACCENT)))
        .highlight_style(Style::default().fg(ACCENT_2).add_modifier(Modifier::BOLD))
        .highlight_symbol("▸ ");
    let mut state = ListState::default();
    if !app.filtered.is_empty() {
        state.select(Some(app.cursor.min(app.filtered.len() - 1)));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_queue(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .queue
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let marker = if app.status.as_ref().and_then(|s| s.playlist_pos) == Some(i) {
                "▶ "
            } else {
                "  "
            };
            ListItem::new(format!("{marker}{t}"))
        })
        .collect();
    // discoverability: the panel advertises its own keys when there's room
    let title = if area.width >= 40 {
        format!(" queue ({}) · d remove · D clear ", app.queue.len())
    } else {
        format!(" queue ({}) ", app.queue.len())
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(ACCENT_2)),
        )
        .highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");
    let mut state = ListState::default();
    if !app.queue.is_empty() {
        state.select(Some(app.queue_cursor.min(app.queue.len() - 1)));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_playbar(f: &mut Frame, app: &App, area: Rect) {
    let st = app.status.as_ref();
    let title = st
        .and_then(|s| s.path.as_deref())
        .map(file_display)
        .unwrap_or("(nothing playing)")
        .to_string();

    let (pos, dur) = (
        st.and_then(|s| s.time_pos).unwrap_or(0.0),
        st.and_then(|s| s.duration).unwrap_or(0.0),
    );
    let ratio = if dur > 0.0 { (pos / dur).clamp(0.0, 1.0) } else { 0.0 };
    let paused = st.map(|s| s.paused).unwrap_or(false);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);

    let state_marker = if paused { "⏸ " } else { "▶ " };
    let flags = format!(
        "{}{}",
        if app.shuffle { "  [S]huffle" } else { "" },
        if app.repeat { "  [R]epeat" } else { "" },
    );
    f.render_widget(
        Paragraph::new(Line::from(format!(
            "{state_marker}{title}   {}:{:02}/{}:{:02}{flags}",
            (pos as u64) / 60,
            (pos as u64) % 60,
            (dur as u64) / 60,
            (dur as u64) % 60
        )))
        .style(Style::default().fg(Color::White)),
        rows[0],
    );

    f.render_widget(
        Gauge::default()
            .ratio(ratio)
            .gauge_style(Style::default().fg(ACCENT).bg(Color::Black)),
        rows[1],
    );

    // hints; volume hidden on very narrow terminals
    let hints = if area.width >= 60 {
        format!(
            " j/k move · enter play · space pause · e queue · / search · tab · l lyrics · i cover · s shuffle · r repeat · ⇦⇨ seek  vol:{}%  q quit",
            app.volume
        )
    } else {
        " j/k · enter · space · e · / · tab · s · r · q".to_string()
    };
    f.render_widget(
        Paragraph::new(Line::from(hints)).style(Style::default().fg(Color::DarkGray)),
        rows[2],
    );
}

pub fn view_name(v: View) -> &'static str {
    match v {
        View::Library => "library",
        View::Queue => "queue",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::player::PlayerStatus;
    use std::path::PathBuf;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn app_playing() -> App {
        let mut app = App::new(
            PathBuf::from("/m"),
            vec![
                "Vida.flac".into(),
                "Perdon.flac".into(),
                "Es Epico.flac".into(),
            ],
        );
        app.status = Some(PlayerStatus {
            paused: false,
            time_pos: Some(42.0),
            duration: Some(300.0),
            path: Some("/m/Es Epico.flac".into()),
            playlist_pos: Some(0),
            playlist: vec!["Es Epico.flac".into()],
            volume: Some(100),
            shuffle: false,
            loop_playlist: "no".into(),
        });
        app
    }

    fn render_at(width: u16, height: u16, app: &App) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| draw(f, app))
            .unwrap();
        let buf = terminal.backend().buffer();
        buf.content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .replace('\n', "")
    }

    #[test]
    fn cover_popup_renders_blocks_and_title() {
        use crate::covers::{HalfBlocks, HalfCell};
        let mut app = app_playing();
        app.cover = Some(crate::app::CoverView::Ready {
            art: HalfBlocks {
                w: 4,
                h: 4,
                cells: vec![
                    HalfCell {
                        top: [255, 0, 0],
                        bottom: [0, 0, 255],
                    };
                    16
                ],
            },
            title: "Es Epico.flac".into(),
        });
        let text = render_at(100, 40, &app);
        assert!(text.contains('▀'), "half-blocks expected, got: {text}");
        assert!(text.contains("cover"), "title expected, got: {text}");
        assert!(text.contains("Es Epico"), "track title expected, got: {text}");
    }

    #[test]
    fn cover_popup_plain_message() {
        let mut app = app_playing();
        app.cover = Some(crate::app::CoverView::Plain("no embedded cover".into()));
        let text = render_at(100, 40, &app);
        assert!(text.contains("no embedded cover"), "got: {text}");
        assert!(!text.contains('▀'), "no art expected, got: {text}");
    }

    #[test]
    fn cover_popup_tiny_area_no_panic() {
        use crate::covers::{HalfBlocks, HalfCell};
        let mut app = app_playing();
        app.cover = Some(crate::app::CoverView::Ready {
            art: HalfBlocks {
                w: 4,
                h: 4,
                cells: vec![
                    HalfCell {
                        top: [1, 2, 3],
                        bottom: [4, 5, 6],
                    };
                    16
                ],
            },
            title: "x.flac".into(),
        });
        // must not panic on a tiny terminal
        let _ = render_at(20, 6, &app);
    }

    #[test]
    fn cover_popup_closed_renders_nothing() {
        let app = app_playing();
        let text = render_at(100, 40, &app);
        assert!(!text.contains('▀'));
    }

    #[test]
    fn playbar_shows_playing_track() {
        let app = app_playing();
        let text = render_at(120, 30, &app);
        assert!(text.contains("Es Epico.flac"), "got: {text}");
        assert!(text.contains("▶"));
    }

    #[test]
    fn paused_marker_shown() {
        let mut app = app_playing();
        app.status.as_mut().unwrap().paused = true;
        let text = render_at(120, 30, &app);
        assert!(text.contains("⏸"));
    }

    #[test]
    fn queue_hidden_on_narrow_terminal() {
        let app = app_playing();
        let text = render_at(80, 24, &app);
        assert!(!text.contains("queue ("), "queue should be hidden at 80 cols");
        let wide = render_at(120, 30, &app);
        assert!(wide.contains("queue ("));
    }

    #[test]
    fn volume_hidden_below_60_cols_and_no_panic() {
        let app = app_playing();
        let text = render_at(50, 12, &app);
        assert!(!text.contains("vol:"));
    }

    #[test]
    fn lyrics_popup_renders_over_library_and_scrolls() {
        let mut app = app_playing();
        // distinct first/last markers: "line 01" would substring-match "line 10"
        // 22 lines vs ~19 visible → max scroll 3, so scroll=2 shows the tail
        let mut lines = vec!["primera".to_string()];
        lines.extend((2..=38).map(|i| format!("relleno {i}")));
        lines.push("última".to_string());
        app.lyrics = Some(crate::app::Lyrics::Plain(lines.join("\n")));
        let text = render_at(120, 30, &app);
        assert!(text.contains("primera"), "popup should render: {text}");
        assert!(text.contains("lyrics"), "title bar");

        // fullscreen: inner ~28 rows vs 39 lines → max scroll = 11;
        // al fondo se ve el final y la primera línea desaparece
        app.lyrics_scroll = 11;
        let scrolled = render_at(120, 30, &app);
        assert!(!scrolled.contains("primera"), "scrolled past first line");
        assert!(scrolled.contains("última"));
    }

    // ---- timed (karaoke-follow) lyrics ----

    /// App with a 20-line Timed popup (times i*10 s, texts `line {i:02}`).
    fn app_timed() -> App {
        let mut app = app_playing();
        let lines = (0..20)
            .map(|i| crate::lyrics::LrcLine {
                time: i as f64 * 10.0,
                text: format!("line {i:02}"),
            })
            .collect();
        app.lyrics = Some(crate::app::Lyrics::Timed(crate::lyrics::LrcLines { lines }));
        app
    }

    #[test]
    fn timed_centers_active_line() {
        let mut app = app_timed();
        app.status.as_mut().unwrap().time_pos = Some(100.0); // active = 10
        app.sync_lyrics_follow();
        let text = render_at(120, 30, &app);
        assert!(text.contains("line 10"), "active line centered: {text}");
    }

    #[test]
    fn timed_fade_gradient() {
        let mut app = app_timed();
        app.status.as_mut().unwrap().time_pos = Some(100.0);
        app.sync_lyrics_follow();
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        // locate the active line by text (geometry-proof, no hardcoded rows)
        let row_of = |needle: &str| -> Option<(u16, u16)> {
            for y in 0..30u16 {
                let mut acc = String::new();
                for x in 0..120u16 {
                    acc.push_str(buf[(x, y)].symbol());
                    if acc.contains(needle) {
                        return Some((x.saturating_sub(needle.len() as u16 - 1), y));
                    }
                }
            }
            None
        };
        let (x, active_row) = row_of("line 10").expect("active line rendered");
        assert_eq!(buf[(x, active_row)].fg, ratatui::style::Color::Magenta, "active accent");
        let d2 = buf[(x, active_row + 2)].fg;
        assert_eq!(d2, ratatui::style::Color::Rgb(206, 206, 206), "fade at d=2");
        let d5 = buf[(x, active_row + 5)].fg;
        assert_eq!(d5, ratatui::style::Color::Rgb(60, 60, 60), "fade floor");
    }

    #[test]
    fn timed_manual_offset_shifts() {
        let mut app = app_timed();
        app.status.as_mut().unwrap().time_pos = Some(100.0); // active 10
        app.sync_lyrics_follow();
        app.lyrics_offset = 3;
        let text = render_at(120, 30, &app);
        assert!(text.contains("line 13"), "offset shifts the window: {text}");
    }

    #[test]
    fn timed_g_shows_top() {
        // 40 lines vs ~28 fullscreen rows: g must anchor the list top, and the
        // tail goes out of view
        let mut app = app_playing();
        let lines = (0..40)
            .map(|i| crate::lyrics::LrcLine {
                time: i as f64 * 10.0,
                text: format!("line {i:02}"),
            })
            .collect();
        app.lyrics = Some(crate::app::Lyrics::Timed(crate::lyrics::LrcLines { lines }));
        app.status.as_mut().unwrap().time_pos = Some(100.0); // active 10
        app.sync_lyrics_follow();
        app.lyrics_offset = -10; // center = 0 → list top at window top
        let text = render_at(120, 30, &app);
        assert!(text.contains("line 00"), "g shows the list top: {text}");
        assert!(!text.contains("line 39"), "tail out of view when top-anchored");
    }

    #[test]
    fn timed_without_time_pos_renders_from_top() {
        let mut app = app_timed();
        app.status.as_mut().unwrap().time_pos = None;
        app.sync_lyrics_follow();
        let text = render_at(120, 30, &app);
        assert!(text.contains("line 00"), "list from the top: {text}");
    }

    #[test]
    fn timed_lines_are_horizontally_centered() {
        let mut app = app_timed();
        app.status.as_mut().unwrap().time_pos = Some(100.0);
        app.sync_lyrics_follow();
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        // one char per cell → string index == column (take only the first char
        // of each symbol: multi-char symbols would inflate the index)
        let col_of = |needle: &str| -> Option<(u16, u16)> {
            for y in 0..30u16 {
                let row: String = (0..120u16)
                    .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                    .collect();
                // find() is BYTE index; multi-byte chars before the match
                // (border glyphs, accents behind the popup) shift it — map
                // back to char index == cell column
                if let Some(b) = row.find(needle) {
                    return Some((row[..b].chars().count() as u16, y));
                }
            }
            None
        };
        let (x_found, y_found) = col_of("line 10").expect("active line rendered");
        if std::env::var("DBG_CENTER").is_ok() {
            eprintln!("DBG x={x_found} y={y_found}");
            for x in 55..70u16 {
                let s = buf[(x, 16)].symbol();
                eprintln!("DBG ({x},16)={s:?} len={}", s.chars().count());
            }
        }
        // expected: centered within the popup's inner area (fullscreen,
        // ratatui rounds up)
        let area = ratatui::layout::Rect::new(0, 0, 120, 30);
        let inner_w = area.width - 2;
        let expected = area.x + 1 + (inner_w - 7 + 1) / 2; // 7 = len("line 10")
        assert_eq!(x_found, expected, "lyric line must be horizontally centered");
    }

    #[test]
    fn lyrics_popup_covers_full_screen() {
        let mut app = app_playing();
        app.lyrics = Some(crate::app::Lyrics::Plain("hola mundo".into()));
        let text = render_at(120, 30, &app);
        assert!(
            !text.contains("library ("),
            "lyrics fullscreen must hide the library behind: {text}"
        );
        let mut app = app_playing();
        app.cover = Some(crate::app::CoverView::Plain("no embedded cover".into()));
        let text = render_at(120, 30, &app);
        assert!(
            !text.contains("library ("),
            "cover fullscreen must hide the library behind: {text}"
        );
    }

    #[test]
    fn timed_tiny_area_no_panic() {
        let mut app = app_timed();
        app.status.as_mut().unwrap().time_pos = Some(100.0);
        app.sync_lyrics_follow();
        let _ = render_at(20, 6, &app);
    }

    #[test]
    fn queue_title_shows_remove_and_clear_keys() {
        // discoverability: the queue panel must advertise d/D
        let mut app = app_playing();
        app.queue = vec!["a.flac".into(), "b.flac".into(), "c.flac".into()];
        let text = render_at(120, 30, &app);
        assert!(text.contains("d remove"), "queue title lacks remove hint: {text}");
        assert!(text.contains("D clear"), "queue title lacks clear hint: {text}");
        assert!(text.contains("queue (3"), "queue count missing: {text}");
    }

    #[test]
    fn library_lists_tracks_and_search_line() {
        let mut app = app_playing();
        app.search = Some("vida".into());
        app.apply_search("vida");
        let text = render_at(120, 30, &app);
        assert!(text.contains("/vida"));
        assert!(text.contains("Vida.flac"));
    }
}
