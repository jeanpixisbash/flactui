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

    if app.lyrics.is_some() {
        draw_lyrics(f, app);
    }
}

fn draw_lyrics(f: &mut Frame, app: &App) {
    let area = centered_rect(70, 70, f.area());
    f.render_widget(ratatui::widgets::Clear, area); // wipe what's underneath

    let text = app.lyrics.as_deref().unwrap_or("");
    let n_lines = text.lines().count().max(1);
    let inner_h = area.height.saturating_sub(2) as usize;
    let max_off = n_lines.saturating_sub(inner_h);
    let off = app.lyrics_scroll.min(max_off) as u16;

    let para = Paragraph::new(text)
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
            " j/k move · enter play · space pause · e queue · / search · tab · l lyrics · s shuffle · r repeat · ⇦⇨ seek  vol:{}%  q quit",
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
        lines.extend((2..=21).map(|i| format!("relleno {i}")));
        lines.push("última".to_string());
        app.lyrics = Some(lines.join("\n"));
        let text = render_at(120, 30, &app);
        assert!(text.contains("primera"), "popup should render: {text}");
        assert!(text.contains("lyrics"), "title bar");

        // con scroll al máximo (22 líneas, ~19 visibles → máx 3),
        // la primera línea sale del área y se ve el final
        app.lyrics_scroll = 3;
        let scrolled = render_at(120, 30, &app);
        assert!(!scrolled.contains("primera"), "scrolled past first line");
        assert!(scrolled.contains("última"));
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
