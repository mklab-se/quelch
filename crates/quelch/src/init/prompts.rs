//! Interactive prompt sections for `quelch init`.
//!
//! Each `prompt_*` function drives one section of the wizard and returns a
//! piece of the final [`Config`]. The wizard is loosely tree-structured:
//! Azure → source connections → instances. Helpers live alongside the section
//! that needs them.
//!
//! ## Credentials
//!
//! Wizard-collected secrets (PATs, API tokens) are NEVER written to disk.
//! The wizard always stores `${ENV_VAR_NAME}` placeholders in the generated
//! YAML; [`crate::config::env::substitute_env_vars`] resolves them at load
//! time. See [`prompt_credential_env_var`].

use std::collections::BTreeSet;
use std::time::Duration;

use crate::config::schema::{
    AiChat, AiConfig, AiEmbedding, AiProvider, AzureConfig, ContainerLayout, CosmosConfig,
    IngestInstance, InstanceConfig, InstanceSpec, McpInstance, SearchConfig, SourceAuth,
    SourceConnection, SourceType,
};

use super::discover;

// ---------------------------------------------------------------------------
// Azure section
// ---------------------------------------------------------------------------

/// Prompt for Azure subscription, the resource group containing Quelch's
/// dependencies, the Cosmos DB account, the AI Search service, and the AI
/// provider (Foundry or Azure OpenAI) with its embedding + chat deployments.
///
/// Returns a fully-populated [`AzureConfig`].
pub async fn prompt_azure() -> anyhow::Result<AzureConfig> {
    println!("\n=== Azure resources Quelch depends on ===");
    println!(
        "Quelch references three pre-existing Azure resources: a Cosmos DB account\n\
         (the system of record), an AI Search service (the index), and an AI\n\
         model provider (Foundry project or Azure OpenAI account) with one\n\
         embedding and one chat deployment. Quelch does NOT provision any of\n\
         these — `quelch azure apply` only configures their internals.\n"
    );
    println!("Discovering Azure subscriptions...");

    let subs = discover::list_subscriptions().await.unwrap_or_default();

    let subscription_id = if !subs.is_empty() {
        let names: Vec<String> = subs
            .iter()
            .map(|s| {
                if s.is_default {
                    format!("{} ({}) [default]", s.name, s.id)
                } else {
                    format!("{} ({})", s.name, s.id)
                }
            })
            .collect();
        let default_idx = subs.iter().position(|s| s.is_default).unwrap_or(0);
        let chosen = inquire::Select::new("Subscription:", names)
            .with_starting_cursor(default_idx)
            .raw_prompt()?
            .index;
        subs[chosen].id.clone()
    } else {
        println!("  (az not available or no subscriptions found — enter manually)");
        inquire::Text::new("Subscription ID:").prompt()?
    };

    let resource_group = pick_resource_group(
        &subscription_id,
        "Resource group containing Cosmos / AI Search / the AI provider:",
    )
    .await?;

    let cosmos = prompt_cosmos(&subscription_id, &resource_group).await?;
    let search = prompt_search(&subscription_id, &resource_group).await?;
    let ai = prompt_ai(&subscription_id, &resource_group).await?;

    Ok(AzureConfig {
        cosmos,
        search: Some(search),
        ai: Some(ai),
    })
}

/// Show a Select listing every resource group in the subscription, plus an
/// "Enter name manually…" escape hatch. Returns the chosen RG name.
async fn pick_resource_group(subscription_id: &str, prompt_text: &str) -> anyhow::Result<String> {
    let groups = discover::list_resource_groups(subscription_id)
        .await
        .unwrap_or_default();

    if groups.is_empty() {
        println!("  (az returned no resource groups — enter manually)");
        return Ok(inquire::Text::new(prompt_text).prompt()?);
    }

    const ENTER_MANUALLY: &str = "Enter name manually…";
    let mut labels: Vec<String> = groups
        .iter()
        .map(|g| format!("{}  ({})", g.name, g.location))
        .collect();
    labels.push(ENTER_MANUALLY.to_string());

    let idx = inquire::Select::new(prompt_text, labels)
        .with_starting_cursor(0)
        .raw_prompt()?
        .index;

    if idx < groups.len() {
        Ok(groups[idx].name.clone())
    } else {
        Ok(inquire::Text::new("Resource group name:").prompt()?)
    }
}

