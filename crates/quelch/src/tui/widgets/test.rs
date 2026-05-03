#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use crate::config::{
        AuthConfig, AzureConfig, Config, CosmosConfig, JiraSourceConfig, OpenAiConfig, SourceConfig,
    };
    use crate::tui::app::App;
    use crate::tui::prefs::Prefs;
    use crate::tui::widgets::source_table::SourceTable;

    fn cfg() -> Config {
        // TODO(quelch v2 phase 3+): move to a shared test fixture builder
        Config {
            azure: AzureConfig {
                subscription_id: "sub".into(),
                resource_group: "rg".into(),
                region: "swedencentral".into(),
                naming: Default::default(),
                skip_role_assignments: false,
            },
            cosmos: CosmosConfig::default(),
            search: Default::default(),
            openai: OpenAiConfig {
                endpoint: "https://x.openai.azure.com".into(),
                embedding_deployment: "te".into(),
                embedding_dimensions: 1536,
            },
            sources: vec![SourceConfig::Jira(JiraSourceConfig {
                name: "my-jira".into(),
                url: "x".into(),
                auth: AuthConfig::DataCenter { pat: "p".into() },
                projects: vec!["DO".into(), "HR".into()],
                container: None,
                companion_containers: Default::default(),
                fields: Default::default(),
            })],
            ingest: Default::default(),
            deployments: vec![],
            mcp: Default::default(),
            rigg: Default::default(),
            state: Default::default(),
        }
    }

    fn rendered_text(app: &App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            f.render_widget(SourceTable { app }, f.area());
        })
        .unwrap();
        let buf = term.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_column_headings() {
        let app = App::new(&cfg(), Prefs::default());
        let text = rendered_text(&app, 120, 10);
        // Destination-side columns. No "Items" / "Rate" / "Last item" / "Updated"
        // (those measured the wrong quantities in v0.6.0).
        assert!(text.contains("Source"), "missing Source heading:\n{text}");
        assert!(text.contains("Stage"), "missing Stage heading");
        assert!(text.contains("Pushed"), "missing Pushed heading");
        assert!(text.contains("Per min"), "missing Per min heading");
        assert!(text.contains("Latest ID"), "missing Latest ID heading");
        assert!(text.contains("Pushed at"), "missing Pushed at heading");
    }

    #[test]
    fn renders_source_row_and_expanded_subsources() {
        let app = App::new(&cfg(), Prefs::default());
        let text = rendered_text(&app, 100, 10);
        assert!(text.contains("my-jira"));
        assert!(text.contains("DO"));
        assert!(text.contains("HR"));
    }

    #[test]
    fn collapsed_source_hides_subsources() {
        let mut app = App::new(&cfg(), Prefs::default());
        app.prefs.toggle_source_collapsed("my-jira");
        let text = rendered_text(&app, 100, 10);
        assert!(text.contains("my-jira"));
        assert!(!text.contains("  DO"));
        assert!(!text.contains("  HR"));
    }

    use crate::tui::widgets::azure_panel::AzurePanelWidget;

    #[test]
    fn azure_panel_shows_destination_side_counters() {
        let app = App::new(&cfg(), Prefs::default());
        let mut term = Terminal::new(TestBackend::new(100, 12)).unwrap();
        term.draw(|f| {
            f.render_widget(AzurePanelWidget { app: &app }, f.area());
        })
        .unwrap();
        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| buf[(x, y)].symbol().to_string()))
            .collect::<Vec<_>>()
            .join("");
        // The user-facing promise: show destination-side quantities, not
        // HTTP-request internals or latency numbers.
        assert!(text.contains("Total pushed"), "rendered: {text}");
        assert!(text.contains("Per minute"));
        assert!(text.contains("4xx"));
        assert!(text.contains("5xx"));
        assert!(text.contains("Throttled"));
        // v0.6.0 had a useless "median XX ms" line — it should be gone.
        assert!(
            !text.contains("median"),
            "latency median resurfaced in panel: {text}"
        );
    }

    use crate::tui::widgets::drilldown::Drilldown;

    #[test]
    fn drilldown_shows_destination_side_pushes() {
        let mut app = App::new(&cfg(), Prefs::default());
        // Populate with confirmed-pushed events (what the drilldown shows).
        for i in 0..3 {
            app.apply(crate::tui::events::QuelchEvent::DocPushed {
                source: "my-jira".into(),
                subsource: "DO".into(),
                id: format!("DO-{i}"),
                updated: chrono::Utc::now(),
            });
        }
        app.move_selection_down(); // focus DO

        let mut term = Terminal::new(TestBackend::new(60, 20)).unwrap();
        term.draw(|f| {
            f.render_widget(Drilldown { app: &app }, f.area());
        })
        .unwrap();
        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| buf[(x, y)].symbol().to_string()))
            .collect::<Vec<_>>()
            .join("");
        // Must surface destination-side language ("Pushed to Azure")
        // rather than ambiguous "Docs synced / Recent".
        assert!(text.contains("Pushed to Azure"), "rendered: {text}");
        assert!(text.contains("Last pushed"), "rendered: {text}");
        assert!(text.contains("DO-2"));
    }

    use crate::tui::widgets::help_overlay::HelpOverlay;

    #[test]
    fn help_overlay_lists_key_bindings() {
        let mut term = Terminal::new(TestBackend::new(70, 30)).unwrap();
        term.draw(|f| {
            f.render_widget(HelpOverlay {}, f.area());
        })
        .unwrap();
        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| buf[(x, y)].symbol().to_string()))
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("Keyboard shortcuts"));
        assert!(text.contains("sync now"));
        assert!(text.contains("pause"));
        assert!(text.contains("quit"));
    }

    use crate::tui::app::LogLine;
    use crate::tui::widgets::log_view::LogView;
    use std::collections::VecDeque;

    #[test]
    fn log_view_renders_column_headings() {
        let mut lines = VecDeque::new();
        lines.push_back(LogLine {
            ts: chrono::Utc::now(),
            level: tracing::Level::INFO,
            target: "quelch::sync".into(),
            message: "Cycle starting".into(),
        });
        let view = LogView {
            lines: &lines,
            focused: false,
        };
        let mut term = Terminal::new(TestBackend::new(100, 10)).unwrap();
        term.draw(|f| f.render_widget(view, f.area())).unwrap();
        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| buf[(x, y)].symbol().to_string()))
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("LEVEL"));
        assert!(text.contains("TIME"));
        assert!(text.contains("TARGET"));
        assert!(text.contains("MESSAGE"));
        assert!(text.contains("Cycle starting"));
    }
}
