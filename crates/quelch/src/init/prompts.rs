/// Interactive prompt sections for `quelch init`.
///
/// Each `*_section` function drives one section of the wizard and returns
/// the corresponding config struct. Credential testing is best-effort:
/// if a test fails the user is warned but not blocked.
///
/// ## Credentials
///
/// Wizard-collected secrets (PATs, API tokens) are NEVER written to disk.
/// The wizard always stores `${ENV_VAR_NAME}` placeholders in the generated
/// YAML; `config::env::substitute_env_vars` resolves them at load time.
/// See [`find_token_env_vars`] / [`prompt_credential_env_var`].
use crate::config::*;

use super::discover;
use std::collections::BTreeSet;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Azure section
// ---------------------------------------------------------------------------

/// Prompt for Azure subscription and the resource group Quelch will deploy
/// its own Container Apps into.
///
/// The naming prefix and environment tag are deferred to [`naming_settings`]
/// (called only if the chosen deployment shape includes any Azure-targeted
/// deployments). Region is not asked at all — see the doc comment on
/// [`naming_settings`] for why.
pub async fn azure_section() -> anyhow::Result<AzureConfig> {
    println!("\n=== Azure subscription & deployment resource group ===");
    println!(
        "First we need the subscription, and the resource group Quelch will deploy\n\
         the Q-MCP / Q-Ingest Container Apps INTO. (Each external resource — Cosmos,\n\
         AI Search, the AI provider, etc. — is picked separately and may live in any\n\
         RG, even a different one from this deployment RG.)\n"
    );
    println!("Discovering Azure subscriptions...");

    let subs = discover::list_subscriptions().await.unwrap_or_default();

    let subscription_id = if !subs.is_empty() {
        let names: Vec<_> = subs
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

    let (resource_group, _location) = pick_resource_group(
        &subscription_id,
        "Resource group for Quelch's Container Apps:",
        None,
    )
    .await?;

    Ok(AzureConfig {
        subscription_id,
        resource_group,
        region: String::new(),
        naming: NamingConfig::default(),
        skip_role_assignments: false,
        resources: AzureExistingResources::default(),
    })
}

/// Prompt for the resource-naming prefix and environment tag — used when
/// generating the names of the Container Apps Quelch creates (e.g. with
/// prefix `quelch` and env `prod`, the MCP container app is named
/// `quelch-prod-mcp`). Called from [`deployments_section`] after the user
/// picks a shape that includes any Azure-targeted deployment.
///
/// Region is *not* asked: the Bicep template's `location` parameter defaults
/// to `resourceGroup().location`, and Quelch never overrides it at deploy
/// time, so any value the user typed here would be ignored.
pub async fn naming_settings(azure: &mut AzureConfig) -> anyhow::Result<()> {
    println!("\n=== Resource naming for the Container Apps Quelch will create ===");
    println!(
        "The Container Apps Quelch deploys are named <prefix>-<env>-<role>, e.g.\n\
         quelch-prod-mcp and quelch-prod-ingest. The prefix and env tag below also\n\
         feed the default names of the existing resources Quelch references — if\n\
         your Container Apps environment is called `cae-quelch-prod` you don't\n\
         have to override anything; otherwise edit the generated yaml later.\n"
    );

    let naming_prefix: String = inquire::Text::new("Resource naming prefix:")
        .with_initial_value("quelch")
        .prompt()?;

    let naming_env: String = inquire::Text::new("Environment tag (e.g. prod, staging, dev):")
        .with_initial_value("prod")
        .prompt()?;

    azure.naming = NamingConfig {
        prefix: Some(naming_prefix),
        environment: Some(naming_env),
    };
    Ok(())
}

/// Show a Select listing every resource group in the subscription, plus a
/// "Create new (enter name)…" escape hatch. Returns the chosen RG name and —
/// when the user picked an existing one — its Azure location for use as a
/// region default.
///
/// `default_rg` is the RG whose entry should be highlighted when the prompt
/// opens (e.g. the deployment RG when broadening discovery to a different one).
async fn pick_resource_group(
    subscription_id: &str,
    prompt_text: &str,
    default_rg: Option<&str>,
) -> anyhow::Result<(String, Option<String>)> {
    let groups = discover::list_resource_groups(subscription_id)
        .await
        .unwrap_or_default();

    if groups.is_empty() {
        println!("  (az returned no resource groups — enter the name manually)");
        let name = inquire::Text::new(prompt_text).prompt()?;
        return Ok((name, None));
    }

    const CREATE_NEW: &str = "Create new (enter name)…";
    let mut labels: Vec<String> = groups
        .iter()
        .map(|g| format!("{}  ({})", g.name, g.location))
        .collect();
    labels.push(CREATE_NEW.to_string());

    let starting_cursor = default_rg
        .and_then(|name| groups.iter().position(|g| g.name == name))
        .unwrap_or(0);

    let idx = inquire::Select::new(prompt_text, labels)
        .with_starting_cursor(starting_cursor)
        .raw_prompt()?
        .index;

    if idx < groups.len() {
        let g = &groups[idx];
        Ok((g.name.clone(), Some(g.location.clone())))
    } else {
        let name = inquire::Text::new("New resource group name:").prompt()?;
        Ok((name, None))
    }
}

// ---------------------------------------------------------------------------
// AI provider section (Foundry or Azure OpenAI; embedding + chat deployments)
// ---------------------------------------------------------------------------

/// Prompt for the AI model provider (Foundry or Azure OpenAI), discover
/// existing accounts in the chosen resource group, and pick embedding +
/// chat deployments inside the selected account.
///
/// If no accounts are found we tell the user clearly and offer the manual
/// entry fallback so they can still write the config — `quelch validate`
/// catches it at re-run time.
pub async fn ai_section(azure: &AzureConfig) -> anyhow::Result<AiConfig> {
    println!("\n=== AI model provider ===");
    println!("Quelch wires up two model deployments in your AI Search Knowledge Base:");
    println!("  - an embedding model (used by the vectorizer / skillset)");
    println!("  - a chat / LLM (used for query planning + answer synthesis)");
    println!("Both can live in the same Azure OpenAI account or Microsoft Foundry project.\n");

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

    let (endpoint, account_name, resource_group_override) =
        pick_ai_account(azure, provider).await?;

    // Discover deployments inside the chosen account/project (best-effort) —
    // looking in whichever RG that account lives in.
    let lookup_rg = resource_group_override
        .as_deref()
        .unwrap_or(azure.resource_group.as_str());
    let deployments = match account_name {
        Some(ref name) => discover::list_model_deployments(&azure.subscription_id, lookup_rg, name)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };

    let embedding = pick_embedding_deployment(&deployments)?;
    let chat = pick_chat_deployment(&deployments)?;

    Ok(AiConfig {
        provider,
        endpoint,
        resource_group: resource_group_override,
        embedding,
        chat,
    })
}

/// Pick a Foundry project or Azure OpenAI account, with the option to scan a
/// different resource group than the deployment RG. Returns
/// `(endpoint, account_name, resource_group_override)` where the override is
/// `Some(rg)` only when the user picked an account from an RG different from
/// `azure.resource_group`.
async fn pick_ai_account(
    azure: &AzureConfig,
    provider: AiProvider,
) -> anyhow::Result<(String, Option<String>, Option<String>)> {
    const PICK_DIFFERENT_RG: &str = "Search a different resource group…";
    const ENTER_MANUALLY: &str = "Enter endpoint manually…";

    let kind = match provider {
        AiProvider::Foundry => "Foundry project",
        AiProvider::AzureOpenai => "Azure OpenAI account",
    };

    let mut current_rg = azure.resource_group.clone();
    loop {
        println!("Looking for {kind}s in '{current_rg}'...");
        let candidates: Vec<(String, String)> = match provider {
            AiProvider::Foundry => {
                discover::list_foundry_projects(&azure.subscription_id, &current_rg)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| (p.name, p.endpoint))
                    .collect()
            }
            AiProvider::AzureOpenai => {
                discover::list_openai_accounts(&azure.subscription_id, &current_rg)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|a| (a.name, a.endpoint))
                    .collect()
            }
        };

        if candidates.is_empty() {
            println!("  ✗ No {kind}s found in resource group '{current_rg}'.");
        } else {
            println!("  Found {} {kind}(s) in '{current_rg}'.", candidates.len());
        }

        let mut labels: Vec<String> = candidates
            .iter()
            .map(|(name, endpoint)| format!("{name} — {endpoint}"))
            .collect();
        let pick_rg_idx = labels.len();
        labels.push(PICK_DIFFERENT_RG.to_string());
        let manual_idx = labels.len();
        labels.push(ENTER_MANUALLY.to_string());

        let idx = inquire::Select::new(&format!("Pick a {kind}:"), labels)
            .with_starting_cursor(0)
            .raw_prompt()?
            .index;

        if idx < candidates.len() {
            let (name, endpoint) = candidates[idx].clone();
            let rg_override = if current_rg != azure.resource_group {
                Some(current_rg)
            } else {
                None
            };
            return Ok((endpoint, Some(name), rg_override));
        } else if idx == pick_rg_idx {
            let (rg, _location) = pick_resource_group(
                &azure.subscription_id,
                &format!("Resource group to scan for {kind}s:"),
                Some(&current_rg),
            )
            .await?;
            current_rg = rg;
            // Loop and re-discover.
        } else {
            debug_assert_eq!(idx, manual_idx);
            let placeholder = match provider {
                AiProvider::Foundry => "https://YOUR-FOUNDRY.cognitiveservices.azure.com",
                AiProvider::AzureOpenai => "https://YOUR-OPENAI.openai.azure.com",
            };
            let endpoint: String = inquire::Text::new(&format!("{kind} endpoint:"))
                .with_initial_value(placeholder)
                .prompt()?;
            return Ok((endpoint, None, None));
        }
    }
}