/// Prompt for the Cosmos DB account inside the chosen resource group.
async fn prompt_cosmos(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<CosmosConfig> {
    println!("\n--- Cosmos DB account ---");
    let accounts = discover::list_cosmos_accounts(subscription_id, resource_group)
        .await
        .unwrap_or_default();

    let (account_name, endpoint) = if accounts.is_empty() {
        println!("  (no Cosmos accounts found in this RG — enter manually)");
        let name = inquire::Text::new("Cosmos DB account name:").prompt()?;
        let default_endpoint = format!("https://{name}.documents.azure.com");
        let endpoint = inquire::Text::new("Cosmos DB endpoint:")
            .with_initial_value(&default_endpoint)
            .prompt()?;
        (name, endpoint)
    } else {
        const ENTER_MANUALLY: &str = "Enter manually…";
        let mut labels: Vec<String> = accounts
            .iter()
            .map(|a| format!("{}  ({})", a.name, a.endpoint))
            .collect();
        labels.push(ENTER_MANUALLY.to_string());
        let idx = inquire::Select::new("Cosmos DB account:", labels)
            .with_starting_cursor(0)
            .raw_prompt()?
            .index;
        if idx < accounts.len() {
            (accounts[idx].name.clone(), accounts[idx].endpoint.clone())
        } else {
            let name = inquire::Text::new("Cosmos DB account name:").prompt()?;
            let default_endpoint = format!("https://{name}.documents.azure.com");
            let endpoint = inquire::Text::new("Cosmos DB endpoint:")
                .with_initial_value(&default_endpoint)
                .prompt()?;
            (name, endpoint)
        }
    };

    let database = inquire::Text::new("Cosmos database name:")
        .with_initial_value("quelch")
        .prompt()?;

    Ok(CosmosConfig {
        subscription_id: Some(subscription_id.to_string()),
        resource_group: Some(resource_group.to_string()),
        account: Some(account_name),
        endpoint,
        database,
        containers: ContainerLayout::default(),
        meta_container: "quelch-meta".to_string(),
    })
}

/// Prompt for the AI Search service endpoint.
async fn prompt_search(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<SearchConfig> {
    println!("\n--- Azure AI Search ---");
    let services = discover::list_search_services(subscription_id, resource_group)
        .await
        .unwrap_or_default();

    let endpoint = if services.is_empty() {
        println!("  (no AI Search services found in this RG — enter manually)");
        let placeholder = "https://YOUR-SEARCH.search.windows.net";
        inquire::Text::new("AI Search service endpoint:")
            .with_initial_value(placeholder)
            .prompt()?
    } else {
        const ENTER_MANUALLY: &str = "Enter endpoint manually…";
        let mut labels: Vec<String> = services
            .iter()
            .map(|s| format!("{}.search.windows.net", s.name))
            .collect();
        labels.push(ENTER_MANUALLY.to_string());
        let idx = inquire::Select::new("AI Search service:", labels)
            .with_starting_cursor(0)
            .raw_prompt()?
            .index;
        if idx < services.len() {
            format!("https://{}.search.windows.net", services[idx].name)
        } else {
            let placeholder = "https://YOUR-SEARCH.search.windows.net";
            inquire::Text::new("AI Search service endpoint:")
                .with_initial_value(placeholder)
                .prompt()?
        }
    };

    Ok(SearchConfig { endpoint })
}

/// Prompt for the AI provider (Foundry or Azure OpenAI), endpoint, embedding
/// deployment, and chat deployment.
async fn prompt_ai(subscription_id: &str, resource_group: &str) -> anyhow::Result<AiConfig> {
    println!("\n--- AI model provider ---");
    println!(
        "Quelch wires up two model deployments in your AI Search Knowledge Base:\n  \
         - an embedding model (used by the vectorizer / skillset)\n  \
         - a chat / LLM (used for query planning + answer synthesis)\n\
         Both can live in the same Azure OpenAI account or Foundry project."
    );

    let providers = vec!["Microsoft Foundry  (recommended)", "Azure OpenAI"];
    let provider_idx = inquire::Select::new("Where do your model deployments live?", providers)
        .with_starting_cursor(0)
        .raw_prompt()?
        .index;
    let provider = if provider_idx == 0 {
        AiProvider::Foundry
    } else {
        AiProvider::AzureOpenai
    };

    let (endpoint, account_name) =
        pick_ai_account(subscription_id, resource_group, provider).await?;

    let deployments = match account_name {
        Some(ref name) => discover::list_model_deployments(subscription_id, resource_group, name)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };

    let embedding = pick_embedding_deployment(&deployments)?;
    let chat = pick_chat_deployment(&deployments)?;

    Ok(AiConfig {
        provider,
        endpoint,
        embedding,
        chat,
    })
}

/// Pick a Foundry project or Azure OpenAI account in the chosen RG, falling
/// back to manual endpoint entry. Returns `(endpoint, account_name)`.
async fn pick_ai_account(
    subscription_id: &str,
    resource_group: &str,
    provider: AiProvider,
) -> anyhow::Result<(String, Option<String>)> {
    let kind = match provider {
        AiProvider::Foundry => "Foundry project",
        AiProvider::AzureOpenai => "Azure OpenAI account",
    };

    let candidates: Vec<(String, String)> = match provider {
        AiProvider::Foundry => discover::list_foundry_projects(subscription_id, resource_group)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.name, p.endpoint))
            .collect(),
        AiProvider::AzureOpenai => discover::list_openai_accounts(subscription_id, resource_group)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|a| (a.name, a.endpoint))
            .collect(),
    };

    if candidates.is_empty() {
        println!("  No {kind}s found in resource group '{resource_group}' — enter manually.");
        let placeholder = match provider {
            AiProvider::Foundry => "https://YOUR-FOUNDRY.cognitiveservices.azure.com",
            AiProvider::AzureOpenai => "https://YOUR-OPENAI.openai.azure.com",
        };
        let endpoint = inquire::Text::new(&format!("{kind} endpoint:"))
            .with_initial_value(placeholder)
            .prompt()?;
        return Ok((endpoint, None));
    }

    const ENTER_MANUALLY: &str = "Enter endpoint manually…";
    let mut labels: Vec<String> = candidates
        .iter()
        .map(|(n, e)| format!("{n} — {e}"))
        .collect();
    labels.push(ENTER_MANUALLY.to_string());

    let idx = inquire::Select::new(&format!("{kind}:"), labels)
        .with_starting_cursor(0)
        .raw_prompt()?
        .index;

    if idx < candidates.len() {
        let (name, endpoint) = candidates[idx].clone();
        Ok((endpoint, Some(name)))
    } else {
        let placeholder = match provider {
            AiProvider::Foundry => "https://YOUR-FOUNDRY.cognitiveservices.azure.com",
            AiProvider::AzureOpenai => "https://YOUR-OPENAI.openai.azure.com",
        };
        let endpoint = inquire::Text::new(&format!("{kind} endpoint:"))
            .with_initial_value(placeholder)
            .prompt()?;
        Ok((endpoint, None))
    }
}

