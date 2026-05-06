use serde_json::Value;

use crate::config::Config;
use crate::mcp::tools::search::IncludeContent;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum IncludeContentArg {
    Snippet,
    Full,
    AgenticAnswer,
}

impl From<IncludeContentArg> for IncludeContent {
    fn from(a: IncludeContentArg) -> Self {
        match a {
            IncludeContentArg::Snippet => IncludeContent::Snippet,
            IncludeContentArg::Full => IncludeContent::Full,
            IncludeContentArg::AgenticAnswer => IncludeContent::AgenticAnswer,
        }
    }
}

#[derive(Debug)]
pub struct SearchOptions {
    pub query: String,
    pub data_sources: Option<Vec<String>>,
    pub where_: Option<Value>,
    pub top: usize,
    pub cursor: Option<String>,
    pub include_content: IncludeContentArg,
    pub include_deleted: bool,
    pub json: bool,
}

pub async fn run(_config: &Config, _options: SearchOptions) -> anyhow::Result<()> {
    todo!("phase 7: rewire `quelch search` against the new instances schema")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::schema::SchemaCatalog;
    use crate::mcp::tools::search::{self, SearchRequest, SearchToolConfig};
    use crate::mcp::tools::search_api::mock::MockSearchApi;
    use crate::mcp::tools::test_helpers::build_expose;

    #[tokio::test]
    async fn search_dispatches_to_search_tool_with_mock() {
        let api = MockSearchApi::new();
        let expose = build_expose(&[("jira_issues", "jira_issue", "jira-issues")]);
        let schema = SchemaCatalog::default();
        let config = SearchToolConfig {
            disable_agentic: true,
            ..Default::default()
        };

        let req = SearchRequest {
            query: "open bugs".to_string(),
            data_sources: Some(vec!["jira_issues".to_string()]),
            r#where: None,
            top: 10,
            cursor: None,
            include_deleted: false,
            include_content: IncludeContent::Snippet,
        };

        let resp = search::run(&api, &expose, &schema, &config, req)
            .await
            .unwrap();

        assert_eq!(resp.total_estimate, 1);
    }

    #[test]
    fn include_content_arg_converts_correctly() {
        assert!(matches!(
            IncludeContent::from(IncludeContentArg::Snippet),
            IncludeContent::Snippet
        ));
        assert!(matches!(
            IncludeContent::from(IncludeContentArg::Full),
            IncludeContent::Full
        ));
        assert!(matches!(
            IncludeContent::from(IncludeContentArg::AgenticAnswer),
            IncludeContent::AgenticAnswer
        ));
    }
}
