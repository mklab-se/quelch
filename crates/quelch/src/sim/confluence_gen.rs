//! Confluence-specific mutation calls.

use anyhow::Result;
use rand::Rng;
use rand::rngs::StdRng;

pub async fn mutate(client: &reqwest::Client, base: &str, rng: &mut StdRng) -> Result<()> {
    let space = if rng.r#gen::<f32>() < 0.7 {
        "QUELCH"
    } else {
        "INFRA"
    };
    // 85% update / 15% create
    if rng.r#gen::<f32>() < 0.15 {
        create_page(client, base, space, rng).await
    } else {
        update_random_page(client, base, space, rng).await
    }
}

async fn create_page(
    client: &reqwest::Client,
    base: &str,
    space: &str,
    rng: &mut StdRng,
) -> Result<()> {
    let id: u32 = rng.gen_range(1_000_000..9_999_999);
    let title = format!("New page {id}");
    let body = format!("<h1>{title}</h1><p>Created by sim.</p>");
    client
        .post(format!("{base}/_sim/confluence/upsert_page"))
        .json(&serde_json::json!({
            "space": space,
            "id": id.to_string(),
            "title": title,
            "body": body,
        }))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn update_random_page(
    client: &reqwest::Client,
    base: &str,
    space: &str,
    rng: &mut StdRng,
) -> Result<()> {
    let search = client
        .get(format!("{base}/confluence/rest/api/content/search"))
        .header("authorization", "Bearer mock-pat-token")
        .query(&[
            ("cql", format!("space = {space}").as_str()),
            ("limit", "100"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    let pages = search
        .get("results")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if pages.is_empty() {
        return create_page(client, base, space, rng).await;
    }
    let idx = rng.gen_range(0..pages.len());
    let chosen = &pages[idx];
    let id = chosen
        .get("id")
        .and_then(|k| k.as_str())
        .unwrap_or("")
        .to_string();
    let title = chosen
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("untitled")
        .to_string();
    client
        .post(format!("{base}/_sim/confluence/upsert_page"))
        .json(&serde_json::json!({
            "space": space,
            "id": id,
            "title": format!("{title} (v+1)"),
            "body": "<p>Updated by sim.</p>",
        }))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}