fn pick_embedding_deployment(
    available: &[discover::ModelDeployment],
) -> anyhow::Result<AiEmbedding> {
    let candidates: Vec<&discover::ModelDeployment> = available
        .iter()
        .filter(|d| d.model_name.starts_with("text-embedding"))
        .collect();

    let deployment = if candidates.is_empty() {
        if !available.is_empty() {
            println!("  (no embedding deployments detected — enter manually)");
        }
        inquire::Text::new("Embedding deployment name:")
            .with_initial_value("text-embedding-3-large")
            .prompt()?
    } else {
        let labels: Vec<String> = candidates
            .iter()
            .map(|d| format!("{} ({})", d.name, d.model_name))
            .collect();
        let idx = inquire::Select::new("Embedding deployment:", labels)
            .with_starting_cursor(0)
            .raw_prompt()?
            .index;
        candidates[idx].name.clone()
    };

    let dims_str: String = inquire::Text::new("Embedding dimensions:")
        .with_initial_value("3072")
        .prompt()?;
    let dimensions: u32 = dims_str
        .parse()
        .map_err(|_| anyhow::anyhow!("embedding dimensions must be a positive integer"))?;

    Ok(AiEmbedding {
        deployment,
        dimensions,
    })
}

/// Chat models supported by AI Search agentic retrieval (per Azure AI Search
/// 2025-11-01-preview). Used to filter the deployment Select.
const SUPPORTED_CHAT_MODELS: &[&str] = &[
    "gpt-4o",
    "gpt-4o-mini",
    "gpt-4.1",
    "gpt-4.1-nano",
    "gpt-4.1-mini",
    "gpt-5",
    "gpt-5-nano",
    "gpt-5-mini",
];

