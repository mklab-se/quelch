use ratatui::{Terminal, backend::TestBackend};

use crate::config::{AuthConfig, AzureConfig, Config, JiraSourceConfig, SourceConfig, SyncConfig};
use crate::tui::app::App;
use crate::tui::prefs::Prefs;
use crate::tui::widgets::source_card::SourceCard;

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
            projects: vec!["DO".into()],
            index: "i".into(),
        })],
        sync: SyncConfig::default(),
    }
}

#[test]
fn source_card_renders_to_test_backend() {
    let backend = TestBackend::new(60, 6);
    let mut term = Terminal::new(backend).unwrap();
    let app = App::new(&cfg(), Prefs::default());
    term.draw(|f| {
        f.render_widget(
            SourceCard {
                view: &app.sources[0],
                collapsed: false,
                focused: true,
                focused_subsource: Some("DO"),
            },
            f.area(),
        );
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
    assert!(text.contains("my-jira"), "rendered:\n{text}");
    assert!(text.contains("DO"));
}
