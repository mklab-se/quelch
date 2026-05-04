/// Azure CLI discovery helpers for `quelch init` and `quelch validate`.
///
/// Wraps `az` shell-outs to list Azure resources Quelch references but does
/// not provision (Cosmos DB, AI Search, Foundry / Azure OpenAI, Container Apps
/// environment, Application Insights, Key Vault).  All functions list **all**
/// resources of a kind in the resource group rather than just the first match,
/// so the wizard can present a Select to the user.
///
/// On any `az` failure (not on PATH, not signed in, transient error) callers
/// receive an empty `Vec` and fall back to manual input. The empty-list signal
/// is itself surfaced — the wizard prints "no <kind> found" with a remediation
/// hint.
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Subscriptions and resource groups
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Discovered resource records
// ---------------------------------------------------------------------------

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

/// A discovered Microsoft Foundry project (kind=Project under
/// `Microsoft.MachineLearningServices/workspaces`).
#[derive(Debug, Clone)]
pub struct FoundryProject {
    pub name: String,
    pub endpoint: String,
}

/// A discovered model deployment inside an Azure OpenAI account or Foundry
/// project. The `kind` field is informational ("OpenAI", "MaaS", etc.).
#[derive(Debug, Clone)]
pub struct ModelDeployment {
    pub name: String,
    pub model_name: String,
    pub kind: String,
}

/// A discovered Container Apps environment.
#[derive(Debug, Clone)]
pub struct ContainerAppsEnvironment {
    pub name: String,
}

/// A discovered Application Insights component.
#[derive(Debug, Clone)]
pub struct AppInsights {
    pub name: String,
    pub connection_string: String,
}

/// A discovered Key Vault.
#[derive(Debug, Clone)]
pub struct KeyVault {
    pub name: String,
    pub vault_uri: String,
}

// ---------------------------------------------------------------------------
// Subscriptions / resource groups
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Cosmos DB / AI Search
// ---------------------------------------------------------------------------

