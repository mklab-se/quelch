//! Live feed — the ticker the user explicitly asked for: a scrolling list
//! of the most recently pushed documents, newest first. Every entry here
//! means "this item is confirmed in Azure AI Search right now."

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use crate::tui::app::App;

pub struct LiveFeed<'a> {
    pub app: &'a App,
}

impl Widget for LiveFeed<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title("Pushed to Azure AI Search (newest first)");
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
            .enumerate()
            .map(|(i, entry)| {
                // Brightest at the top (most recent), fading to dim deeper in
                // the list so the eye naturally tracks what's new.
                let fade = match i {
                    0 => Color::White,
                    1..=2 => Color::Gray,
                    _ => Color::DarkGray,
                };
                let time = entry.ts.format("%H:%M:%S").to_string();
                Line::from(vec![
                    Span::styled(" ● ", Style::default().fg(Color::Green)),
                    Span::styled(time, Style::default().fg(Color::DarkGray)),
                    Span::raw("  "),
                    Span::styled(
                        format!("{}/{}", entry.source, entry.subsource),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw("  "),
                    Span::styled(entry.id.clone(), Style::default().fg(fade)),
                ])
            })
            .collect();

        Paragraph::new(lines).render(inner, buf);
    }
}
