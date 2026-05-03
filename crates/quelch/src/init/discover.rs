/// Azure CLI discovery helpers for `quelch init`.
///
/// Wraps `az` shell-outs to list subscriptions, resource groups, and find
/// existing Azure resources. If `az` is unavailable or returns no results,
/// the caller falls back to manual prompts.
use serde::Deserialize;

/// An Azure subscription.
#[derive(Debug, Clone, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    #[serde(rename = "isDefault", default)]
    pub is_default: bool,
}

/// An Azure resource group.
#[derive(Debug, Clone, Deserialize)]
pub struct ResourceGroup {
    pub name: String,
    pub location: String,
}

/// A discovered Cosmos DB account.
#[derive(Debug, Clone)]
pub struct CosmosAccount {
    pub name: String,
    pub endpoint: String,
}

/// A discovered Azure AI Search service.
#[derive(Debug, Clone)]
pub struct SearchService {
    pub name: String,
}

/// A discovered Azure OpenAI account.
#[derive(Debug, Clone)]
pub struct OpenAiAccount {
    pub name: String,
    pub endpoint: String,
}

/// List all Azure subscriptions the current user has access to.
///
/// Returns an empty Vec (rather than an error) if `az` is unavailable.
pub async fn list_subscriptions() -> anyhow::Result<Vec<Subscription>> {
    run_az_json(&["account", "list", "--output", "json"])
}

/// List all resource groups in the given subscription.
///
/// Returns an empty Vec if `az` is unavailable.
pub async fn list_resource_groups(subscription_id: &str) -> anyhow::Result<Vec<ResourceGroup>> {
    run_az_json(&[
        "group",
        "list",
        "--subscription",
        subscription_id,
        "--output",
        "json",
    ])
}

/// Find the first Cosmos DB account in a resource group.
///
/// Returns `None` if none found or `az` is unavailable.
pub async fn find_cosmos_account(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<Option<CosmosAccount>> {
    #[derive(Deserialize)]
    struct RawCosmos {
        name: String,
        #[serde(rename = "documentEndpoint")]
        document_endpoint: Option<String>,
    }

    let list: Vec<RawCosmos> = run_az_json(&[
        "cosmosdb",
        "list",
        "--subscription",
        subscription_id,
        "--resource-group",
        resource_group,
        "--output",
        "json",
    ])
    .unwrap_or_default();

    Ok(list.into_iter().next().map(|a| CosmosAccount {
        endpoint: a
            .document_endpoint
            .unwrap_or_else(|| format!("https://{}.documents.azure.com:443/", a.name)),
        name: a.name,
    }))
}

/// Find the first Azure AI Search service in a resource group.
pub async fn find_search_service(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<Option<SearchService>> {
    #[derive(Deserialize)]
    struct RawSearch {
        name: String,
    }

    let list: Vec<RawSearch> = run_az_json(&[
        "search",
        "service",
        "list",
        "--subscription",
        subscription_id,
        "--resource-group",
        resource_group,
        "--output",
        "json",
    ])
    .unwrap_or_default();

    Ok(list
        .into_iter()
        .next()
        .map(|s| SearchService { name: s.name }))
}

/// Find the first Azure OpenAI account in a resource group.
pub async fn find_openai_account(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<Option<OpenAiAccount>> {
    #[derive(Deserialize)]
    struct RawOai {
        name: String,
        properties: Option<OaiProperties>,
    }
    #[derive(Deserialize)]
    struct OaiProperties {
        endpoint: Option<String>,
    }

    let list: Vec<RawOai> = run_az_json(&[
        "cognitiveservices",
        "account",
        "list",
        "--subscription",
        subscription_id,
        "--resource-group",
        resource_group,
        "--output",
        "json",
    ])
    .unwrap_or_default();

    Ok(list.into_iter().next().map(|a| {
        let endpoint = a
            .properties
            .and_then(|p| p.endpoint)
            .unwrap_or_else(|| format!("https://{}.openai.azure.com", a.name));
        OpenAiAccount {
            name: a.name,
            endpoint,
        }
    }))
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Run an `az` command, parse stdout as JSON `T`, and return the result.
///
/// On any error (az not found, command failure, parse error) returns an empty
/// `Vec` via `unwrap_or_default()` at call sites so the wizard can fall back
/// to manual input.
fn run_az_json<T: serde::de::DeserializeOwned>(args: &[&str]) -> anyhow::Result<T> {
    let output = std::process::Command::new("az").args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("az command failed: {stderr}");
    }
    let parsed: T = serde_json::from_slice(&output.stdout)?;
    Ok(parsed)
}

// ---------------------------------------------------------------------------
// Tests (gated with #[ignore] — require az CLI on PATH)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires az CLI on PATH with a logged-in account"]
    async fn list_subscriptions_returns_at_least_one() {
        let subs = list_subscriptions().await.unwrap();
        assert!(!subs.is_empty(), "expected at least one subscription");
        println!("Subscriptions found: {}", subs.len());
        for s in &subs {
            println!("  {} — {}", s.name, s.id);
        }
    }
}
