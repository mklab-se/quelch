//! Azure AI Search panel. Everything on it is keyed to "documents landing
//! in the destination index", not "HTTP requests" (which are batched and
//! opaque to the operator) and not "latency" (not actionable).

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph, Widget},
};

use crate::tui::app::App;

pub struct AzurePanelWidget<'a> {
    pub app: &'a App,
}

impl Widget for AzurePanelWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title("Azure AI Search");
        let inner = block.inner(area);
        block.render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // subtitle or backoff banner
                Constraint::Min(4),    // chart
                Constraint::Length(2), // counter strip
            ])
            .split(inner);

        // Row 1 — either the chart subtitle, or an attention-grabbing backoff
        // banner. Backoff takes precedence because it's actionable.
        let panel = &self.app.pushes_per_sec;
        let max_per_sec = panel.samples().iter().copied().max().unwrap_or(0);
        let subtitle = if let Some(reason) = self.app.backoff_reason.as_deref() {
            Paragraph::new(Line::from(vec![
                Span::styled("◉ Azure backing off", Style::default().fg(Color::Yellow)),
                Span::raw("  "),
                Span::styled(reason.to_string(), Style::default().fg(Color::Yellow)),
            ]))
        } else {
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "Documents pushed per second (last 60s)",
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw("    "),
                Span::styled(
                    format!("peak {max_per_sec}/s"),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        };
        subtitle.render(chunks[0], buf);

        // Row 2 — braille-rendered line chart of pushes/sec. Y-axis auto-scales
        // to the observed peak (min 1 so a flat zero-line still draws).
        let points: Vec<(f64, f64)> = self.app.pushes_per_sec.chart_points();
        let y_max = (max_per_sec as f64).max(1.0);
        let dataset = Dataset::default()
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Cyan))
            .data(&points);
        let x_labels: Vec<Line> = vec!["-60s".into(), "now".into()];
        let y_labels: Vec<Line> = vec!["0".into(), format!("{}", y_max as u64).into()];
        Chart::new(vec![dataset])
            .x_axis(
                Axis::default()
                    .bounds([0.0, 60.0])
                    .labels(x_labels)
                    .style(Style::default().fg(Color::DarkGray)),
            )
            .y_axis(
                Axis::default()
                    .bounds([0.0, y_max])
                    .labels(y_labels)
                    .style(Style::default().fg(Color::DarkGray)),
            )
            .render(chunks[1], buf);

        // Row 3 — two-column counter strip. Every counter answers a concrete
        // operator question:
        //   "How much has landed?" → Total pushed + Per min
        //   "Is it still working?" → Fail counts and drops (non-zero = red)
        let pushes_per_min: u64 = self.app.pushes_per_sec.samples().iter().sum();
        let bad = |n: u64, bad_colour: Color| {
            if n == 0 { Color::DarkGray } else { bad_colour }
        };
        let rows = vec![
            Line::from(vec![
                Span::styled(" Total pushed   ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!("{:<10}", self.app.pushed_total)),
                Span::styled("Per minute  ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!("{:<8}", pushes_per_min)),
                Span::styled("4xx ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:<4}", self.app.azure.count_4xx),
                    Style::default().fg(bad(self.app.azure.count_4xx, Color::Red)),
                ),
                Span::styled("5xx ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:<4}", self.app.azure.count_5xx),
                    Style::default().fg(bad(self.app.azure.count_5xx, Color::Red)),
                ),
                Span::styled("Throttled ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:<4}", self.app.azure.count_throttled),
                    Style::default().fg(bad(self.app.azure.count_throttled, Color::Yellow)),
                ),
                Span::styled("Dropped ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:<4}", self.app.drops),
                    Style::default().fg(bad(self.app.drops, Color::Yellow)),
                ),
            ]),
            Line::from(""),
        ];
        Paragraph::new(rows).render(chunks[2], buf);
    }
}
