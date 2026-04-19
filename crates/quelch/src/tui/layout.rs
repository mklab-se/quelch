use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

use super::app::{App, EngineStatus, Focus};
use super::widgets::{azure_panel::AzurePanelWidget, log_view::LogView, source_card::SourceCard};

pub struct LayoutOptions<'a> {
    pub focused_source: Option<&'a str>,
    pub focused_subsource: Option<&'a str>,
}

pub fn draw(f: &mut Frame, app: &App, opts: LayoutOptions) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(6),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_header(f, areas[0], app);
    if app.prefs.log_view_on {
        f.render_widget(
            LogView {
                lines: app.log_tail.as_slices().0,
                focused: matches!(app.focus, Focus::Sources),
            },
            areas[1],
        );
    } else {
        draw_sources(f, areas[1], app, opts);
    }
    f.render_widget(
        AzurePanelWidget {
            panel: &app.azure,
            drops: app.drops,
            focused: matches!(app.focus, Focus::Azure),
        },
        areas[2],
    );
    draw_footer(f, areas[3], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let status = match &app.status {
        EngineStatus::Idle => "● idle".to_string(),
        EngineStatus::Syncing { cycle, .. } => format!("● watching · cycle {cycle}"),
        EngineStatus::Paused => "⏸ paused".to_string(),
        EngineStatus::Shutdown => "⏹ shutdown".to_string(),
    };
    f.render_widget(
        Paragraph::new(Line::from(format!(
            " quelch v{}  {status}",
            env!("CARGO_PKG_VERSION")
        ))),
        area,
    );
}

fn draw_sources(f: &mut Frame, area: Rect, app: &App, opts: LayoutOptions) {
    if app.sources.is_empty() {
        f.render_widget(
            Block::default().borders(Borders::ALL).title("Sources"),
            area,
        );
        return;
    }
    let rows: Vec<(&crate::tui::app::SourceView, bool, u16)> = app
        .sources
        .iter()
        .map(|s| {
            let collapsed = app.prefs.is_source_collapsed(&s.name);
            let height = if collapsed {
                3
            } else {
                3 + s.subsources.len() as u16
            };
            (s, collapsed, height)
        })
        .collect();
    let constraints: Vec<Constraint> = rows
        .iter()
        .map(|(_, _, h)| Constraint::Length(*h))
        .collect();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    for ((view, collapsed, _), rect) in rows.iter().zip(chunks.iter()) {
        let focused_here = opts.focused_source.map(|n| n == view.name).unwrap_or(false);
        f.render_widget(
            SourceCard {
                view,
                collapsed: *collapsed,
                focused: focused_here,
                focused_subsource: if focused_here {
                    opts.focused_subsource
                } else {
                    None
                },
            },
            *rect,
        );
    }
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let msg = if app.footer.is_empty() {
        " q quit  space collapse  r sync-now  p pause  s logs  tab focus  ? help".to_string()
    } else {
        format!(" {}", app.footer)
    };
    f.render_widget(
        Paragraph::new(Line::from(msg)).style(Style::default().fg(Color::Gray)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AuthConfig, AzureConfig, Config, JiraSourceConfig, SourceConfig, SyncConfig,
    };
    use crate::tui::app::App;
    use crate::tui::prefs::Prefs;
    use ratatui::backend::TestBackend;

    fn cfg() -> Config {
        Config {
            azure: AzureConfig {
                endpoint: "x".into(),
                api_key: "k".into(),
            },
            sources: vec![SourceConfig::Jira(JiraSourceConfig {
                name: "j".into(),
                url: "x".into(),
                auth: AuthConfig::DataCenter { pat: "p".into() },
                projects: vec!["DO".into(), "HR".into()],
                index: "i".into(),
            })],
            sync: SyncConfig::default(),
        }
    }

    #[test]
    fn layout_renders_without_panicking() {
        let app = App::new(&cfg(), Prefs::default());
        let mut term = ratatui::Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| {
            draw(
                f,
                &app,
                LayoutOptions {
                    focused_source: Some("j"),
                    focused_subsource: Some("DO"),
                },
            );
        })
        .unwrap();
    }
}
