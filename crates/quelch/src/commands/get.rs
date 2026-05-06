use crate::config::Config;

#[derive(Debug)]
pub struct GetOptions {
    pub id: String,
    pub data_source: String,
    pub include_deleted: bool,
    pub json: bool,
}

pub async fn run(_config: &Config, _options: GetOptions) -> anyhow::Result<()> {
    todo!("phase 7: rewire `quelch get` against the new instances schema")
}

#[cfg(test)]
mod tests {
    use crate::cosmos::{CosmosBackend, InMemoryCosmos};
    use crate::mcp::tools::get::{GetRequest, run as get_run};
    use crate::mcp::tools::test_helpers::build_expose_jira_issues;
    use serde_json::json;

    #[tokio::test]
    async fn get_dispatches_to_get_tool() {
        let cosmos = InMemoryCosmos::new();
        CosmosBackend::upsert(
            &cosmos,
            "jira-issues",
            json!({
                "id": "DO-1",
                "_partition_key": "DO",
                "status": "Open",
                "title": "Test issue",
            }),
        )
        .await
        .unwrap();

        let expose = build_expose_jira_issues();

        let req = GetRequest {
            data_source: "jira_issues".into(),
            id: "DO-1".into(),
            include_deleted: false,
        };

        let resp = get_run(&cosmos, &expose, req).await.unwrap();
        assert!(resp.document.is_some(), "document should be found");
        assert_eq!(resp.document.unwrap()["id"], "DO-1");
    }

    #[tokio::test]
    async fn get_returns_none_for_missing_document() {
        let cosmos = InMemoryCosmos::new();
        let expose = build_expose_jira_issues();

        let req = GetRequest {
            data_source: "jira_issues".into(),
            id: "NOPE-999".into(),
            include_deleted: false,
        };

        let resp = get_run(&cosmos, &expose, req).await.unwrap();
        assert!(
            resp.document.is_none(),
            "missing document should return None"
        );
    }
}
