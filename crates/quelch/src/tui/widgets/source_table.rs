//! Table-based Sources pane: columns + headings, tree-indented rows,
//! per-row state glyph, selected-row inverse-video highlight.

use chrono::Utc;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Text,
    widgets::{Cell, Row, Table, Widget},
};

use crate::tui::app::{App, SourceState, SourceView, SubsourceState, SubsourceView};

pub struct SourceTable<'a> {
    pub app: &'a App,
}

impl Widget for SourceTable<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut rows: Vec<Row> = vec![header_row()];
        rows.push(rule_row());

        let sel_src = self.app.selected_source;
        let sel_sub = self.app.selected_subsource;

        for (si, src) in self.app.sources.iter().enumerate() {
            let collapsed = self.app.prefs.is_source_collapsed(&src.name);
            let is_src_selected = si == sel_src && sel_sub.is_none();
            rows.push(source_row(
                src,
                collapsed,
                is_src_selected,
                self.app.spinner_glyph(),
            ));

            if !collapsed {
                for (ssi, sub) in src.subsources.iter().enumerate() {
                    let is_sub_selected = si == sel_src && sel_sub == Some(ssi);
                    rows.push(subsource_row(
                        sub,
                        is_sub_selected,
                        self.app.spinner_glyph(),
                    ));
                }
            }
        }

        let widths = [
            Constraint::Length(22),
            Constraint::Length(14),
            Constraint::Length(8),
            Constraint::Length(11),
            Constraint::Length(18),
            Constraint::Min(10),
        ];

        Table::new(rows, widths).column_spacing(1).render(area, buf);
    }
}

fn header_row() -> Row<'static> {
    Row::new(vec![
        Cell::from("Source"),
        Cell::from("Status"),
        Cell::from(Text::from("Items").alignment(ratatui::layout::Alignment::Right)),
        Cell::from("Rate"),
        Cell::from("Last item"),
        Cell::from("Updated"),
    ])
    .style(Style::default().fg(Color::DarkGray))
}

fn rule_row() -> Row<'static> {
    Row::new(vec![
        Cell::from("──────────────────"),
        Cell::from("────────────"),
        Cell::from("──────"),
        Cell::from("─────────"),
        Cell::from("────────────────"),
        Cell::from("────────"),
    ])
    .style(Style::default().fg(Color::DarkGray))
}

fn source_row(src: &SourceView, collapsed: bool, selected: bool, spin: char) -> Row<'static> {
    let name_col = format!(
        "{arrow} {name}",
        arrow = if collapsed { "▸" } else { "▾" },
        name = src.name,
    );
    let total_docs: u64 = src.subsources.iter().map(|s| s.docs_synced_total).sum();
    let row = Row::new(vec![
        Cell::from(name_col).style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from(format_state_src(&src.state, spin)),
        Cell::from(Text::from(total_docs.to_string()).alignment(ratatui::layout::Alignment::Right)),
        Cell::from("—"),
        Cell::from("—"),
        Cell::from("—"),
    ]);
    if selected {
        row.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        row
    }
}

fn subsource_row(sub: &SubsourceView, selected: bool, spin: char) -> Row<'static> {
    let name_col = format!("    {name}", name = sub.key);
    let items = sub.docs_synced_total.to_string();
    let rate_label = if sub.docs_synced_total == 0 && !matches!(sub.state, SubsourceState::Syncing)
    {
        "—".to_string()
    } else {
        let per_min: u64 = sub.throughput.samples().iter().sum();
        format!("{:.1}/min", per_min as f32)
    };
    let last_item = sub.last_sample_id.as_deref().unwrap_or("—").to_string();
    let updated = sub
        .last_cursor
        .map(format_relative)
        .unwrap_or_else(|| "—".into());

    let row = Row::new(vec![
        Cell::from(name_col),
        Cell::from(format_state_sub(&sub.state, spin)),
        Cell::from(Text::from(items).alignment(ratatui::layout::Alignment::Right)),
        Cell::from(rate_label),
        Cell::from(last_item),
        Cell::from(updated),
    ]);
    if selected {
        row.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        row
    }
}

fn format_state_src(state: &SourceState, spin: char) -> Text<'static> {
    match state {
        SourceState::Idle => Text::from("● idle").style(Style::default().fg(Color::Green)),
        SourceState::Syncing => {
            Text::from(format!("{spin} syncing")).style(Style::default().fg(Color::Cyan))
        }
        SourceState::Error(_) => Text::from("● error").style(Style::default().fg(Color::Red)),
        SourceState::Backoff { .. } => {
            Text::from("◉ backoff").style(Style::default().fg(Color::Yellow))
        }
    }
}

fn format_state_sub(state: &SubsourceState, spin: char) -> Text<'static> {
    match state {
        SubsourceState::Idle => Text::from("● idle").style(Style::default().fg(Color::Green)),
        SubsourceState::Syncing => {
            Text::from(format!("{spin} syncing")).style(Style::default().fg(Color::Cyan))
        }
        SubsourceState::Error(_) => Text::from("● error").style(Style::default().fg(Color::Red)),
    }
}

fn format_relative(ts: chrono::DateTime<Utc>) -> String {
    let diff = Utc::now().signed_duration_since(ts);
    let secs = diff.num_seconds();
    if secs < 5 {
        "now".into()
    } else if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m {}s ago", secs / 60, secs % 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}