/// List all Cosmos DB accounts in a resource group.
pub async fn list_cosmos_accounts(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<Vec<CosmosAccount>> {
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

    Ok(list
        .into_iter()
        .map(|a| CosmosAccount {
            endpoint: a
                .document_endpoint
                .unwrap_or_else(|| format!("https://{}.documents.azure.com:443/", a.name)),
            name: a.name,
        })
        .collect())
}

/// Find the first Cosmos DB account in a resource group (back-compat shim
/// for the existing wizard step). Prefer `list_cosmos_accounts` for new code.
pub async fn find_cosmos_account(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<Option<CosmosAccount>> {
    Ok(list_cosmos_accounts(subscription_id, resource_group)
        .await?
        .into_iter()
        .next())
}

/// List all Azure AI Search services in a resource group.
pub async fn list_search_services(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<Vec<SearchService>> {
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
        .map(|s| SearchService { name: s.name })
        .collect())
}

/// Find the first AI Search service in a resource group.
pub async fn find_search_service(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<Option<SearchService>> {
    Ok(list_search_services(subscription_id, resource_group)
        .await?
        .into_iter()
        .next())
}

// ---------------------------------------------------------------------------
// Azure OpenAI accounts and Foundry projects
// ---------------------------------------------------------------------------

/// List all Azure OpenAI accounts (Cognitive Services accounts with
/// `kind=OpenAI`) in a resource group.
pub async fn list_openai_accounts(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<Vec<OpenAiAccount>> {
    #[derive(Deserialize)]
    struct RawOai {
        name: String,
        kind: Option<String>,
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

    Ok(list
        .into_iter()
        .filter(|a| a.kind.as_deref() == Some("OpenAI"))
        .map(|a| {
            let endpoint = a
                .properties
                .and_then(|p| p.endpoint)
                .unwrap_or_else(|| format!("https://{}.openai.azure.com", a.name));
            OpenAiAccount {
                name: a.name,
                endpoint,
            }
        })
        .collect())
}

/// Find the first OpenAI account in a resource group.
pub async fn find_openai_account(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<Option<OpenAiAccount>> {
    Ok(list_openai_accounts(subscription_id, resource_group)
        .await?
        .into_iter()
        .next())
}

/// List all Microsoft Foundry projects in a resource group.
///
/// Foundry projects are surfaced as Cognitive Services accounts with
/// `kind=AIServices` (the multi-service Foundry account) — the SDK and
/// portal aliases for "Foundry project" cover both AIServices accounts and
/// per-project sub-resources. We list AIServices accounts here; the user
/// then picks one as their model provider.
pub async fn list_foundry_projects(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<Vec<FoundryProject>> {
    #[derive(Deserialize)]
    struct RawCog {
        name: String,
        kind: Option<String>,
        properties: Option<CogProperties>,
    }
    #[derive(Deserialize)]
    struct CogProperties {
        endpoint: Option<String>,
    }

    let list: Vec<RawCog> = run_az_json(&[
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

    Ok(list
        .into_iter()
        .filter(|a| a.kind.as_deref() == Some("AIServices"))
        .map(|a| {
            let endpoint = a
                .properties
                .and_then(|p| p.endpoint)
                .unwrap_or_else(|| format!("https://{}.cognitiveservices.azure.com", a.name));
            FoundryProject {
                name: a.name,
                endpoint,
            }
        })
        .collect())
}

/// List model deployments inside a Cognitive Services account (works for both
/// OpenAI accounts and Foundry projects — both expose deployments through the
/// same `cognitiveservices account deployment` command).
pub async fn list_model_deployments(
    subscription_id: &str,
    resource_group: &str,
    account_name: &str,
) -> anyhow::Result<Vec<ModelDeployment>> {
    #[derive(Deserialize)]
    struct RawDep {
        name: String,
        properties: Option<DepProperties>,
    }
    #[derive(Deserialize)]
    struct DepProperties {
        model: Option<DepModel>,
    }
    #[derive(Deserialize)]
    struct DepModel {
        name: Option<String>,
        format: Option<String>,
    }

    let list: Vec<RawDep> = run_az_json(&[
        "cognitiveservices",
        "account",
        "deployment",
        "list",
        "--subscription",
        subscription_id,
        "--resource-group",
        resource_group,
        "--name",
        account_name,
        "--output",
        "json",
    ])
    .unwrap_or_default();

    Ok(list
        .into_iter()
        .map(|d| {
            let (model_name, kind) = d
                .properties
                .and_then(|p| p.model)
                .map(|m| {
                    (
                        m.name.unwrap_or_default(),
                        m.format.unwrap_or_else(|| "OpenAI".to_string()),
                    )
                })
                .unwrap_or_else(|| (String::new(), "OpenAI".to_string()));
            ModelDeployment {
                name: d.name,
                model_name,
                kind,
            }
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Container Apps environment
// ---------------------------------------------------------------------------

/// List all Container Apps environments in a resource group.
pub async fn list_container_apps_environments(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<Vec<ContainerAppsEnvironment>> {
    #[derive(Deserialize)]
    struct RawEnv {
        name: String,
    }

    let list: Vec<RawEnv> = run_az_json(&[
        "containerapp",
        "env",
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
        .map(|e| ContainerAppsEnvironment { name: e.name })
        .collect())
}

// ---------------------------------------------------------------------------
// Application Insights
// ---------------------------------------------------------------------------

/// List all Application Insights components in a resource group.
pub async fn list_application_insights(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<Vec<AppInsights>> {
    #[derive(Deserialize)]
    struct RawAi {
        name: String,
        #[serde(rename = "connectionString", default)]
        connection_string: Option<String>,
    }

    let list: Vec<RawAi> = run_az_json(&[
        "monitor",
        "app-insights",
        "component",
        "show",
        "--subscription",
        subscription_id,
        "--resource-group",
        resource_group,
        "--output",
        "json",
    ])
    .unwrap_or_default();

    // `component show` may return either a single object or an array
    // depending on whether `--app` was passed. We handle the array case here.
    Ok(list
        .into_iter()
        .map(|a| AppInsights {
            connection_string: a.connection_string.unwrap_or_default(),
            name: a.name,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Key Vault
// ---------------------------------------------------------------------------

/// List all Key Vaults in a resource group.
pub async fn list_key_vaults(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<Vec<KeyVault>> {
    #[derive(Deserialize)]
    struct RawKv {
        name: String,
        properties: Option<KvProperties>,
    }
    #[derive(Deserialize)]
    struct KvProperties {
        #[serde(rename = "vaultUri", default)]
        vault_uri: Option<String>,
    }

    let list: Vec<RawKv> = run_az_json(&[
        "keyvault",
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
        .map(|k| KeyVault {
            vault_uri: k
                .properties
                .and_then(|p| p.vault_uri)
                .unwrap_or_else(|| format!("https://{}.vault.azure.net/", k.name)),
            name: k.name,
        })
        .collect())
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
        for s in &subs {
            println!("  {} — {}", s.name, s.id);
        }
    }
}
