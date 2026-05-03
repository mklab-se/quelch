//! Fleet table widget: renders one row per `(CursorKey, Cursor)`.
//!
//! Columns: Deployment · Source · Subsource · Last sync · Docs · State

use chrono::Utc;
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Widget},
};

use crate::cosmos::meta::{Cursor, CursorKey};
use crate::tui::app::App;

// ---------------------------------------------------------------------------
// Fleet table
// ---------------------------------------------------------------------------

/// Main fleet table widget — one row per `(CursorKey, Cursor)`.
pub struct FleetTable<'a> {
    pub app: &'a App,
}

impl Widget for FleetTable<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let outer = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title("Sources");
        let inner = outer.inner(area);
        outer.render(area, buf);

        if self.app.rows.is_empty() {
            Paragraph::new("(no cursors — nothing has synced yet)")
                .style(Style::default().fg(Color::DarkGray))
                .render(inner, buf);
            return;
        }

        let mut rows: Vec<Row> = vec![header_row(), rule_row()];

        for (i, (key, cursor)) in self.app.rows.iter().enumerate() {
            let selected = i == self.app.selected_index;
            rows.push(data_row(key, cursor, selected));
        }

        let widths = [
            Constraint::Length(22), // deployment
            Constraint::Length(20), // source
            Constraint::Length(12), // subsource
            Constraint::Length(14), // last sync
            Constraint::Length(8),  // docs
            Constraint::Min(12),    // state
        ];

        Table::new(rows, widths)
            .column_spacing(1)
            .render(inner, buf);
    }
}

fn header_row() -> Row<'static> {
    Row::new(vec![
        Cell::from("Deployment"),
        Cell::from("Source"),
        Cell::from("Subsource"),
        Cell::from("Last sync"),
        Cell::from(Text::from("Docs").alignment(Alignment::Right)),
        Cell::from("State"),
    ])
    .style(Style::default().fg(Color::DarkGray))
}

fn rule_row() -> Row<'static> {
    Row::new(vec![
        Cell::from("──────────────────────"),
        Cell::from("────────────────────"),
        Cell::from("────────────"),
        Cell::from("──────────────"),
        Cell::from("────────"),
        Cell::from("────────────"),
    ])
    .style(Style::default().fg(Color::DarkGray))
}

fn data_row(key: &CursorKey, cursor: &Cursor, selected: bool) -> Row<'static> {
    let last_sync = fmt_last_sync(cursor);
    let docs = if cursor.documents_synced_total == 0 {
        "—".to_string()
    } else {
        cursor.documents_synced_total.to_string()
    };
    let selector = if selected { "▶" } else { " " };
    let deployment = format!("{selector} {}", key.deployment_name);

    let state_cell = state_text(cursor);

    let row = Row::new(vec![
        Cell::from(deployment),
        Cell::from(key.source_name.clone()),
        Cell::from(key.subsource.clone()),
        Cell::from(last_sync),
        Cell::from(Text::from(docs).alignment(Alignment::Right)),
        Cell::from(state_cell),
    ]);

    if selected {
        row.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        row
    }
}

fn fmt_last_sync(cursor: &Cursor) -> String {
    if cursor.backfill_in_progress {
        return "backfill…".to_string();
    }
    match cursor.last_sync_at {
        None => "—".to_string(),
        Some(t) => {
            let secs = Utc::now().signed_duration_since(t).num_seconds();
            if secs < 0 {
                "just now".to_string()
            } else if secs < 120 {
                format!("{secs}s ago")
            } else if secs < 7200 {
                format!("{}m ago", secs / 60)
            } else {
                format!("{}h ago", secs / 3600)
            }
        }
    }
}

fn state_text(cursor: &Cursor) -> Text<'static> {
    if cursor.last_error.is_some() {
        return Text::from("error").style(Style::default().fg(Color::Red));
    }
    if cursor.backfill_in_progress {
        return Text::from("backfilling").style(Style::default().fg(Color::Yellow));
    }
    Text::from("ok").style(Style::default().fg(Color::Green))
}

// ---------------------------------------------------------------------------
// Detail pane
// ---------------------------------------------------------------------------

/// Detail pane widget — shown below the table for the selected row.
pub struct DetailPane<'a> {
    pub app: &'a App,
}

impl Widget for DetailPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let outer = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title("Details");
        let inner = outer.inner(area);
        outer.render(area, buf);

        let Some((key, cursor)) = self.app.selected_row() else {
            Paragraph::new("(no selection)")
                .style(Style::default().fg(Color::DarkGray))
                .render(inner, buf);
            return;
        };

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(inner);

        let left = detail_left(key, cursor);
        let right = detail_right(cursor);

        Paragraph::new(left).render(cols[0], buf);
        Paragraph::new(right).render(cols[1], buf);
    }
}

fn fmt_opt_dt(dt: &Option<chrono::DateTime<Utc>>) -> String {
    match dt {
        None => "—".to_string(),
        Some(t) => t.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
    }
}

fn detail_left(key: &CursorKey, cursor: &Cursor) -> Vec<Line<'static>> {
    vec![
        kv("Deployment", &key.deployment_name),
        kv("Source", &key.source_name),
        kv("Subsource", &key.subsource),
    ]
    .into_iter()
    .chain(std::iter::once(kv(
        "Docs synced",
        &cursor.documents_synced_total.to_string(),
    )))
    .chain(std::iter::once(kv(
        "Last sync",
        &fmt_opt_dt(&cursor.last_sync_at),
    )))
    .chain(std::iter::once(kv(
        "Last complete min",
        &fmt_opt_dt(&cursor.last_complete_minute),
    )))
    .collect()
}

