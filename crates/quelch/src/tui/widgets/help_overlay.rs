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
        let modal_w = 40u16.min(area.width.saturating_sub(4));
        let modal_h = 10u16.min(area.height.saturating_sub(2));
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
            .title("Quelch TUI")
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(outer);
        block.render(outer, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        let body = vec![
            kv("↑/↓", "navigate"),
            kv("q/Esc", "quit"),
            kv("?", "toggle help"),
        ];
        Paragraph::new(body).render(chunks[0], buf);
        Paragraph::new(Line::from(Span::styled(
            "press ? or Esc to dismiss",
            Style::default().fg(Color::DarkGray),
        )))
        .render(chunks[1], buf);
    }
}

fn kv(k: &'static str, v: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {k:<10}"), Style::default().fg(Color::Cyan)),
        Span::raw(v),
    ])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn help_overlay_lists_key_bindings() {
        let mut term = Terminal::new(TestBackend::new(60, 20)).unwrap();
        term.draw(|f| f.render_widget(HelpOverlay, f.area()))
            .unwrap();
        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| buf[(x, y)].symbol().to_string()))
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("Quelch TUI"), "title missing: {text}");
        assert!(text.contains("navigate"), "navigate hint missing: {text}");
        assert!(text.contains("quit"), "quit hint missing: {text}");
        assert!(text.contains("toggle help"), "help hint missing: {text}");
    }
}