fn pick_chat_deployment(available: &[discover::ModelDeployment]) -> anyhow::Result<AiChat> {
    let candidates: Vec<&discover::ModelDeployment> = available
        .iter()
        .filter(|d| SUPPORTED_CHAT_MODELS.iter().any(|m| d.model_name == *m))
        .collect();

    let (deployment, model_name) = if candidates.is_empty() {
        if !available.is_empty() {
            println!(
                "  (no supported chat deployments detected; supported models: {})",
                SUPPORTED_CHAT_MODELS.join(", ")
            );
        }
        let dep: String = inquire::Text::new("Chat deployment name:")
            .with_initial_value("gpt-5-mini")
            .prompt()?;
        let model: String = inquire::Text::new("Chat model name:")
            .with_initial_value(&dep)
            .prompt()?;
        (dep, model)
    } else {
        let labels: Vec<String> = candidates
            .iter()
            .map(|d| format!("{} ({})", d.name, d.model_name))
            .collect();
        let idx = inquire::Select::new("Chat (LLM) deployment:", labels)
            .with_starting_cursor(0)
            .raw_prompt()?
            .index;
        let c = candidates[idx];
        (c.name.clone(), c.model_name.clone())
    };

    Ok(AiChat {
        deployment,
        model_name,
    })
}

// ---------------------------------------------------------------------------
// Source connections section
// ---------------------------------------------------------------------------

/// Prompt to add one or more source connections (Jira / Confluence). Loops
/// until the user picks "Done".
pub async fn prompt_source_connections() -> anyhow::Result<Vec<SourceConnection>> {
    println!("\n=== Source connections ===");
    println!(
        "A source connection is one (base URL × credential) tuple. Each instance\n\
         (q-ingest, q-mcp) references connections by name. You can add as many as\n\
         you like — typical setups have one Jira connection per PAT, plus a\n\
         Confluence connection.\n"
    );

    let mut connections: Vec<SourceConnection> = Vec::new();
    let mut seen_names: BTreeSet<String> = BTreeSet::new();

    loop {
        let prompt = if connections.is_empty() {
            "Add a source connection?"
        } else {
            "Add another source connection?"
        };
        let idx = inquire::Select::new(
            prompt,
            vec!["Jira", "Confluence", "Done (no more connections)"],
        )
        .with_starting_cursor(if connections.is_empty() { 0 } else { 2 })
        .raw_prompt()?
        .index;

        match idx {
            0 => connections.push(prompt_jira_connection(&seen_names)?),
            1 => connections.push(prompt_confluence_connection(&seen_names)?),
            _ => break,
        }

        if let Some(last) = connections.last() {
            seen_names.insert(last.name.clone());
        }
    }

    Ok(connections)
}

fn prompt_jira_connection(seen: &BTreeSet<String>) -> anyhow::Result<SourceConnection> {
    println!("\n  --- Jira connection ---");
    let name = prompt_unique_name(
        seen,
        "  Connection name (used in `instances:` references):",
        "jira-cloud",
    )?;

    let base_url: String = inquire::Text::new("  Base URL (e.g. https://your-org.atlassian.net):")
        .with_initial_value("https://your-org.atlassian.net")
        .prompt()?;

    let is_cloud = prompt_hosting_kind("Jira")?;

    let projects_str: String =
        inquire::Text::new("  Project keys to ingest (comma-separated, e.g. PROJ,ENG):")
            .prompt()?;
    let projects: Vec<String> = parse_csv(&projects_str);

    let auth = prompt_source_auth(is_cloud, "jira", &name)?;

    Ok(SourceConnection {
        name,
        source_type: SourceType::Jira,
        base_url,
        auth,
        projects,
        spaces: vec![],
    })
}

