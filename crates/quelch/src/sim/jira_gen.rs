//! Jira-specific mutation calls: create/update issues, add comments.

use anyhow::Result;
use rand::Rng;
use rand::rngs::StdRng;

pub async fn mutate(client: &reqwest::Client, base: &str, rng: &mut StdRng) -> Result<()> {
    let project = pick_project(rng);
    // 70% update / 30% create
    if rng.r#gen::<f32>() < 0.3 {
        create_issue(client, base, project, rng).await
    } else {
        update_random_issue(client, base, project, rng).await
    }
}

fn pick_project(rng: &mut StdRng) -> &'static str {
    if rng.r#gen::<f32>() < 0.7 {
        "QUELCH"
    } else {
        "DEMO"
    }
}

async fn create_issue(
    client: &reqwest::Client,
    base: &str,
    project: &str,
    rng: &mut StdRng,
) -> Result<()> {
    let n: u32 = rng.gen_range(1000..99999);
    let key = format!("{project}-{n}");
    let body = serde_json::json!({
        "project": project,
        "key": key,
        "summary": summary_text(rng),
        "description": "Created by sim.",
    });
    client
        .post(format!("{base}/_sim/jira/upsert_issue"))
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn update_random_issue(
    client: &reqwest::Client,
    base: &str,
    project: &str,
    rng: &mut StdRng,
) -> Result<()> {
    let search = client
        .get(format!("{base}/jira/rest/api/2/search"))
        .header("authorization", format!("Bearer {}", crate::sim::MOCK_PAT))
        .query(&[
            ("jql", format!("project = {project}").as_str()),
            ("maxResults", "100"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    let issues = search
        .get("issues")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if issues.is_empty() {
        return create_issue(client, base, project, rng).await;
    }
    let idx = rng.gen_range(0..issues.len());
    let chosen = &issues[idx];
    let key = chosen
        .get("key")
        .and_then(|k| k.as_str())
        .unwrap_or("UNKNOWN")
        .to_string();
    let summary = chosen
        .get("fields")
        .and_then(|f| f.get("summary"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let body = serde_json::json!({
        "project": project,
        "key": key,
        "summary": format!("{summary} (updated)"),
        "description": "Updated by sim.",
    });
    client
        .post(format!("{base}/_sim/jira/upsert_issue"))
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

fn summary_text(rng: &mut StdRng) -> String {
    const WORDS: &[&str] = &[
        "fix",
        "refactor",
        "implement",
        "investigate",
        "document",
        "optimise",
        "cleanup",
        "polish",
        "design",
        "review",
        "audit",
    ];
    const NOUNS: &[&str] = &[
        "sync loop",
        "mock server",
        "TUI header",
        "Azure retry",
        "config parser",
        "embedding cache",
        "state file",
        "cursor",
    ];
    let verb = WORDS[rng.gen_range(0..WORDS.len())];
    let noun = NOUNS[rng.gen_range(0..NOUNS.len())];
    format!("{verb} {noun}")
}
