//! Drilldown pane: per-subsource detail view triggered by Enter. Every
//! readout is destination-side — "this is what landed in Azure", not
//! "this is what we fetched from the source."

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Rect},
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

        let chunks = ratatui::layout::Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(7), // summary lines
                Constraint::Min(4),    // last pushed
                Constraint::Length(5), // recent errors
            ])
            .split(inner);

        let status = match &sub.state {
            SubsourceState::Idle => ("● idle", Color::Green),
            SubsourceState::Syncing => ("◐ syncing", Color::Cyan),
            SubsourceState::Error(_) => ("● error", Color::Red),
        };
        let last_id = sub.last_pushed_id.as_deref().unwrap_or("—");
        let last_pushed_at = sub
            .last_pushed_at
            .map(|t| t.format("%H:%M:%S").to_string())
            .unwrap_or_else(|| "—".into());
        let pushed_item_at = sub
            .last_pushed_item_at
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "—".into());
        let rate: u64 = sub.push_throughput.samples().iter().sum();
        let summary = vec![
            label_line("Status           ", status.0, status.1),
            plain_line("Pushed to Azure  ", &format!("{} docs", sub.pushed_total)),
            plain_line("Push rate (60s)  ", &format!("{rate}/min")),
            plain_line("Latest ID        ", last_id),
            plain_line("Pushed at        ", &last_pushed_at),
            plain_line("Source updated   ", &pushed_item_at),
            Line::from(""),
        ];
        Paragraph::new(summary).render(chunks[0], buf);

        // "Last pushed to Azure AI Search" — recent_pushes is populated ONLY
        // on a confirmed DocPushed event, so every line here represents a
        // document that is definitively in the destination index.
        let mut lines: Vec<Line> = vec![Line::from(Span::styled(
            "Last pushed to Azure AI Search (up to 10, newest first)",
            Style::default().fg(Color::DarkGray),
        ))];
        if sub.recent_pushes.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (nothing pushed yet)",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for doc in sub.recent_pushes.iter().rev() {
                let time = doc.ts.format("%H:%M:%S").to_string();
                lines.push(Line::from(vec![
                    Span::styled("  ● ", Style::default().fg(Color::Green)),
                    Span::styled(time, Style::default().fg(Color::DarkGray)),
                    Span::raw("  "),
                    Span::raw(doc.id.clone()),
                ]));
            }
        }
        Paragraph::new(lines).render(chunks[1], buf);

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
