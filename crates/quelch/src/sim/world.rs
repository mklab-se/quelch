//! Builds the sim's starter corpus by posting to the mock's /_sim/* endpoints.
//! Expects a mock server already running at `base_url`.

use anyhow::{Context, Result};
use rand::{SeedableRng, rngs::StdRng};

pub async fn seed(base_url: &str, seed: Option<u64>) -> Result<()> {
    let mut rng = match seed {
        Some(s) => StdRng::seed_from_u64(s),
        None => StdRng::from_entropy(),
    };
    let client = reqwest::Client::new();

    seed_jira_project(&client, base_url, "QUELCH", 40, &mut rng).await?;
    seed_jira_project(&client, base_url, "DEMO", 15, &mut rng).await?;
    seed_confluence_space(&client, base_url, "QUELCH", 20, &mut rng).await?;
    seed_confluence_space(&client, base_url, "INFRA", 8, &mut rng).await?;
    Ok(())
}

async fn seed_jira_project(
    client: &reqwest::Client,
    base: &str,
    project: &str,
    target_count: usize,
    _rng: &mut StdRng,
) -> Result<()> {
    for i in 0..target_count {
        let key = format!("{project}-SIM-{i}");
        let summary = format!("[{project}] generated issue {i}");
        let description = "Auto-generated entry for simulator starter corpus.".to_string();
        let body = serde_json::json!({
            "project": project,
            "key": key,
            "summary": summary,
            "description": description,
        });
        client
            .post(format!("{base}/_sim/jira/upsert_issue"))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("seed jira {key}"))?
            .error_for_status()?;
    }
    Ok(())
}

async fn seed_confluence_space(
    client: &reqwest::Client,
    base: &str,
    space: &str,
    target_count: usize,
    _rng: &mut StdRng,
) -> Result<()> {
    for i in 0..target_count {
        let id = format!("{space}-SIM-{i}");
        let title = format!("[{space}] generated page {i}");
        let body = format!("<h1>{title}</h1><p>Auto-generated for simulator.</p>");
        client
            .post(format!("{base}/_sim/confluence/upsert_page"))
            .json(&serde_json::json!({
                "space": space,
                "id": id,
                "title": title,
                "body": body,
            }))
            .send()
            .await
            .with_context(|| format!("seed confluence {id}"))?
            .error_for_status()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    async fn spawn_mock() -> String {
        let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, crate::mock::build_router())
                .await
                .unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn seeds_expected_counts() {
        let base = spawn_mock().await;
        seed(&base, Some(42)).await.unwrap();

        let client = reqwest::Client::new();
        let r = client
            .get(format!("{base}/jira/rest/api/2/search"))
            .header("authorization", format!("Bearer {}", crate::sim::MOCK_PAT))
            .query(&[("jql", "project = QUELCH"), ("maxResults", "500")])
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = r.json().await.unwrap();
        let issues = body.get("issues").unwrap().as_array().unwrap();
        // 17 built-in + 40 seeded = 57. Allow range to tolerate data.rs changes.
        assert!(issues.len() >= 57, "QUELCH issues: {}", issues.len());
    }
}
