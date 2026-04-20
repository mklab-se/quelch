use std::collections::VecDeque;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Cell, Row, Table, Widget},
};

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
            .title("Log (tail)");
        let inner = block.inner(area);
        block.render(area, buf);

        let rows_visible = inner.height.saturating_sub(2) as usize;
        let start = self.lines.len().saturating_sub(rows_visible);

        let header = Row::new(vec![
            Cell::from("LEVEL"),
            Cell::from("TIME"),
            Cell::from("TARGET"),
            Cell::from("MESSAGE"),
        ])
        .style(
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        );

        let rule = Row::new(vec![
            Cell::from("─────"),
            Cell::from("────────"),
            Cell::from("────────────────────────"),
            Cell::from("────────────────────────"),
        ])
        .style(Style::default().fg(Color::DarkGray));

        let mut rows = vec![header, rule];
        for line in self.lines.iter().skip(start) {
            let lvl = format!("{:>5}", format!("{}", line.level));
            let time = line.ts.format("%H:%M:%S").to_string();
            let target = line.target.clone();
            rows.push(Row::new(vec![
                Cell::from(Span::styled(
                    lvl,
                    Style::default().fg(level_colour(&line.level)),
                )),
                Cell::from(time),
                Cell::from(target),
                Cell::from(line.message.clone()),
            ]));
        }

        Table::new(
            rows,
            [
                Constraint::Length(5),
                Constraint::Length(8),
                Constraint::Length(24),
                Constraint::Min(20),
            ],
        )
        .column_spacing(1)
        .render(inner, buf);
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
