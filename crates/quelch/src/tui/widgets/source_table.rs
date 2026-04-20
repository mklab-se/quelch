//! Sources pane: one row per source + one per subsource. Every column is
//! keyed to what *actually landed in Azure AI Search* — never to what was
//! merely fetched from Jira/Confluence. Columns: Source · Stage · Pushed
//! · Per min · Latest ID · Pushed at.

use chrono::Utc;
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Text,
    widgets::{Cell, Row, Table, Widget},
};

use crate::tui::app::{App, SourceState, SourceView, SubsourceState, SubsourceView};
use crate::tui::events::Stage;

pub struct SourceTable<'a> {
    pub app: &'a App,
}

impl Widget for SourceTable<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut rows: Vec<Row> = vec![header_row(), rule_row()];

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
            Constraint::Length(20),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Min(24),
            Constraint::Length(12),
        ];

        Table::new(rows, widths).column_spacing(1).render(area, buf);
    }
}

fn header_row() -> Row<'static> {
    Row::new(vec![
        Cell::from("Source"),
        Cell::from("Stage"),
        Cell::from(Text::from("Pushed").alignment(Alignment::Right)),
        Cell::from(Text::from("Per min").alignment(Alignment::Right)),
        Cell::from("Latest ID"),
        Cell::from("Pushed at"),
    ])
    .style(Style::default().fg(Color::DarkGray))
}

fn rule_row() -> Row<'static> {
    Row::new(vec![
        Cell::from("──────────────────────"),
        Cell::from("────────────────────"),
        Cell::from("─────────"),
        Cell::from("─────────"),
        Cell::from("────────────────────────"),
        Cell::from("────────────"),
    ])
    .style(Style::default().fg(Color::DarkGray))
}

fn source_row(src: &SourceView, collapsed: bool, selected: bool, spin: char) -> Row<'static> {
    let name_col = format!(
        "{arrow} {name}",
        arrow = if collapsed { "▸" } else { "▾" },
        name = src.name,
    );
    let pushed_total: u64 = src.subsources.iter().map(|s| s.pushed_total).sum();
    let pushes_per_min: u64 = src
        .subsources
        .iter()
        .map(|s| s.push_throughput.samples().iter().sum::<u64>())
        .sum();
    let latest = src
        .subsources
        .iter()
        .filter_map(|s| s.last_pushed_at.zip(s.last_pushed_id.as_ref()))
        .max_by_key(|(ts, _)| *ts)
        .map(|(_, id)| id.clone());
    let latest_ts = src.subsources.iter().filter_map(|s| s.last_pushed_at).max();

    let row = Row::new(vec![
        Cell::from(name_col).style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from(format_source_state(&src.state, spin)),
        Cell::from(Text::from(pushed_total.to_string()).alignment(Alignment::Right)),
        Cell::from(Text::from(pushes_per_min.to_string()).alignment(Alignment::Right)),
        Cell::from(latest.unwrap_or_else(|| "—".into())),
        Cell::from(latest_ts.map(format_relative).unwrap_or_else(|| "—".into())),
    ]);
    if selected {
        row.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        row
    }
}

fn subsource_row(sub: &SubsourceView, selected: bool, spin: char) -> Row<'static> {
    let name_col = format!("    {name}", name = sub.key);
    let pushes_per_min: u64 = sub.push_throughput.samples().iter().sum();
    let latest_id = sub.last_pushed_id.as_deref().unwrap_or("—").to_string();
    let pushed_at = sub
        .last_pushed_at
        .map(format_relative)
        .unwrap_or_else(|| "—".into());

    let row = Row::new(vec![
        Cell::from(name_col),
        Cell::from(format_subsource_stage(&sub.state, &sub.stage, spin)),
        Cell::from(Text::from(sub.pushed_total.to_string()).alignment(Alignment::Right)),
        Cell::from(Text::from(pushes_per_min.to_string()).alignment(Alignment::Right)),
        Cell::from(latest_id),
        Cell::from(pushed_at),
    ]);
    if selected {
        row.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        row
    }
}

/// Source-level status. Aggregate of its subsource states — no per-batch
/// stage detail at this level (would flicker too fast to be readable).
fn format_source_state(state: &SourceState, spin: char) -> Text<'static> {
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

/// Subsource-level status. Reflects the current pipeline stage so the
/// operator can distinguish fetching-from-Jira from embedding from pushing
/// to Azure — the most common "what is quelch doing right now?" question.
fn format_subsource_stage(state: &SubsourceState, stage: &Stage, spin: char) -> Text<'static> {
    match state {
        SubsourceState::Error(_) => Text::from("● error").style(Style::default().fg(Color::Red)),
        SubsourceState::Idle => Text::from("● idle").style(Style::default().fg(Color::Green)),
        SubsourceState::Syncing => match stage {
            Stage::Fetching => {
                Text::from(format!("{spin} fetching")).style(Style::default().fg(Color::Cyan))
            }
            Stage::Embedding { done, total } => Text::from(format!("{spin} embed {done}/{total}"))
                .style(Style::default().fg(Color::Cyan)),
            Stage::Pushing { total } => Text::from(format!("{spin} pushing {total}"))
                .style(Style::default().fg(Color::Cyan)),
            Stage::Idle => {
                Text::from(format!("{spin} syncing")).style(Style::default().fg(Color::Cyan))
            }
        },
    }
}

/// Short relative time string — "now", "Ns ago", "Nm ago", "Nh ago".
/// Used for the "Pushed at" column so the user can see at a glance whether
/// items are currently landing or whether the source has gone quiet.
fn format_relative(ts: chrono::DateTime<Utc>) -> String {
    let diff = Utc::now().signed_duration_since(ts);
    let secs = diff.num_seconds();
    if secs < 2 {
        "now".into()
    } else if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}