/// Pick (or manually enter) an embedding deployment.
fn pick_embedding_deployment(
    available: &[discover::ModelDeployment],
) -> anyhow::Result<AiEmbeddingConfig> {
    let candidates: Vec<&discover::ModelDeployment> = available
        .iter()
        .filter(|d| d.model_name.starts_with("text-embedding"))
        .collect();

    let deployment = if candidates.is_empty() {
        if !available.is_empty() {
            println!("  (no embedding deployments found in the chosen account)");
        }
        inquire::Text::new("Embedding deployment name:")
            .with_initial_value("text-embedding-3-large")
            .prompt()?
    } else {
        let labels: Vec<_> = candidates
            .iter()
            .map(|d| format!("{} ({})", d.name, d.model_name))
            .collect();
        let chosen = inquire::Select::new("Embedding deployment:", labels)
            .with_starting_cursor(0)
            .raw_prompt()?
            .index;
        candidates[chosen].name.clone()
    };

    let dims_str: String = inquire::Text::new("Embedding dimensions:")
        .with_initial_value("3072")
        .prompt()?;
    let dimensions: u32 = dims_str
        .parse()
        .map_err(|_| anyhow::anyhow!("embedding dimensions must be a number"))?;

    Ok(AiEmbeddingConfig {
        deployment,
        dimensions,
    })
}

