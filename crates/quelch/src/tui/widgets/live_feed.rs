//! Live feed — one row per pushed batch, newest first. Each row shows the
//! batch size, the first few doc IDs verbatim, and a tail count so the
//! operator reads "batch of 92 · DO-1, DO-2, DO-3, DO-4, DO-5, ... (87 more)"
//! rather than 92 near-identical rows with the same timestamp.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use crate::tui::app::App;
use crate::tui::widgets::source_table::format_local_ts;

pub struct LiveFeed<'a> {
    pub app: &'a App,
}

impl Widget for LiveFeed<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title("Pushed to Azure AI Search (newest first, local time)");
        let inner = block.inner(area);
        block.render(area, buf);

        if self.app.live_feed.is_empty() {
            Paragraph::new(Line::from(Span::styled(
                "  (nothing pushed yet — waiting for the first batch)",
                Style::default().fg(Color::DarkGray),
            )))
            .render(inner, buf);
            return;
        }

        let rows_visible = inner.height as usize;
        let lines: Vec<Line> = self
            .app
            .live_feed
            .iter()
            .take(rows_visible)
            .map(|batch| {
                let time = format_local_ts(batch.ts);
                let shown = batch.sample_ids.len() as u64;
                let tail = batch.count.saturating_sub(shown);
                let samples = batch.sample_ids.join(", ");
                let items = if tail > 0 {
                    format!("{samples}, … ({tail} more)")
                } else {
                    samples
                };
                Line::from(vec![
                    Span::styled(" ● ", Style::default().fg(Color::Green)),
                    Span::styled(time, Style::default().fg(Color::DarkGray)),
                    Span::raw("  "),
                    Span::styled(
                        format!("{}/{}", batch.source, batch.subsource),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("batch of {}", batch.count),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(ratatui::style::Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::raw(items),
                ])
            })
            .collect();

        Paragraph::new(lines).render(inner, buf);
    }
}