fn prompt_confluence_connection(seen: &BTreeSet<String>) -> anyhow::Result<SourceConnection> {
    println!("\n  --- Confluence connection ---");
    let name = prompt_unique_name(
        seen,
        "  Connection name (used in `instances:` references):",
        "confluence-cloud",
    )?;

    let base_url: String =
        inquire::Text::new("  Base URL (e.g. https://your-org.atlassian.net/wiki):")
            .with_initial_value("https://your-org.atlassian.net/wiki")
            .prompt()?;

    let is_cloud = prompt_hosting_kind("Confluence")?;

    let spaces_str: String =
        inquire::Text::new("  Space keys to ingest (comma-separated, e.g. ENG,DOCS):").prompt()?;
    let spaces: Vec<String> = parse_csv(&spaces_str);

    let auth = prompt_source_auth(is_cloud, "confluence", &name)?;

    Ok(SourceConnection {
        name,
        source_type: SourceType::Confluence,
        base_url,
        auth,
        projects: vec![],
        spaces,
    })
}

fn prompt_unique_name(
    seen: &BTreeSet<String>,
    prompt: &str,
    initial: &str,
) -> anyhow::Result<String> {
    loop {
        let name: String = inquire::Text::new(prompt)
            .with_initial_value(initial)
            .prompt()?;
        if name.trim().is_empty() {
            println!("  Name cannot be empty.");
            continue;
        }
        if seen.contains(&name) {
            println!(
                "  Name '{name}' is already used by another connection. Pick a different name."
            );
            continue;
        }
        return Ok(name);
    }
}

/// Atlassian Cloud (email + token) vs Data Center / Server (PAT only).
fn prompt_hosting_kind(product: &str) -> anyhow::Result<bool> {
    let idx = inquire::Select::new(
        &format!("  Where is your {product} hosted?"),
        vec![
            "Atlassian Cloud (*.atlassian.net)",
            "Data Center / Server (self-hosted)",
        ],
    )
    .with_starting_cursor(0)
    .raw_prompt()?
    .index;
    Ok(idx == 0)
}

fn prompt_source_auth(
    is_cloud: bool,
    product_hint: &str,
    connection_name: &str,
) -> anyhow::Result<SourceAuth> {
    let env_stem = env_var_stem_from_name(connection_name);
    if is_cloud {
        println!(
            "\n  Atlassian Cloud uses email + API token. Create the token at\n\
             https://id.atlassian.com/manage-profile/security/api-tokens — the\n\
             account that owns it must have read access to your projects/spaces."
        );
        let email_default = git_user_email().unwrap_or_default();
        let mut email_prompt = inquire::Text::new("  Atlassian account email:");
        if !email_default.is_empty() {
            email_prompt = email_prompt.with_initial_value(&email_default);
        }
        let email: String = email_prompt.prompt()?;

        let var = prompt_credential_env_var(
            product_hint,
            &format!("{env_stem}_API_TOKEN"),
            "Atlassian Cloud API token",
        )?;
        Ok(SourceAuth::Basic {
            email,
            token: format!("${{{var}}}"),
        })
    } else {
        println!(
            "\n  Data Center / Server uses a Personal Access Token (PAT). Generate one\n\
             from your profile → Personal Access Tokens with read access to the\n\
             projects/spaces above."
        );
        let var = prompt_credential_env_var(
            product_hint,
            &format!("{env_stem}_PAT"),
            "Personal Access Token",
        )?;
        Ok(SourceAuth::Pat {
            token: format!("${{{var}}}"),
        })
    }
}

/// Convert a name into a sensible env-var name suffix (uppercase, non-alnum
/// → `_`). E.g. `jira-cloud` → `JIRA_CLOUD`.
fn env_var_stem_from_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn parse_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Best-effort: read `git config user.email` to suggest a default for the
/// Atlassian Cloud email field.
fn git_user_email() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["config", "--get", "user.email"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let email = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if email.is_empty() { None } else { Some(email) }
}

/// Scan the current process env for variable names that look like they hold
/// a credential for the given product. Reads names only — never values.
pub(crate) fn find_token_env_vars(product_hint: &str) -> Vec<String> {
    let names = std::env::vars_os().filter_map(|(k, _)| k.into_string().ok());
    match_token_env_var_names(product_hint, names)
}

/// Pure-function core of [`find_token_env_vars`] — separated for testing.
fn match_token_env_var_names<I>(product_hint: &str, names: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let hint = product_hint.to_ascii_uppercase();
    let mut hits: BTreeSet<String> = BTreeSet::new();
    for name in names {
        let upper = name.to_ascii_uppercase();
        if !upper.contains(&hint) {
            continue;
        }
        if upper.contains("PAT")
            || upper.contains("TOKEN")
            || upper.contains("API_KEY")
            || upper.contains("APIKEY")
        {
            hits.insert(name);
        }
    }
    hits.into_iter().collect()
}

