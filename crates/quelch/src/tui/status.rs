//! Centralised header-string builder. All TUI states map here.

use chrono::{DateTime, Utc};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::app::{App, EngineStatus};

/// Render the header as one `Line`, with colour coded by state.
pub fn header_line(app: &App, now: DateTime<Utc>, uptime: std::time::Duration) -> Line<'static> {
    let version = env!("CARGO_PKG_VERSION");
    let up = format_uptime(uptime);
    let state = state_span(app, now);
    Line::from(vec![
        Span::styled(
            format!(" quelch {version} "),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" · "),
        state,
        Span::raw("   "),
        Span::styled(format!("uptime {up}"), Style::default().fg(Color::DarkGray)),
    ])
}

fn state_span(app: &App, _now: DateTime<Utc>) -> Span<'static> {
    if app.backoff_reason.is_some() {
        let remaining = app
            .backoff_until
            .map(|u| u.signed_duration_since(Utc::now()).num_seconds().max(0))
            .unwrap_or(0);
        return Span::styled(
            format!("◉ Azure client backing off · {remaining}s remaining"),
            Style::default().fg(Color::Yellow),
        );
    }
    match &app.status {
        EngineStatus::Idle => Span::styled(
            "○ Ready · press r to sync now".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        EngineStatus::Syncing { cycle, .. } => Span::styled(
            format!("{spin} Syncing · cycle {cycle}", spin = app.spinner_glyph()),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        EngineStatus::Paused => Span::styled(
            "⏸ Paused · press p to resume".to_string(),
            Style::default().fg(Color::Yellow),
        ),
        EngineStatus::Shutdown => Span::styled(
            "⏹ Shutting down".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
    }
}

fn format_uptime(d: std::time::Duration) -> String {
    let s = d.as_secs();
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let ss = s % 60;
    format!("{h}:{m:02}:{ss:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AuthConfig, AzureConfig, Config, JiraSourceConfig, SourceConfig, SyncConfig,
    };
    use crate::tui::app::App;
    use crate::tui::prefs::Prefs;

    fn app() -> App {
        let cfg = Config {
            azure: AzureConfig {
                endpoint: "x".into(),
                api_key: "k".into(),
            },
            sources: vec![SourceConfig::Jira(JiraSourceConfig {
                name: "j".into(),
                url: "x".into(),
                auth: AuthConfig::DataCenter { pat: "p".into() },
                projects: vec!["DO".into()],
                index: "i".into(),
            })],
            sync: SyncConfig::default(),
        };
        App::new(&cfg, Prefs::default())
    }

    #[test]
    fn idle_header_mentions_ready() {
        let a = app();
        let line = header_line(&a, Utc::now(), std::time::Duration::from_secs(5));
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("Ready"));
        assert!(text.contains("uptime 0:00:05"));
    }

    #[test]
    fn paused_header_shows_pause_glyph() {
        let mut a = app();
        a.status = EngineStatus::Paused;
        let line = header_line(&a, Utc::now(), std::time::Duration::from_secs(0));
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("Paused"));
    }

    #[test]
    fn backoff_header_takes_precedence() {
        let mut a = app();
        a.backoff_reason = Some("HTTP 429".into());
        a.backoff_until = Some(Utc::now() + chrono::Duration::seconds(30));
        let line = header_line(&a, Utc::now(), std::time::Duration::from_secs(0));
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("backing off"));
    }
}