fn detail_right(cursor: &Cursor) -> Vec<Line<'static>> {
    let backfill = if cursor.backfill_in_progress {
        "yes".to_string()
    } else {
        "no".to_string()
    };
    let error = cursor
        .last_error
        .as_deref()
        .unwrap_or("—")
        .chars()
        .take(60)
        .collect::<String>();
    let reconciliation = fmt_opt_dt(&cursor.last_reconciliation_at);

    vec![
        kv("Backfill", &backfill),
        kv("Last reconcile", &reconciliation),
        kv("Error", &error),
    ]
}

fn kv(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label:<18} "),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(value.to_string()),
    ])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmos::meta::{Cursor, CursorKey};
    use crate::tui::app::App;
    use chrono::Utc;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(deployment: &str, source: &str, subsource: &str) -> CursorKey {
        CursorKey {
            deployment_name: deployment.to_string(),
            source_name: source.to_string(),
            subsource: subsource.to_string(),
        }
    }

    fn rendered_text(app: &App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            f.render_widget(FleetTable { app }, f.area());
        })
        .unwrap();
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

    fn detail_text(app: &App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            f.render_widget(DetailPane { app }, f.area());
        })
        .unwrap();
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
    fn renders_column_headings() {
        let mut app = App::new();
        app.handle_poll_result(Ok(vec![(key("prod", "jira", "DO"), Cursor::default())]));
        let text = rendered_text(&app, 120, 10);
        assert!(text.contains("Deployment"), "missing Deployment: {text}");
        assert!(text.contains("Source"), "missing Source: {text}");
        assert!(text.contains("Subsource"), "missing Subsource: {text}");
        assert!(text.contains("Last sync"), "missing Last sync: {text}");
        assert!(text.contains("Docs"), "missing Docs: {text}");
        assert!(text.contains("State"), "missing State: {text}");
    }

    #[test]
    fn renders_two_rows() {
        let mut app = App::new();
        app.handle_poll_result(Ok(vec![
            (key("ingest-prod", "jira-cloud", "DO"), Cursor::default()),
            (key("ingest-prod", "jira-cloud", "INT"), Cursor::default()),
        ]));
        let text = rendered_text(&app, 120, 12);
        assert!(text.contains("ingest-prod"), "missing deployment: {text}");
        assert!(text.contains("DO"), "missing DO: {text}");
        assert!(text.contains("INT"), "missing INT: {text}");
    }

    #[test]
    fn backfill_row_shows_distinctly() {
        let mut app = App::new();
        let mut c = Cursor::default();
        c.backfill_in_progress = true;
        app.handle_poll_result(Ok(vec![(key("prod", "jira", "DO"), c)]));
        let text = rendered_text(&app, 120, 10);
        assert!(
            text.contains("backfill"),
            "backfill not shown distinctly: {text}"
        );
    }

    #[test]
    fn error_row_shows_distinctly() {
        let mut app = App::new();
        let mut c = Cursor::default();
        c.last_error = Some("429 rate limit".into());
        app.handle_poll_result(Ok(vec![(key("prod", "jira", "DO"), c)]));
        let text = rendered_text(&app, 120, 10);
        assert!(text.contains("error"), "error not shown distinctly: {text}");
    }

    #[test]
    fn selected_row_has_selection_indicator() {
        let mut app = App::new();
        app.handle_poll_result(Ok(vec![
            (key("prod", "jira", "DO"), Cursor::default()),
            (key("prod", "jira", "INT"), Cursor::default()),
        ]));
        app.selected_index = 1;
        let text = rendered_text(&app, 120, 12);
        // The second row should have the ▶ marker.
        assert!(text.contains('▶'), "selection marker missing: {text}");
    }

    #[test]
    fn empty_table_shows_placeholder() {
        let app = App::new();
        let text = rendered_text(&app, 120, 10);
        assert!(text.contains("no cursors"), "placeholder missing: {text}");
    }

    #[test]
    fn detail_pane_shows_selected_row() {
        let mut app = App::new();
        let mut c = Cursor::default();
        c.documents_synced_total = 9999;
        c.last_sync_at = Some(Utc::now());
        app.handle_poll_result(Ok(vec![(key("prod", "my-jira", "DO"), c)]));
        let text = detail_text(&app, 120, 8);
        assert!(text.contains("prod"), "deployment missing: {text}");
        assert!(text.contains("my-jira"), "source missing: {text}");
        assert!(text.contains("9999"), "doc count missing: {text}");
    }

    #[test]
    fn detail_pane_shows_error_field() {
        let mut app = App::new();
        let mut c = Cursor::default();
        c.last_error = Some("rate limited".into());
        app.handle_poll_result(Ok(vec![(key("prod", "jira", "DO"), c)]));
        let text = detail_text(&app, 120, 8);
        assert!(
            text.contains("rate limited"),
            "error message missing: {text}"
        );
    }

    #[test]
    fn detail_pane_empty_when_no_rows() {
        let app = App::new();
        let text = detail_text(&app, 120, 8);
        assert!(text.contains("no selection"), "placeholder missing: {text}");
    }
}
