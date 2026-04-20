use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use super::app::App;
use super::status::header_line;
use super::widgets::{
    azure_panel::AzurePanelWidget, drilldown::Drilldown, help_overlay::HelpOverlay,
    live_feed::LiveFeed, log_view::LogView, source_table::SourceTable,
};

pub fn draw(f: &mut Frame, app: &App, uptime: std::time::Duration, help_open: bool) {
    // Vertical stack:
    //   Header | Sources table | Live feed (pushes) | Azure panel | Footer
    // The live feed is the pane the user asked for explicitly — it makes
    // "is the thing working right now?" answerable at a glance.
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(10),   // sources or log
            Constraint::Length(8), // live feed
            Constraint::Length(8), // azure
            Constraint::Length(1), // footer
        ])
        .split(f.area());

    f.render_widget(Clear, f.area());
    draw_header(f, areas[0], app, uptime);

    if app.prefs.log_view_on {
        f.render_widget(
            LogView {
                lines: &app.log_tail,
                focused: false,
            },
            areas[1],
        );
    } else {
        draw_sources_area(f, areas[1], app);
    }

    f.render_widget(LiveFeed { app }, areas[2]);
    f.render_widget(AzurePanelWidget { app }, areas[3]);
    draw_footer(f, areas[4], app);

    if help_open {
        f.render_widget(HelpOverlay, f.area());
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App, uptime: std::time::Duration) {
    f.render_widget(
        Paragraph::new(header_line(app, chrono::Utc::now(), uptime)),
        area,
    );
}

fn draw_sources_area(f: &mut Frame, area: Rect, app: &App) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title("Sources");
    let inner = outer.inner(area);
    outer.render(area, f.buffer_mut());

    if app.sources.is_empty() {
        f.render_widget(Paragraph::new("No sources configured"), inner);
        return;
    }

    if app.drilldown_open && app.selected_subsource.is_some() {
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(inner);
        f.render_widget(SourceTable { app }, split[0]);
        f.render_widget(Drilldown { app }, split[1]);
    } else {
        f.render_widget(SourceTable { app }, inner);
    }
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let msg = if app.footer.is_empty() {
        " ↑↓ select  ·  ←/→ collapse  ·  enter details  ·  r sync now  ·  p pause  ·  s logs  ·  ? help  ·  q quit".to_string()
    } else {
        format!(" {}", app.footer)
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            msg,
            Style::default().fg(Color::Gray),
        ))),
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
        let mut term = ratatui::Terminal::new(TestBackend::new(100, 26)).unwrap();
        term.draw(|f| {
            draw(f, &app, std::time::Duration::from_secs(1), false);
        })
        .unwrap();
    }

    #[test]
    fn footer_shows_only_one_keybinding_line() {
        let app = App::new(&cfg(), Prefs::default());
        let mut term = ratatui::Terminal::new(TestBackend::new(100, 26)).unwrap();
        term.draw(|f| {
            draw(f, &app, std::time::Duration::from_secs(1), false);
        })
        .unwrap();
        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let occurrences = text.matches("sync now").count();
        assert_eq!(
            occurrences, 1,
            "expected 1 footer line, found {occurrences}: {text}"
        );
    }
}
