use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use std::collections::VecDeque;

use crate::tui::app::LogLine;

pub struct LogView<'a> {
    pub lines: &'a VecDeque<LogLine>,
    pub focused: bool,
}

impl Widget for LogView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border = if self.focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title("Log");
        let inner = block.inner(area);
        block.render(area, buf);

        if self.lines.is_empty() {
            Paragraph::new("Waiting for log events").render(inner, buf);
            return;
        }

        let height = inner.height as usize;
        let start = self.lines.len().saturating_sub(height);
        let lines: Vec<Line> = self.lines
            .iter()
            .skip(start)
            .map(|l| {
                Line::from(vec![
                    Span::styled(
                        format!("{:>5}", format!("{:?}", l.level)),
                        Style::default().fg(level_colour(&l.level)),
                    ),
                    Span::raw(" "),
                    Span::raw(format!("{} {}", l.target, l.message)),
                ])
            })
            .collect();
        Paragraph::new(lines).wrap(Wrap { trim: true }).render(inner, buf);
    }
}

fn level_colour(l: &tracing::Level) -> Color {
    match *l {
        tracing::Level::ERROR => Color::Red,
        tracing::Level::WARN => Color::Yellow,
        tracing::Level::INFO => Color::Green,
        tracing::Level::DEBUG => Color::Cyan,
        tracing::Level::TRACE => Color::Gray,
    }
}
