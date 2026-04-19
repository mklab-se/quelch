use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols,
    text::Line,
    widgets::{Block, Borders, Paragraph, Sparkline, Widget},
};

use crate::tui::metrics::AzurePanel;

pub struct AzurePanelWidget<'a> {
    pub panel: &'a AzurePanel,
    pub drops: u64,
    pub focused: bool,
}

impl Widget for AzurePanelWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border = if self.focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title("Azure AI Search");
        let inner = block.inner(area);
        block.render(area, buf);

        let req_samples = self.panel.requests_per_sec.samples();
        let err_samples = self.panel.errors_5xx_per_sec.samples();
        let (p50, p95) = self.panel.p50_p95();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(inner);

        let req = Sparkline::default()
            .data(&req_samples)
            .bar_set(symbols::bar::NINE_LEVELS)
            .style(Style::default().fg(Color::Green));
        req.render(chunks[0], buf);

        let err = Sparkline::default()
            .data(&err_samples)
            .bar_set(symbols::bar::NINE_LEVELS)
            .style(Style::default().fg(Color::Red));
        err.render(chunks[1], buf);

        let counters = format!(
            "total {}  p50 {}ms  p95 {}ms  4xx {}  5xx {}  throttled {}  drops {}",
            self.panel.total,
            p50.as_millis(),
            p95.as_millis(),
            self.panel.count_4xx,
            self.panel.count_5xx,
            self.panel.count_throttled,
            self.drops
        );
        Paragraph::new(Line::from(counters)).render(chunks[2], buf);
    }
}