/// Prompt the user to pick (or name) the env var that will hold a credential.
/// Returns the env-var name only — never the value.
fn prompt_credential_env_var(
    product_hint: &str,
    default_name: &str,
    scope_text: &str,
) -> anyhow::Result<String> {
    let candidates = find_token_env_vars(product_hint);
    const ENTER_NAME: &str = "Use a different env var (enter name)…";

    if candidates.is_empty() {
        println!(
            "  No {product_hint}-related env vars found in your shell. Quelch will\n  \
             store a `${{<NAME>}}` placeholder in quelch.yaml — set the env var\n  \
             before running `quelch …`, both locally and on whatever runs Q-Ingest."
        );
        let name: String =
            inquire::Text::new(&format!("  Env var name that will hold the {scope_text}:"))
                .with_initial_value(default_name)
                .prompt()?;
        return Ok(name);
    }

    println!(
        "  Found {} env var(s) that look like a {product_hint} credential.\n  \
         (Quelch only reads the NAME — the value is not displayed or written\n  \
         to quelch.yaml; only `${{<NAME>}}` is.)",
        candidates.len()
    );

    let mut labels: Vec<String> = candidates
        .iter()
        .map(|name| {
            let set = std::env::var(name).map(|v| !v.is_empty()).unwrap_or(false);
            let marker = if set { "(set)" } else { "(empty!)" };
            format!("{name}  {marker}")
        })
        .collect();
    let enter_name_idx = labels.len();
    labels.push(ENTER_NAME.to_string());

    let idx = inquire::Select::new(&format!("  Which env var holds the {scope_text}?"), labels)
        .with_starting_cursor(0)
        .raw_prompt()?
        .index;

    if idx < candidates.len() {
        Ok(candidates[idx].clone())
    } else {
        debug_assert_eq!(idx, enter_name_idx);
        let name: String =
            inquire::Text::new(&format!("  Env var name that will hold the {scope_text}:"))
                .with_initial_value(default_name)
                .prompt()?;
        Ok(name)
    }
}

// ---------------------------------------------------------------------------
// Instances section
// ---------------------------------------------------------------------------

/// Prompt to add one or more instances (q-ingest / q-mcp). At least one
/// instance is required by `quelch validate`, so the loop runs at least once.
pub async fn prompt_instances(
    connections: &[SourceConnection],
) -> anyhow::Result<Vec<InstanceConfig>> {
    println!("\n=== Instances ===");
    println!(
        "An instance is one process you'll run somewhere. Two kinds:\n  \
         - ingest: pulls from one or more source connections, writes to Cosmos.\n  \
         - mcp: serves the agent-facing MCP API, queries Cosmos + AI Search.\n\
         A typical setup has one of each. Add as many as you need.\n"
    );

    let mut instances: Vec<InstanceConfig> = Vec::new();
    let mut seen_names: BTreeSet<String> = BTreeSet::new();

    loop {
        let must_add = instances.is_empty();
        let prompt = if must_add {
            "Add an instance:"
        } else {
            "Add another instance?"
        };
        let mut options = vec!["Ingest", "MCP"];
        if !must_add {
            options.push("Done (no more instances)");
        }
        let idx = inquire::Select::new(prompt, options.clone())
            .with_starting_cursor(0)
            .raw_prompt()?
            .index;

        // "Done" only present when !must_add, so it sits at index 2.
        if idx == 2 {
            break;
        }
        let inst = if idx == 0 {
            prompt_ingest_instance(&seen_names, connections)?
        } else {
            prompt_mcp_instance(&seen_names, connections)?
        };
        seen_names.insert(inst.name.clone());
        instances.push(inst);
    }

    Ok(instances)
}

