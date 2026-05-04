//! Layout and drawing helpers for the v2 fleet-dashboard TUI.

use chrono::Utc;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use super::app::App;
use super::widgets::help_overlay::HelpOverlay;
use super::widgets::source_table::{DetailPane, FleetTable};

/// Draw the complete TUI for one frame.
pub fn draw(f: &mut Frame, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header (title + poll status)
            Constraint::Min(6),    // fleet table
            Constraint::Length(5), // detail pane (selected row)
            Constraint::Length(1), // footer
        ])
        .split(f.area());

    f.render_widget(Clear, f.area());
    draw_header(f, areas[0], app);
    f.render_widget(FleetTable { app }, areas[1]);
    draw_detail(f, areas[2], app);
    draw_footer(f, areas[3], app);

    if app.help_visible {
        f.render_widget(HelpOverlay, f.area());
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let title_line = Line::from(vec![
        Span::styled(" Quelch Status", Style::default().fg(Color::White)),
        Span::styled(
            format!("  v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let poll_line = if let Some(err) = &app.last_poll_error {
        Line::from(vec![
            Span::styled(" Poll error: ", Style::default().fg(Color::Red)),
            Span::styled(
                err.chars().take(80).collect::<String>(),
                Style::default().fg(Color::Red),
            ),
        ])
    } else if let Some(at) = app.last_poll_at {
        let secs = Utc::now().signed_duration_since(at).num_seconds().max(0);
        let ago = if secs < 5 {
            "just now".to_string()
        } else if secs < 120 {
            format!("{secs}s ago")
        } else {
            format!("{}m ago", secs / 60)
        };
        Line::from(vec![
            Span::styled(" Last poll: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                Style::default().fg(Color::Gray),
            ),
            Span::styled(format!("  ({ago})"), Style::default().fg(Color::DarkGray)),
        ])
    } else {
        Line::from(Span::styled(
            " Waiting for first poll…",
            Style::default().fg(Color::DarkGray),
        ))
    };

    let block_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    f.render_widget(Paragraph::new(title_line), block_area[0]);
    f.render_widget(Paragraph::new(poll_line), block_area[1]);
}

fn draw_detail(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(DetailPane { app }, area);
}

fn draw_footer(f: &mut Frame, area: Rect, _app: &App) {
    let msg = " ↑/↓ select   q quit   ? help";
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            msg,
            Style::default().fg(Color::DarkGray),
        ))),
        area,
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmos::meta::{Cursor, CursorKey};
    use crate::tui::app::App;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn sample_app() -> App {
        let mut app = App::new();
        app.handle_poll_result(Ok(vec![
            (
                CursorKey {
                    deployment_name: "prod".into(),
                    source_name: "jira-cloud".into(),
                    subsource: "DO".into(),
                },
                Cursor {
                    documents_synced_total: 1842,
                    ..Default::default()
                },
            ),
            (
                CursorKey {
                    deployment_name: "prod".into(),
                    source_name: "jira-cloud".into(),
                    subsource: "INT".into(),
                },
                Cursor {
                    documents_synced_total: 312,
                    ..Default::default()
                },
            ),
        ]));
        app
    }

    #[test]
    fn draw_does_not_panic() {
        let app = sample_app();
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| draw(f, &app)).unwrap();
    }

    #[test]
    fn draw_with_help_overlay_does_not_panic() {
        let mut app = sample_app();
        app.help_visible = true;
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| draw(f, &app)).unwrap();
    }

    #[test]
    fn draw_with_empty_rows_does_not_panic() {
        let app = App::new();
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| draw(f, &app)).unwrap();
    }

    fn rendered_text(app: &App) -> String {
        let backend = TestBackend::new(120, 40);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        let buf = term.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn header_shows_quelch_status() {
        let app = sample_app();
        let text = rendered_text(&app);
        assert!(text.contains("Quelch Status"), "missing title:\n{text}");
    }

    #[test]
    fn footer_shows_key_hints() {
        let app = sample_app();
        let text = rendered_text(&app);
        assert!(text.contains("quit"), "missing quit hint:\n{text}");
        assert!(text.contains("help"), "missing help hint:\n{text}");
    }
}
