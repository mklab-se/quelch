/// Interactive prompt sections for `quelch init`.
///
/// Each `*_section` function drives one section of the wizard and returns
/// the corresponding config struct. Credential testing is best-effort:
/// if a test fails the user is warned but not blocked.
use crate::config::*;

use super::discover;
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
        let chosen = inquire::Select::new("Subscription", names)
            .with_starting_cursor(default_idx)
            .raw_prompt()?
            .index;
        subs[chosen].id.clone()
    } else {
        println!("  (az not available or no subscriptions found — enter manually)");
        inquire::Text::new("Subscription ID").prompt()?
    };

    let (resource_group, _location) = pick_resource_group(
        &subscription_id,
        "Resource group for Quelch's Container Apps",
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

    let naming_prefix: String = inquire::Text::new("Resource naming prefix")
        .with_initial_value("quelch")
        .prompt()?;

    let naming_env: String = inquire::Text::new("Environment tag (e.g. prod, staging, dev)")
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
        let name = inquire::Text::new("New resource group name").prompt()?;
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

        let idx = inquire::Select::new(&format!("Pick a {kind}"), labels)
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
                &format!("Resource group to scan for {kind}s"),
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
            let endpoint: String = inquire::Text::new(&format!("{kind} endpoint"))
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
        inquire::Text::new("Embedding deployment name")
            .with_initial_value("text-embedding-3-large")
            .prompt()?
    } else {
        let labels: Vec<_> = candidates
            .iter()
            .map(|d| format!("{} ({})", d.name, d.model_name))
            .collect();
        let chosen = inquire::Select::new("Embedding deployment", labels)
            .with_starting_cursor(0)
            .raw_prompt()?
            .index;
        candidates[chosen].name.clone()
    };

    let dims_str: String = inquire::Text::new("Embedding dimensions")
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
        let dep: String = inquire::Text::new("Chat deployment name")
            .with_initial_value("gpt-5-mini")
            .prompt()?;
        let model: String = inquire::Text::new("Chat model name")
            .with_initial_value(&dep)
            .prompt()?;
        (dep, model)
    } else {
        let labels: Vec<_> = candidates
            .iter()
            .map(|d| format!("{} ({})", d.name, d.model_name))
            .collect();
        let chosen = inquire::Select::new("Chat (LLM) deployment", labels)
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

/// Prompt for a Jira source and return a built `JiraSourceConfig`.
pub fn prompt_jira_source() -> anyhow::Result<JiraSourceConfig> {
    println!("\n  --- Jira source ---");
    let name: String =
        inquire::Text::new("  Short identifier for this source (used in `quelch query --source`)")
            .with_initial_value("jira-cloud")
            .prompt()?;

    let url: String = inquire::Text::new("  Base URL of your Jira instance")
        .with_initial_value("https://your-org.atlassian.net")
        .prompt()?;

    let is_cloud = prompt_hosting_kind("Jira")?;

    println!(
        "\n  Jira project keys are the short uppercase prefixes you see in issue\n\
         keys (e.g. issue PROJ-123 belongs to project PROJ). Quelch will only\n\
         ingest issues from the projects you list here."
    );
    let projects_str: String =
        inquire::Text::new("  Project keys to ingest (comma-separated, e.g. PROJ,ENG)").prompt()?;
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

    let auth = if is_cloud {
        println!(
            "\n  Atlassian Cloud uses email + API token. Create one at\n\
             https://id.atlassian.com/manage-profile/security/api-tokens — the\n\
             account that owns the token must have read access to {project_list}."
        );
        let email: String =
            inquire::Text::new("  Atlassian account email (the API-token owner)").prompt()?;
        let api_token: String =
            inquire::Password::new(&format!("  Paste the API token for {email}"))
                .without_confirmation()
                .prompt()?;
        AuthConfig::Cloud { email, api_token }
    } else {
        println!(
            "\n  Jira Data Center uses a Personal Access Token (PAT). Generate one\n\
             from your Jira profile → Personal Access Tokens. The token's owner\n\
             must have read access to {project_list}."
        );
        let pat: String = inquire::Password::new("  Paste the PAT")
            .without_confirmation()
            .prompt()?;
        AuthConfig::DataCenter { pat }
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
        inquire::Text::new("  Short identifier for this source (used in `quelch query --source`)")
            .with_initial_value("confluence-cloud")
            .prompt()?;

    let url: String = inquire::Text::new("  Base URL of your Confluence instance")
        .with_initial_value("https://your-org.atlassian.net/wiki")
        .prompt()?;

    let is_cloud = prompt_hosting_kind("Confluence")?;

    println!(
        "\n  Confluence space keys are the short identifiers shown in space URLs\n\
         (e.g. /spaces/ENG/ → key `ENG`). Quelch will only ingest pages from the\n\
         spaces you list here."
    );
    let spaces_str: String =
        inquire::Text::new("  Space keys to ingest (comma-separated, e.g. ENG,DOCS)").prompt()?;
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

    let auth = if is_cloud {
        println!(
            "\n  Atlassian Cloud uses email + API token. Create one at\n\
             https://id.atlassian.com/manage-profile/security/api-tokens — the\n\
             account that owns the token must have read access to {space_list}."
        );
        let email: String =
            inquire::Text::new("  Atlassian account email (the API-token owner)").prompt()?;
        let api_token: String =
            inquire::Password::new(&format!("  Paste the API token for {email}"))
                .without_confirmation()
                .prompt()?;
        AuthConfig::Cloud { email, api_token }
    } else {
        println!(
            "\n  Confluence Data Center uses a Personal Access Token (PAT). Generate\n\
             one from your Confluence profile → Personal Access Tokens. The token's\n\
             owner must have read access to {space_list}."
        );
        let pat: String = inquire::Password::new("  Paste the PAT")
            .without_confirmation()
            .prompt()?;
        AuthConfig::DataCenter { pat }
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

    let chosen = inquire::Select::new("Deployment shape", shapes)
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