fn prompt_ingest_instance(
    seen: &BTreeSet<String>,
    connections: &[SourceConnection],
) -> anyhow::Result<InstanceConfig> {
    println!("\n  --- Ingest instance ---");

    if connections.is_empty() {
        anyhow::bail!(
            "an ingest instance needs at least one source connection — go back \
             and add one first (this should not happen via the wizard)"
        );
    }

    let name = prompt_unique_name(
        seen,
        "  Instance name (used by `quelch ingest --instance ...`):",
        "ingest-main",
    )?;

    let labels: Vec<String> = connections
        .iter()
        .map(|c| format!("{} ({:?}, {})", c.name, c.source_type, c.base_url))
        .collect();
    let chosen_indices =
        inquire::MultiSelect::new("  Connections this instance should ingest from:", labels)
            .with_default(&[0])
            .raw_prompt()?
            .into_iter()
            .map(|s| s.index)
            .collect::<Vec<_>>();

    if chosen_indices.is_empty() {
        anyhow::bail!("an ingest instance must reference at least one connection");
    }

    let chosen_connections: Vec<String> = chosen_indices
        .iter()
        .map(|i| connections[*i].name.clone())
        .collect();

    let interval_str: String = inquire::Text::new("  Cycle interval (e.g. 5m, 30s, 1h):")
        .with_initial_value("5m")
        .prompt()?;
    let cycle_interval = parse_duration(&interval_str)?;

    Ok(InstanceConfig {
        name,
        spec: InstanceSpec::Ingest(IngestInstance {
            connections: chosen_connections,
            cycle_interval,
        }),
    })
}

fn prompt_mcp_instance(
    seen: &BTreeSet<String>,
    connections: &[SourceConnection],
) -> anyhow::Result<InstanceConfig> {
    println!("\n  --- MCP instance ---");

    let name = prompt_unique_name(
        seen,
        "  Instance name (used by `quelch mcp --instance ...`):",
        "mcp-prod",
    )?;

    // Derive available logical data sources from the kinds of connections
    // declared so far. This matches `config::data_sources::resolve`.
    let available = available_data_sources(connections);
    if available.is_empty() {
        println!(
            "  (no source connections declared — the MCP instance needs at least one\n  \
             data source. Add a connection first, then re-run init.)"
        );
        anyhow::bail!("MCP instance needs at least one source connection to expose");
    }

    let chosen_indices = inquire::MultiSelect::new("  Data sources to expose:", available.clone())
        .with_default(&(0..available.len()).collect::<Vec<_>>())
        .raw_prompt()?
        .into_iter()
        .map(|s| s.index)
        .collect::<Vec<_>>();
    if chosen_indices.is_empty() {
        anyhow::bail!("an MCP instance must expose at least one data source");
    }
    let expose: Vec<String> = chosen_indices
        .iter()
        .map(|i| available[*i].clone())
        .collect();

    let env_stem = env_var_stem_from_name(&name);
    let api_key_var =
        prompt_credential_env_var("mcp", &format!("{env_stem}_API_KEY"), "MCP API key")?;
    let api_key = format!("${{{api_key_var}}}");

    let listen: String = inquire::Text::new("  Listen address:")
        .with_initial_value("0.0.0.0:8080")
        .prompt()?;

    let knowledge_base: String = inquire::Text::new("  Knowledge Base name (rigg + AI Search):")
        .with_initial_value("quelch-prod-kb")
        .prompt()?;

    Ok(InstanceConfig {
        name,
        spec: InstanceSpec::Mcp(McpInstance {
            expose,
            api_key,
            knowledge_base,
            listen,
        }),
    })
}

/// Logical data sources implied by the kinds of source connections present.
/// Mirrors [`crate::config::data_sources::resolve`] but produces just the
/// public names so the MCP `expose:` MultiSelect can offer them.
pub(crate) fn available_data_sources(connections: &[SourceConnection]) -> Vec<String> {
    let mut has_jira = false;
    let mut has_confluence = false;
    for c in connections {
        match c.source_type {
            SourceType::Jira => has_jira = true,
            SourceType::Confluence => has_confluence = true,
        }
    }
    let mut out = Vec::new();
    if has_jira {
        out.extend([
            "jira_issues".to_string(),
            "jira_sprints".to_string(),
            "jira_fix_versions".to_string(),
            "jira_projects".to_string(),
        ]);
    }
    if has_confluence {
        out.extend([
            "confluence_pages".to_string(),
            "confluence_spaces".to_string(),
        ]);
    }
    out
}

/// Parse a duration string like "5m", "30s", "1h", returning a [`Duration`].
fn parse_duration(s: &str) -> anyhow::Result<Duration> {
    humantime::parse_duration(s.trim()).map_err(|e| anyhow::anyhow!("invalid duration '{s}': {e}"))
}

