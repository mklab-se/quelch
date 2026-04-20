//! Drilldown pane: per-subsource detail view triggered by Enter.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use crate::tui::app::{App, SubsourceState, SubsourceView};

pub struct Drilldown<'a> {
    pub app: &'a App,
}

impl Widget for Drilldown<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let sub = match focused_subsource(self.app) {
            Some(s) => s,
            None => {
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title("Drilldown")
                    .border_style(Style::default().fg(Color::DarkGray));
                block.render(area, buf);
                return;
            }
        };

        let src_name = self
            .app
            .sources
            .get(self.app.selected_source)
            .map(|s| s.name.clone())
            .unwrap_or_default();

        let title = format!("{key} ({src_name})", key = sub.key);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        block.render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(7), // summary lines
                Constraint::Min(4),    // recent docs
                Constraint::Length(5), // recent errors
            ])
            .split(inner);

        // Summary
        let status = match &sub.state {
            SubsourceState::Idle => ("● idle", Color::Green),
            SubsourceState::Syncing => ("◐ syncing", Color::Cyan),
            SubsourceState::Error(_) => ("● error", Color::Red),
        };
        let cursor = sub
            .last_cursor
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "—".into());
        let last = sub.last_sample_id.as_deref().unwrap_or("—");
        let rate: u64 = sub.throughput.samples().iter().sum();
        let summary = vec![
            label_line("Status         ", status.0, status.1),
            plain_line("Docs synced    ", &sub.docs_synced_total.to_string()),
            plain_line("Rate (60s)     ", &format!("{rate} per minute")),
            plain_line("Cursor         ", &cursor),
            plain_line("Last item      ", last),
            Line::from(""),
            plain_line("Recent (up to 10)", ""),
        ];
        Paragraph::new(summary).render(chunks[0], buf);

        // Recent docs
        let mut recent_lines: Vec<Line> = Vec::new();
        for doc in sub.recent_docs.iter().rev() {
            let time = doc.ts.format("%H:%M:%S").to_string();
            recent_lines.push(Line::from(vec![
                Span::styled("  ● ", Style::default().fg(Color::Green)),
                Span::raw(time),
                Span::raw("  "),
                Span::raw(doc.id.clone()),
            ]));
        }
        if recent_lines.is_empty() {
            recent_lines.push(Line::from(Span::styled(
                "  (none yet)",
                Style::default().fg(Color::DarkGray),
            )));
        }
        Paragraph::new(recent_lines).render(chunks[1], buf);

        // Recent errors
        let mut err_lines: Vec<Line> = vec![Line::from(Span::styled(
            "Recent errors (last 3)",
            Style::default().fg(Color::DarkGray),
        ))];
        if sub.last_errors.is_empty() {
            err_lines.push(Line::from(Span::styled(
                "  (none)",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for e in sub.last_errors.iter().rev() {
                err_lines.push(Line::from(vec![
                    Span::styled("  × ", Style::default().fg(Color::Red)),
                    Span::raw(e.clone()),
                ]));
            }
        }
        Paragraph::new(err_lines).render(chunks[2], buf);
    }
}

fn focused_subsource(app: &App) -> Option<&SubsourceView> {
    let src = app.sources.get(app.selected_source)?;
    let idx = app.selected_subsource?;
    src.subsources.get(idx)
}

fn label_line(label: &'static str, value: &'static str, colour: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(label, Style::default().fg(Color::DarkGray)),
        Span::styled(value, Style::default().fg(colour)),
    ])
}

fn plain_line(label: &'static str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(label, Style::default().fg(Color::DarkGray)),
        Span::raw(value.to_string()),
    ])
}
