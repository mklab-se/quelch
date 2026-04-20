//! Help overlay — modal list of key bindings.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

pub struct HelpOverlay;

impl Widget for HelpOverlay {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let modal_w = 52u16.min(area.width.saturating_sub(4));
        let modal_h = 22u16.min(area.height.saturating_sub(2));
        let h_pad = (area.width.saturating_sub(modal_w)) / 2;
        let v_pad = (area.height.saturating_sub(modal_h)) / 2;
        let outer = Rect {
            x: area.x + h_pad,
            y: area.y + v_pad,
            width: modal_w,
            height: modal_h,
        };

        Clear.render(outer, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Keyboard shortcuts")
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(outer);
        block.render(outer, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        let body = vec![
            heading("Navigation"),
            kv("↑ ↓", "move up / down"),
            kv("← →", "collapse / expand"),
            kv("Enter", "open drilldown"),
            Line::from(""),
            heading("Actions"),
            kv("r", "sync now"),
            kv("p", "pause / resume"),
            kv("R", "reset cursor (press twice)"),
            kv("P", "purge source (press twice)"),
            Line::from(""),
            heading("View"),
            kv("s", "toggle log view"),
            kv("c", "clear footer flash"),
            Line::from(""),
            heading("Other"),
            kv("?", "this help"),
            kv("q or ^C", "quit"),
        ];
        Paragraph::new(body).render(chunks[0], buf);
        Paragraph::new(Line::from(Span::styled(
            "press ? or Esc to dismiss",
            Style::default().fg(Color::DarkGray),
        )))
        .render(chunks[1], buf);
    }
}

fn heading(s: &'static str) -> Line<'static> {
    Line::from(Span::styled(s, Style::default().fg(Color::Yellow)))
}

fn kv(k: &'static str, v: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {k:<10}"), Style::default().fg(Color::Cyan)),
        Span::raw(v),
    ])
}
