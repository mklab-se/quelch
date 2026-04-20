//! Sources pane. Every cell is keyed to destination-side truth — what
//! actually landed in Azure AI Search — with a live badge while a push is
//! in flight so the number is understood as "authoritative + in-flight"
//! rather than "frozen". Columns: Source · Stage · Pushed · Per min ·
//! Latest ID · Pushed at.

use std::time::Instant;

use chrono::{DateTime, Local, Utc};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Cell, Row, Table, Widget},
};

use crate::tui::app::{App, SourceState, SourceView, SubsourceState, SubsourceView};
use crate::tui::events::Stage;

pub struct SourceTable<'a> {
    pub app: &'a App,
}

impl Widget for SourceTable<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let now = Instant::now();
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
                now,
            ));

            if !collapsed {
                for (ssi, sub) in src.subsources.iter().enumerate() {
                    let is_sub_selected = si == sel_src && sel_sub == Some(ssi);
                    rows.push(subsource_row(
                        sub,
                        is_sub_selected,
                        self.app.spinner_glyph(),
                        now,
                    ));
                }
            }
        }

        let widths = [
            Constraint::Length(22),
            Constraint::Length(20),
            Constraint::Length(11), // number + optional live dot
            Constraint::Length(9),
            Constraint::Min(24),
            Constraint::Length(20),
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
        Cell::from("Pushed at (local)"),
    ])
    .style(Style::default().fg(Color::DarkGray))
}

fn rule_row() -> Row<'static> {
    Row::new(vec![
        Cell::from("──────────────────────"),
        Cell::from("────────────────────"),
        Cell::from("───────────"),
        Cell::from("─────────"),
        Cell::from("────────────────────────"),
        Cell::from("────────────────────"),
    ])
    .style(Style::default().fg(Color::DarkGray))
}

fn source_row(
    src: &SourceView,
    collapsed: bool,
    selected: bool,
    spin: char,
    now: Instant,
) -> Row<'static> {
    let name_col = format!(
        "{arrow} {name}",
        arrow = if collapsed { "▸" } else { "▾" },
        name = src.name,
    );
    // Prefer the authoritative source-level index count; fall back to summing
    // per-subsource counts (+ session deltas) during the brief window before
    // the first count query returns.
    let sum_subs: u64 = src.subsources.iter().map(|s| s.displayed_pushed()).sum();
    let pushed_display = src
        .index_count
        .map(|c| c.to_string())
        .unwrap_or_else(|| sum_subs.to_string());
    let pushes_per_min: u64 = src
        .subsources
        .iter()
        .map(|s| s.push_throughput.per_minute_at(now))
        .sum();
    let latest = src
        .subsources
        .iter()
        .filter_map(|s| s.last_pushed_at.zip(s.last_pushed_id.as_ref()))
        .max_by_key(|(ts, _)| *ts)
        .map(|(_, id)| id.clone());
    let latest_ts = src.subsources.iter().filter_map(|s| s.last_pushed_at).max();
    let any_live = src.subsources.iter().any(|s| s.is_live());

    let row = Row::new(vec![
        Cell::from(name_col).style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from(format_source_state(&src.state, spin)),
        Cell::from(pushed_cell(pushed_display, any_live)),
        Cell::from(Text::from(pushes_per_min.to_string()).alignment(Alignment::Right)),
        Cell::from(latest.unwrap_or_else(|| "—".into())),
        Cell::from(latest_ts.map(format_local_ts).unwrap_or_else(|| "—".into())),
    ]);
    if selected {
        row.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        row
    }
}

fn subsource_row(sub: &SubsourceView, selected: bool, spin: char, now: Instant) -> Row<'static> {
    let name_col = format!("    {name}", name = sub.key);
    let pushes_per_min = sub.push_throughput.per_minute_at(now);
    let latest_id = sub.last_pushed_id.as_deref().unwrap_or("—").to_string();
    let pushed_at = sub
        .last_pushed_at
        .map(format_local_ts)
        .unwrap_or_else(|| "—".into());

    let row = Row::new(vec![
        Cell::from(name_col),
        Cell::from(format_subsource_stage(&sub.state, &sub.stage, spin)),
        Cell::from(pushed_cell(
            sub.displayed_pushed().to_string(),
            sub.is_live(),
        )),
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

/// Right-aligned "N" with a trailing green `●` when the row is in the push
/// pipeline. The dot is the "live" badge — signals that the displayed
/// number is authoritative + in-flight (not a stale snapshot).
fn pushed_cell(value: String, live: bool) -> Text<'static> {
    if live {
        Text::from(Line::from(vec![
            Span::styled(format!("{value:>8} "), Style::default().fg(Color::White)),
            Span::styled("●", Style::default().fg(Color::Green)),
        ]))
    } else {
        Text::from(value).alignment(Alignment::Right)
    }
}

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

/// Format a UTC instant in local time as `YYYY-MM-DD HH:MM:SS`. One
/// format used everywhere in the TUI so the operator never has to decode
/// mixed 24h-only / ISO-8601 / UTC-vs-local across columns.
pub fn format_local_ts(ts: DateTime<Utc>) -> String {
    ts.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}
