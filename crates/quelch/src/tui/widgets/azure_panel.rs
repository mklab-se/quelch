use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph, Widget},
};

use crate::tui::metrics::AzurePanel;

pub struct AzurePanelWidget<'a> {
    pub panel: &'a AzurePanel,
    pub drops: u64,
    pub focused: bool,
    pub backoff_reason: Option<&'a str>,
}

impl Widget for AzurePanelWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title("Azure AI Search");
        let inner = block.inner(area);
        block.render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // backoff banner OR subtitle
                Constraint::Min(5),    // chart
                Constraint::Length(3), // counter strip (3 rows)
            ])
            .split(inner);

        // --- Row 1: backoff banner OR chart subtitle with max ---
        let subtitle_max = self
            .panel
            .requests_per_sec
            .samples()
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        let subtitle = if let Some(reason) = self.backoff_reason {
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "◉ Azure client backing off",
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw("  "),
                Span::styled(reason.to_string(), Style::default().fg(Color::Yellow)),
            ]))
        } else {
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "Requests per second (last 60s)",
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw("    "),
                Span::styled(
                    format!("max {subtitle_max} req/s"),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        };
        subtitle.render(chunks[0], buf);

        // --- Row 2: chart ---
        let points: Vec<(f64, f64)> = self.panel.requests_per_sec.chart_points();
        let y_max = (subtitle_max as f64).max(1.0);
        let dataset = Dataset::default()
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Cyan))
            .data(&points);
        let x_labels: Vec<Line> = vec![Line::from("-60s"), Line::from("now")];
        let y_labels: Vec<Line> = vec![Line::from("0"), Line::from(format!("{}", y_max as u64))];
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

        // --- Row 3: counter strip ---
        let (p50, p95) = self.panel.p50_p95();
        let bad_color = |n: u64, bad: Color| if n == 0 { Color::DarkGray } else { bad };
        let rows = vec![
            Line::from(vec![
                Span::styled("Total requests  ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!("{:<8}", self.panel.total)),
                Span::styled("Latency      ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!(
                    "median {} ms · 95th {} ms",
                    p50.as_millis(),
                    p95.as_millis()
                )),
            ]),
            Line::from(vec![
                Span::styled("Failed (4xx)    ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:<8}", self.panel.count_4xx),
                    Style::default().fg(bad_color(self.panel.count_4xx, Color::Red)),
                ),
                Span::styled("Throttled (429)  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", self.panel.count_throttled),
                    Style::default().fg(bad_color(self.panel.count_throttled, Color::Yellow)),
                ),
            ]),
            Line::from(vec![
                Span::styled("Failed (5xx)    ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:<8}", self.panel.count_5xx),
                    Style::default().fg(bad_color(self.panel.count_5xx, Color::Red)),
                ),
                Span::styled("Dropped events   ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", self.drops),
                    Style::default().fg(bad_color(self.drops, Color::Yellow)),
                ),
            ]),
        ];
        Paragraph::new(rows).render(chunks[2], buf);
    }
}
