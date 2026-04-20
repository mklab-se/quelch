use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use super::app::{App, EngineStatus, Focus};
use super::widgets::{azure_panel::AzurePanelWidget, log_view::LogView};

pub fn draw(f: &mut Frame, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(12),
            Constraint::Length(7),
            Constraint::Length(2),
        ])
        .split(f.area());

    f.render_widget(Clear, f.area());
    draw_header(f, areas[0], app);
    if app.prefs.log_view_on {
        f.render_widget(
            LogView {
                lines: &app.log_tail,
                focused: matches!(app.focus, Focus::Sources),
            },
            areas[1],
        );
    } else {
        draw_sources(f, areas[1], app);
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
    let selected = match (app.focused_source_name(), app.focused_subsource_name()) {
        (Some(source), Some(subsource)) => format!("selected {source}/{subsource}"),
        (Some(source), None) => format!("selected {source}"),
        _ => "no sources".into(),
    };
    f.render_widget(
        Paragraph::new(Line::from(format!(
            " quelch v{}  {status}  {selected}",
            env!("CARGO_PKG_VERSION")
        )))
        .style(Style::default().fg(Color::White)),
        area,
    );
}

fn draw_sources(f: &mut Frame, area: Rect, app: &App) {
    use crate::tui::widgets::source_table::SourceTable;

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(if matches!(app.focus, Focus::Sources) {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        })
        .title("Sources");
    let inner = outer.inner(area);
    outer.render(area, f.buffer_mut());

    if app.sources.is_empty() {
        f.render_widget(Paragraph::new("No sources configured"), inner);
        return;
    }

    f.render_widget(SourceTable { app }, inner);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let msg = if app.footer.is_empty() {
        " arrows move  enter collapse  r sync-now  p pause  s logs  tab focus  q quit".to_string()
    } else {
        format!(" {}", app.footer)
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(msg),
            Line::from(
                " q quit  arrows move  enter collapse  r sync-now  p pause  s logs  tab focus",
            ),
        ])
        .style(Style::default().fg(Color::Gray)),
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
            draw(f, &app);
        })
        .unwrap();
    }
}