/// Supported chat models for AI Search agentic retrieval (per Azure AI Search
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

/// Pick (or manually enter) a chat deployment.
fn pick_chat_deployment(available: &[discover::ModelDeployment]) -> anyhow::Result<AiChatConfig> {
    let candidates: Vec<&discover::ModelDeployment> = available
        .iter()
        .filter(|d| SUPPORTED_CHAT_MODELS.iter().any(|m| d.model_name == *m))
        .collect();

    let (deployment, model_name) = if candidates.is_empty() {
        if !available.is_empty() {
            println!(
                "  (no supported chat deployments found; supported models: {})",
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
        let labels: Vec<_> = candidates
            .iter()
            .map(|d| format!("{} ({})", d.name, d.model_name))
            .collect();
        let chosen = inquire::Select::new("Chat (LLM) deployment:", labels)
            .with_starting_cursor(0)
            .raw_prompt()?
            .index;
        let c = candidates[chosen];
        (c.name.clone(), c.model_name.clone())
    };

    println!(
        "\nThe Knowledge Base uses your chat deployment for two things — query\n\
         planning (turning the user's question into search queries) and, optionally,\n\
         answer synthesis. The two questions below tune that behaviour."
    );
    let efforts = vec![
        "Minimal — skip the LLM, just run vector + keyword + semantic search",
        "Low — single-pass query plan (Azure portal default)",
        "Medium — allow follow-up subqueries when the first plan looks thin",
    ];
    let effort_idx = inquire::Select::new(
        "How hard should the LLM think when planning queries?",
        efforts,
    )
    .with_starting_cursor(1)
    .raw_prompt()?
    .index;
    let retrieval_reasoning_effort = match effort_idx {
        0 => ReasoningEffort::Minimal,
        1 => ReasoningEffort::Low,
        _ => ReasoningEffort::Medium,
    };

    let modes = vec![
        "Synthesised answer with citations (Azure portal default)",
        "Raw ranked search results — no LLM-side composition",
    ];
    let mode_idx = inquire::Select::new("What should the MCP `search` tool return?", modes)
        .with_starting_cursor(0)
        .raw_prompt()?
        .index;
    let output_mode = if mode_idx == 0 {
        OutputMode::AnswerSynthesis
    } else {
        OutputMode::ExtractedData
    };

    Ok(AiChatConfig {
        deployment,
        model_name,
        retrieval_reasoning_effort,
        output_mode,
    })
}

// ---------------------------------------------------------------------------
// Sources section
// ---------------------------------------------------------------------------

/// Prompt to add one or more Jira/Confluence sources.
pub async fn sources_section() -> anyhow::Result<Vec<SourceConfig>> {
    println!("\n=== Source connections ===");
    println!(
        "A source is one Jira or Confluence instance Quelch will ingest from.\n\
         You can add as many as you like — for example one Jira Cloud + one\n\
         Confluence Data Center. Quelch needs read-only access (an API token\n\
         for Cloud, a personal access token for Data Center) to whichever\n\
         projects / spaces you point it at.\n"
    );
    let mut sources = Vec::new();

    loop {
        let add = inquire::Select::new(
            "Add a source?",
            vec!["Jira", "Confluence", "Done (no more sources)"],
        )
        .with_starting_cursor(0)
        .raw_prompt()?
        .index;

        match add {
            0 => sources.push(SourceConfig::Jira(prompt_jira_source()?)),
            1 => sources.push(SourceConfig::Confluence(prompt_confluence_source()?)),
            _ => break,
        }
    }

    Ok(sources)
}

/// Prompt for whether a source is Atlassian Cloud or Data Center. Replaces
/// the historic yes/no Confirm with a Select — the Confirm phrasing
/// ("yes = Cloud, no = Data Center") was confusing because the answer isn't
/// a yes/no question.
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

/// Scan the current process env for variable names that look like they hold
/// a credential for the given product. Matches case-insensitively on names
/// that contain `product_hint` (e.g. "jira") AND at least one of
/// "PAT" / "TOKEN" / "API_KEY" / "APIKEY".
///
/// **Reads env-var names only, never values.** Sorted alphabetically,
/// duplicates removed.
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

/// Convert a source name into a sensible default env-var name suffix:
/// uppercase, non-alphanumerics replaced with `_`. E.g. `jira-cloud` →
/// `JIRA_CLOUD`.
fn env_var_stem_from_source_name(source_name: &str) -> String {
    source_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Prompt the user to pick (or name) the env var that will hold a credential.
/// Returns the env-var name — never the value.
///
/// `product_hint` filters which existing env vars to suggest (e.g. "jira").
/// `default_name` is the suggested name when the user picks "Use a different
/// env var" (typically derived from the source name).
/// `scope_text` is a short description of what the credential will be used
/// for, included in the prompt copy ("Personal Access Token for PROJ, ENG").
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

/// Best-effort: read `git config user.email` from the user's git config to
/// suggest as a default for the Atlassian Cloud email field. Returns `None`
/// if git is missing or no email is configured.
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

/// Prompt for a Jira source and return a built `JiraSourceConfig`.
pub fn prompt_jira_source() -> anyhow::Result<JiraSourceConfig> {
    println!("\n  --- Jira source ---");
    let name: String =
        inquire::Text::new("  Short identifier for this source (used in `quelch query --source`):")
            .with_initial_value("jira-cloud")
            .prompt()?;

    let url: String = inquire::Text::new("  Base URL of your Jira instance:")
        .with_initial_value("https://your-org.atlassian.net")
        .prompt()?;

    let is_cloud = prompt_hosting_kind("Jira")?;

    println!(
        "\n  Jira project keys are the short uppercase prefixes you see in issue\n\
         keys (e.g. issue PROJ-123 belongs to project PROJ). Quelch will only\n\
         ingest issues from the projects you list here."
    );
    let projects_str: String =
        inquire::Text::new("  Project keys to ingest (comma-separated, e.g. PROJ,ENG):")
            .prompt()?;
    let projects: Vec<String> = projects_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let project_list = if projects.is_empty() {
        "the projects above".to_string()
    } else {
        projects.join(", ")
    };

    let env_stem = env_var_stem_from_source_name(&name);
    let auth = if is_cloud {
        println!(
            "\n  Atlassian Cloud uses email + API token. Create the token at\n\
             https://id.atlassian.com/manage-profile/security/api-tokens — the\n\
             account that owns it must have read access to {project_list}."
        );
        let email_default = git_user_email().unwrap_or_default();
        let mut email_prompt =
            inquire::Text::new("  Atlassian account email (the API-token owner):");
        if !email_default.is_empty() {
            email_prompt = email_prompt.with_initial_value(&email_default);
        }
        let email: String = email_prompt.prompt()?;
        let var = prompt_credential_env_var(
            "jira",
            &format!("{env_stem}_API_TOKEN"),
            &format!("Atlassian Cloud API token for {project_list}"),
        )?;
        AuthConfig::Cloud {
            email,
            api_token: format!("${{{var}}}"),
        }
    } else {
        println!(
            "\n  Jira Data Center uses a Personal Access Token (PAT). Generate one\n\
             from your Jira profile → Personal Access Tokens. The PAT's owner\n\
             must have read access to {project_list}."
        );
        let var = prompt_credential_env_var(
            "jira",
            &format!("{env_stem}_PAT"),
            &format!("Jira Data Center PAT for {project_list}"),
        )?;
        AuthConfig::DataCenter {
            pat: format!("${{{var}}}"),
        }
    };

    Ok(build_jira_source(name, url, auth, projects))
}

/// Build a `JiraSourceConfig` from discrete values.
///
/// Separated from the prompt so it can be unit-tested independently.
pub fn build_jira_source(
    name: String,
    url: String,
    auth: AuthConfig,
    projects: Vec<String>,
) -> JiraSourceConfig {
    JiraSourceConfig {
        name,
        url,
        auth,
        projects,
        container: None,
        companion_containers: CompanionContainersConfig::default(),
        fields: HashMap::new(),
    }
}

/// Prompt for a Confluence source and return a built `ConfluenceSourceConfig`.
pub fn prompt_confluence_source() -> anyhow::Result<ConfluenceSourceConfig> {
    println!("\n  --- Confluence source ---");
    let name: String =
        inquire::Text::new("  Short identifier for this source (used in `quelch query --source`):")
            .with_initial_value("confluence-cloud")
            .prompt()?;

    let url: String = inquire::Text::new("  Base URL of your Confluence instance:")
        .with_initial_value("https://your-org.atlassian.net/wiki")
        .prompt()?;

    let is_cloud = prompt_hosting_kind("Confluence")?;

    println!(
        "\n  Confluence space keys are the short identifiers shown in space URLs\n\
         (e.g. /spaces/ENG/ → key `ENG`). Quelch will only ingest pages from the\n\
         spaces you list here."
    );
    let spaces_str: String =
        inquire::Text::new("  Space keys to ingest (comma-separated, e.g. ENG,DOCS):").prompt()?;
    let spaces: Vec<String> = spaces_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let space_list = if spaces.is_empty() {
        "the spaces above".to_string()
    } else {
        spaces.join(", ")
    };

    let env_stem = env_var_stem_from_source_name(&name);
    let auth = if is_cloud {
        println!(
            "\n  Atlassian Cloud uses email + API token. Create the token at\n\
             https://id.atlassian.com/manage-profile/security/api-tokens — the\n\
             account that owns it must have read access to {space_list}."
        );
        let email_default = git_user_email().unwrap_or_default();
        let mut email_prompt =
            inquire::Text::new("  Atlassian account email (the API-token owner):");
        if !email_default.is_empty() {
            email_prompt = email_prompt.with_initial_value(&email_default);
        }
        let email: String = email_prompt.prompt()?;
        let var = prompt_credential_env_var(
            "confluence",
            &format!("{env_stem}_API_TOKEN"),
            &format!("Atlassian Cloud API token for {space_list}"),
        )?;
        AuthConfig::Cloud {
            email,
            api_token: format!("${{{var}}}"),
        }
    } else {
        println!(
            "\n  Confluence Data Center uses a Personal Access Token (PAT). Generate\n\
             one from your Confluence profile → Personal Access Tokens. The PAT's\n\
             owner must have read access to {space_list}."
        );
        let var = prompt_credential_env_var(
            "confluence",
            &format!("{env_stem}_PAT"),
            &format!("Confluence Data Center PAT for {space_list}"),
        )?;
        AuthConfig::DataCenter {
            pat: format!("${{{var}}}"),
        }
    };

    Ok(build_confluence_source(name, url, auth, spaces))
}

/// Build a `ConfluenceSourceConfig` from discrete values.
pub fn build_confluence_source(
    name: String,
    url: String,
    auth: AuthConfig,
    spaces: Vec<String>,
) -> ConfluenceSourceConfig {
    ConfluenceSourceConfig {
        name,
        url,
        auth,
        spaces,
        container: None,
        companion_containers: CompanionContainersConfig::default(),
    }
}

// ---------------------------------------------------------------------------
// Deployments section
// ---------------------------------------------------------------------------

/// Prompt for deployment shape selection.
pub async fn deployments_section(
    sources: &[SourceConfig],
) -> anyhow::Result<Vec<DeploymentConfig>> {
    println!("\n=== Deployments ===");

    let shapes = vec![
        "All in Azure (ingest + MCP both as Azure Container Apps)",
        "Ingest on-prem + MCP in Azure",
        "Custom (configure each deployment manually)",
    ];

    let chosen = inquire::Select::new("Deployment shape:", shapes)
        .with_starting_cursor(0)
        .raw_prompt()?
        .index;

    match chosen {
        0 => Ok(all_azure_deployments(sources)),
        1 => Ok(split_deployments(sources)),
        _ => {
            println!("  Custom deployment setup is not yet supported by the wizard.");
            println!("  Using all-Azure defaults — edit quelch.yaml afterwards.");
            Ok(all_azure_deployments(sources))
        }
    }
}

fn all_azure_deployments(sources: &[SourceConfig]) -> Vec<DeploymentConfig> {
    let source_refs: Vec<DeploymentSource> = sources
        .iter()
        .map(|s| DeploymentSource {
            source: s.name().to_string(),
            projects: None,
            spaces: None,
        })
        .collect();

    let expose = auto_expose_list(sources);

    vec![
        DeploymentConfig {
            name: "ingest".to_string(),
            role: DeploymentRole::Ingest,
            target: DeploymentTarget::Azure,
            sources: Some(source_refs),
            expose: None,
            azure: Some(DeploymentAzureConfig {
                container_app: ContainerAppSpec {
                    cpu: Some(0.5),
                    memory: Some("1.0Gi".to_string()),
                    min_replicas: None,
                    max_replicas: None,
                },
            }),
            auth: None,
        },
        DeploymentConfig {
            name: "mcp".to_string(),
            role: DeploymentRole::Mcp,
            target: DeploymentTarget::Azure,
            sources: None,
            expose: Some(expose),
            azure: Some(DeploymentAzureConfig {
                container_app: ContainerAppSpec {
                    cpu: Some(1.0),
                    memory: Some("2.0Gi".to_string()),
                    min_replicas: Some(0),
                    max_replicas: None,
                },
            }),
            auth: Some(DeploymentAuthConfig {
                mode: McpAuthMode::ApiKey,
            }),
        },
    ]
}

fn split_deployments(sources: &[SourceConfig]) -> Vec<DeploymentConfig> {
    let source_refs: Vec<DeploymentSource> = sources
        .iter()
        .map(|s| DeploymentSource {
            source: s.name().to_string(),
            projects: None,
            spaces: None,
        })
        .collect();

    let expose = auto_expose_list(sources);

    vec![
        DeploymentConfig {
            name: "ingest-onprem".to_string(),
            role: DeploymentRole::Ingest,
            target: DeploymentTarget::Onprem,
            sources: Some(source_refs),
            expose: None,
            azure: None,
            auth: None,
        },
        DeploymentConfig {
            name: "mcp".to_string(),
            role: DeploymentRole::Mcp,
            target: DeploymentTarget::Azure,
            sources: None,
            expose: Some(expose),
            azure: Some(DeploymentAzureConfig {
                container_app: ContainerAppSpec {
                    cpu: Some(1.0),
                    memory: Some("2.0Gi".to_string()),
                    min_replicas: Some(0),
                    max_replicas: None,
                },
            }),
            auth: Some(DeploymentAuthConfig {
                mode: McpAuthMode::ApiKey,
            }),
        },
    ]
}

/// Derive a default expose list from sources.
fn auto_expose_list(sources: &[SourceConfig]) -> Vec<String> {
    let mut expose = Vec::new();
    for s in sources {
        match s {
            SourceConfig::Jira(_) => {
                if !expose.contains(&"jira_issues".to_string()) {
                    expose.push("jira_issues".to_string());
                }
            }
            SourceConfig::Confluence(_) => {
                if !expose.contains(&"confluence_pages".to_string()) {
                    expose.push("confluence_pages".to_string());
                }
            }
        }
    }
    expose
}

// ---------------------------------------------------------------------------
// MCP section
// ---------------------------------------------------------------------------

/// Prompt for MCP data source configuration.
pub async fn mcp_section(deployments: &[DeploymentConfig]) -> anyhow::Result<McpConfig> {
    // Derive data_sources from the MCP deployment's expose list (auto-derived).
    let expose: Vec<&str> = deployments
        .iter()
        .filter(|d| d.role == DeploymentRole::Mcp)
        .flat_map(|d| d.expose.as_deref().unwrap_or(&[]))
        .map(String::as_str)
        .collect();

    let mut data_sources = HashMap::new();
    for ds_name in expose {
        let (kind, container) = match ds_name {
            "jira_issues" => ("jira_issue", "jira-issues"),
            "confluence_pages" => ("confluence_page", "confluence-pages"),
            other => (other, other),
        };
        data_sources.insert(
            ds_name.to_string(),
            McpDataSourceSpec {
                kind: kind.to_string(),
                backed_by: vec![BackedBy {
                    container: container.to_string(),
                }],
            },
        );
    }

    Ok(McpConfig {
        data_sources,
        ..McpConfig::default()
    })
}

// ---------------------------------------------------------------------------
// Env-var summary helpers — used by the post-init "before running Quelch" hint
// ---------------------------------------------------------------------------

/// Scan a YAML string for `${VAR_NAME}` references and return the unique set
/// of variable names. Hand-rolled (cheaper than pulling in `regex`) and
/// matches what `shellexpand` will resolve at config-load time.
pub(crate) fn collect_env_var_refs(yaml: &str) -> BTreeSet<String> {
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
            "jira_pat_lowercase".to_string(),
            "CONFLUENCE_PAT".to_string(),
            "BITBUCKET_PAT".to_string(),
            "JIRA_API_TOKEN".to_string(),
            "JIRA_HOME".to_string(),
            "MY_jira_token_lowercase".to_string(),
        ];
        let hits = match_token_env_var_names("jira", names);
        assert_eq!(
            hits,
            vec![
                "JIRA_API_TOKEN".to_string(),
                "JIRA_CLOUD_PAT".to_string(),
                "JIRA_PAT".to_string(),
                "MY_jira_token_lowercase".to_string(),
                "jira_pat_lowercase".to_string(),
            ]
        );
    }

    #[test]
    fn match_token_env_var_names_excludes_path_substring_only_match() {
        // "PATH" contains "PAT" but not the product hint — must not match.
        let names = vec!["PATH".to_string(), "EDITOR".to_string()];
        let hits = match_token_env_var_names("jira", names);
        assert!(hits.is_empty(), "PATH must not match jira-pat search");
    }

    #[test]
    fn env_var_stem_uppercases_and_replaces_punctuation() {
        assert_eq!(env_var_stem_from_source_name("jira-cloud"), "JIRA_CLOUD");
        assert_eq!(
            env_var_stem_from_source_name("confluence.dc"),
            "CONFLUENCE_DC"
        );
        assert_eq!(env_var_stem_from_source_name("MyJira"), "MYJIRA");
    }

    #[test]
    fn collect_env_var_refs_finds_all_unique_placeholders() {
        let yaml =
            "auth:\n  pat: ${JIRA_PAT}\n  other: ${JIRA_PAT}\n  api: ${CONFLUENCE_API_TOKEN}\n";
        let refs = collect_env_var_refs(yaml);
        assert_eq!(refs.len(), 2);
        assert!(refs.contains("JIRA_PAT"));
        assert!(refs.contains("CONFLUENCE_API_TOKEN"));
    }

    #[test]
    fn collect_env_var_refs_ignores_malformed() {
        // Unterminated, leading-digit, empty — none should appear.
        let yaml = "${UNTERMINATED\nfoo: ${1BADNAME}\nbar: ${}\nok: ${GOOD}\n";
        let refs = collect_env_var_refs(yaml);
        assert_eq!(refs.iter().collect::<Vec<_>>(), vec!["GOOD"]);
    }

    #[test]
    fn build_jira_source_creates_correct_config() {
        let cfg = build_jira_source(
            "my-jira".to_string(),
            "https://example.atlassian.net".to_string(),
            AuthConfig::Cloud {
                email: "user@example.com".to_string(),
                api_token: "tok".to_string(),
            },
            vec!["PROJ".to_string(), "ENG".to_string()],
        );

        assert_eq!(cfg.name, "my-jira");
        assert_eq!(cfg.url, "https://example.atlassian.net");
        assert_eq!(cfg.projects, vec!["PROJ", "ENG"]);
        assert!(matches!(cfg.auth, AuthConfig::Cloud { .. }));
    }

    #[test]
    fn build_confluence_source_creates_correct_config() {
        let cfg = build_confluence_source(
            "my-confluence".to_string(),
            "https://example.atlassian.net/wiki".to_string(),
            AuthConfig::DataCenter {
                pat: "my-pat".to_string(),
            },
            vec!["ENG".to_string()],
        );

        assert_eq!(cfg.name, "my-confluence");
        assert!(matches!(cfg.auth, AuthConfig::DataCenter { .. }));
        assert_eq!(cfg.spaces, vec!["ENG"]);
    }

    #[test]
    fn auto_expose_list_derives_from_sources() {
        let sources = vec![
            SourceConfig::Jira(build_jira_source(
                "j".to_string(),
                "https://x.atlassian.net".to_string(),
                AuthConfig::Cloud {
                    email: "u@example.com".to_string(),
                    api_token: "t".to_string(),
                },
                vec!["X".to_string()],
            )),
            SourceConfig::Confluence(build_confluence_source(
                "c".to_string(),
                "https://x.atlassian.net/wiki".to_string(),
                AuthConfig::Cloud {
                    email: "u@example.com".to_string(),
                    api_token: "t".to_string(),
                },
                vec!["ENG".to_string()],
            )),
        ];
        let expose = auto_expose_list(&sources);
        assert!(expose.contains(&"jira_issues".to_string()));
        assert!(expose.contains(&"confluence_pages".to_string()));
    }

    #[test]
    fn all_azure_deployments_produces_two_deployments() {
        let sources = vec![SourceConfig::Jira(build_jira_source(
            "j".to_string(),
            "https://x.atlassian.net".to_string(),
            AuthConfig::Cloud {
                email: "u@example.com".to_string(),
                api_token: "t".to_string(),
            },
            vec!["X".to_string()],
        ))];
        let deps = all_azure_deployments(&sources);
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|d| d.role == DeploymentRole::Ingest));
        assert!(deps.iter().any(|d| d.role == DeploymentRole::Mcp));
    }
}