// ---------------------------------------------------------------------------
// Env-var summary
// ---------------------------------------------------------------------------

/// Scan a YAML string for `${VAR_NAME}` references and return the unique set
/// of variable names. Hand-rolled (cheaper than pulling in `regex`) and
/// matches what `shellexpand` resolves at config-load time.
pub fn collect_env_var_refs(yaml: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes = yaml.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'$' && bytes[i + 1] == b'{' {
            let start = i + 2;
            if let Some(end_offset) = bytes[start..].iter().position(|&b| b == b'}') {
                let name = &yaml[start..start + end_offset];
                let valid = !name.is_empty()
                    && name
                        .bytes()
                        .next()
                        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
                if valid {
                    out.insert(name.to_string());
                }
                i = start + end_offset + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_token_env_var_names_finds_jira_pat_variants() {
        let names = vec![
            "PATH".to_string(),
            "JIRA_PAT".to_string(),
            "JIRA_CLOUD_PAT".to_string(),
            "CONFLUENCE_PAT".to_string(),
            "JIRA_API_TOKEN".to_string(),
        ];
        let hits = match_token_env_var_names("jira", names);
        assert_eq!(
            hits,
            vec![
                "JIRA_API_TOKEN".to_string(),
                "JIRA_CLOUD_PAT".to_string(),
                "JIRA_PAT".to_string(),
            ]
        );
    }

    #[test]
    fn match_token_env_var_names_excludes_path_substring_only_match() {
        let names = vec!["PATH".to_string(), "EDITOR".to_string()];
        let hits = match_token_env_var_names("jira", names);
        assert!(hits.is_empty(), "PATH must not match jira-pat search");
    }

    #[test]
    fn env_var_stem_uppercases_and_replaces_punctuation() {
        assert_eq!(env_var_stem_from_name("jira-cloud"), "JIRA_CLOUD");
        assert_eq!(env_var_stem_from_name("confluence.dc"), "CONFLUENCE_DC");
        assert_eq!(env_var_stem_from_name("MyJira"), "MYJIRA");
    }

    #[test]
    fn collect_env_var_refs_finds_unique_placeholders() {
        let yaml =
            "auth:\n  pat: ${JIRA_PAT}\n  again: ${JIRA_PAT}\n  api: ${CONFLUENCE_API_TOKEN}\n";
        let refs = collect_env_var_refs(yaml);
        assert_eq!(refs.len(), 2);
        assert!(refs.contains("JIRA_PAT"));
        assert!(refs.contains("CONFLUENCE_API_TOKEN"));
    }

    #[test]
    fn collect_env_var_refs_ignores_malformed() {
        let yaml = "${UNTERMINATED\nfoo: ${1BADNAME}\nbar: ${}\nok: ${GOOD}\n";
        let refs = collect_env_var_refs(yaml);
        assert_eq!(refs.iter().collect::<Vec<_>>(), vec!["GOOD"]);
    }

    #[test]
    fn parse_duration_accepts_common_formats() {
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn parse_duration_rejects_garbage() {
        assert!(parse_duration("five minutes").is_err());
    }

    #[test]
    fn parse_csv_trims_and_filters_empty() {
        assert_eq!(
            parse_csv("PROJ, ENG ,, , ANNA"),
            vec!["PROJ".to_string(), "ENG".to_string(), "ANNA".to_string()]
        );
        assert!(parse_csv("").is_empty());
    }

    #[test]
    fn available_data_sources_picks_jira_only_when_only_jira_present() {
        let conns = vec![SourceConnection {
            name: "j".to_string(),
            source_type: SourceType::Jira,
            base_url: "https://j".to_string(),
            auth: SourceAuth::Pat {
                token: "T".to_string(),
            },
            projects: vec!["X".to_string()],
            spaces: vec![],
        }];
        let avail = available_data_sources(&conns);
        assert!(avail.contains(&"jira_issues".to_string()));
        assert!(avail.contains(&"jira_projects".to_string()));
        assert!(!avail.iter().any(|d| d.starts_with("confluence")));
    }

    #[test]
    fn available_data_sources_returns_empty_when_no_connections() {
        let avail = available_data_sources(&[]);
        assert!(avail.is_empty());
    }
}
