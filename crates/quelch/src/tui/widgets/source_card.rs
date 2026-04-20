use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

use crate::tui::app::{Focus, SourceState, SourceView, SubsourceState};

pub struct SourceCard<'a> {
    pub view: &'a SourceView,
    pub collapsed: bool,
    pub focused: bool,
    pub focused_subsource: Option<&'a str>,
}

impl Widget for SourceCard<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(format!("{} ({})", self.view.name, self.view.kind));
        let inner = block.inner(area);
        block.render(area, buf);

        let total_docs: u64 = self
            .view
            .subsources
            .iter()
            .map(|sub| sub.docs_synced_total)
            .sum();
        let active_subsources = self
            .view
            .subsources
            .iter()
            .filter(|sub| matches!(sub.state, SubsourceState::Syncing))
            .count();
        let mut lines: Vec<Line> = vec![Line::from(vec![
            state_line(&self.view.state),
            Span::raw(format!(
                "  {} subsources  {} active  {} docs total",
                self.view.subsources.len(),
                active_subsources,
                total_docs
            )),
        ])];
        if !self.collapsed {
            for sub in &self.view.subsources {
                let marker = if Some(sub.key.as_str()) == self.focused_subsource {
                    "›"
                } else {
                    " "
                };
                let status = match &sub.state {
                    SubsourceState::Idle => "idle",
                    SubsourceState::Syncing => "syncing",
                    SubsourceState::Error(_) => "error",
                };
                let last = sub
                    .last_sample_id
                    .as_deref()
                    .map(str::to_string)
                    .or_else(|| sub.last_cursor.map(|cursor| cursor.to_rfc3339()))
                    .unwrap_or_else(|| "waiting".into());
                lines.push(Line::from(vec![
                    Span::raw(format!("{marker} ")),
                    Span::styled(
                        format!("{:12}", sub.key),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(format!(
                        "  {:8}  +{} docs  last {}",
                        status,
                        sub.docs_synced_total,
                        last
                    )),
                ]));
                if !sub.last_errors.is_empty() {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("recent error: ", Style::default().fg(Color::Red)),
                        Span::raw(sub.last_errors.back().cloned().unwrap_or_default()),
                    ]));
                }
            }
        }
        Paragraph::new(lines).wrap(Wrap { trim: true }).render(inner, buf);
    }
}

fn state_line(s: &SourceState) -> Span<'_> {
    match s {
        SourceState::Idle => Span::styled("[idle]", Style::default().fg(Color::Green)),
        SourceState::Syncing => Span::styled("[syncing]", Style::default().fg(Color::Cyan)),
        SourceState::Error(_) => Span::styled("[error]", Style::default().fg(Color::Red)),
        SourceState::Backoff { .. } => {
            Span::styled("[backoff]", Style::default().fg(Color::Yellow))
        }
    }
}

#[allow(dead_code)]
pub fn _referenced(_: Focus) {}
