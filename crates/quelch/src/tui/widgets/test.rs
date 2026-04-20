#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use crate::config::{
        AuthConfig, AzureConfig, Config, JiraSourceConfig, SourceConfig, SyncConfig,
    };
    use crate::tui::app::App;
    use crate::tui::prefs::Prefs;
    use crate::tui::widgets::source_table::SourceTable;

    fn cfg() -> Config {
        Config {
            azure: AzureConfig {
                endpoint: "x".into(),
                api_key: "k".into(),
            },
            sources: vec![SourceConfig::Jira(JiraSourceConfig {
                name: "my-jira".into(),
                url: "x".into(),
                auth: AuthConfig::DataCenter { pat: "p".into() },
                projects: vec!["DO".into(), "HR".into()],
                index: "i".into(),
            })],
            sync: SyncConfig::default(),
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
        let text = rendered_text(&app, 100, 10);
        assert!(text.contains("Source"), "missing Source heading:\n{text}");
        assert!(text.contains("Status"), "missing Status heading");
        assert!(text.contains("Items"), "missing Items heading");
        assert!(text.contains("Rate"), "missing Rate heading");
        assert!(text.contains("Last item"), "missing Last item heading");
        assert!(text.contains("Updated"), "missing Updated heading");
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
}
