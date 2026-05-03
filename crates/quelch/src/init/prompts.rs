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

/// Prompt for Azure subscription, resource group, and region.
pub async fn azure_section() -> anyhow::Result<AzureConfig> {
    println!("\n=== Azure resources ===");
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
        let chosen = dialoguer::Select::new()
            .with_prompt("Subscription")
            .items(&names)
            .default(default_idx)
            .interact()?;
        subs[chosen].id.clone()
    } else {
        println!("  (az not available or no subscriptions found — enter manually)");
        dialoguer::Input::new()
            .with_prompt("Subscription ID")
            .interact_text()?
    };

    let resource_group: String = dialoguer::Input::new()
        .with_prompt("Resource group name")
        .with_initial_text("rg-quelch-prod")
        .interact_text()?;

    let region: String = dialoguer::Input::new()
        .with_prompt("Azure region")
        .with_initial_text("swedencentral")
        .interact_text()?;

    let naming_prefix: String = dialoguer::Input::new()
        .with_prompt("Resource naming prefix")
        .with_initial_text("quelch")
        .interact_text()?;

    let naming_env: String = dialoguer::Input::new()
        .with_prompt("Environment tag (e.g. prod, staging)")
        .with_initial_text("prod")
        .interact_text()?;

    Ok(AzureConfig {
        subscription_id,
        resource_group,
        region,
        naming: NamingConfig {
            prefix: Some(naming_prefix),
            environment: Some(naming_env),
        },
        skip_role_assignments: false,
    })
}

// ---------------------------------------------------------------------------
// OpenAI section
// ---------------------------------------------------------------------------

/// Prompt for Azure OpenAI endpoint and embedding deployment.
pub async fn openai_section(
    azure: &AzureConfig,
    _subscription_id: &str,
) -> anyhow::Result<OpenAiConfig> {
    println!("\n=== Azure OpenAI ===");
    println!(
        "Looking for Azure OpenAI accounts in '{}'...",
        azure.resource_group
    );

    let discovered = discover::find_openai_account(&azure.subscription_id, &azure.resource_group)
        .await
        .ok()
        .flatten();

    let default_endpoint = discovered
        .as_ref()
        .map(|a| a.endpoint.clone())
        .unwrap_or_else(|| "https://YOUR-OPENAI.openai.azure.com".to_string());

    let endpoint: String = dialoguer::Input::new()
        .with_prompt("Azure OpenAI endpoint")
        .with_initial_text(&default_endpoint)
        .interact_text()?;

    let deployment_name: String = dialoguer::Input::new()
        .with_prompt("Embedding deployment name")
        .with_initial_text("text-embedding-3-large")
        .interact_text()?;

    let dimensions_str: String = dialoguer::Input::new()
        .with_prompt("Embedding dimensions")
        .with_initial_text("3072")
        .interact_text()?;
    let embedding_dimensions: u32 = dimensions_str
        .parse()
        .map_err(|_| anyhow::anyhow!("embedding dimensions must be a number"))?;

    Ok(OpenAiConfig {
        endpoint,
        embedding_deployment: deployment_name,
        embedding_dimensions,
    })
}

// ---------------------------------------------------------------------------
// Sources section
// ---------------------------------------------------------------------------

/// Prompt to add one or more Jira/Confluence sources.
pub async fn sources_section() -> anyhow::Result<Vec<SourceConfig>> {
    println!("\n=== Source connections ===");
    let mut sources = Vec::new();

    loop {
        let add = dialoguer::Select::new()
            .with_prompt("Add a source?")
            .items(&["Jira", "Confluence", "Done (no more sources)"])
            .default(0)
            .interact()?;

        match add {
            0 => sources.push(SourceConfig::Jira(prompt_jira_source()?)),
            1 => sources.push(SourceConfig::Confluence(prompt_confluence_source()?)),
            _ => break,
        }
    }

    Ok(sources)
}

/// Prompt for a Jira source and return a built `JiraSourceConfig`.
pub fn prompt_jira_source() -> anyhow::Result<JiraSourceConfig> {
    println!("  --- Jira source ---");
    let name: String = dialoguer::Input::new()
        .with_prompt("  Source name (unique identifier)")
        .with_initial_text("jira-cloud")
        .interact_text()?;

    let url: String = dialoguer::Input::new()
        .with_prompt("  Jira URL")
        .with_initial_text("https://your-org.atlassian.net")
        .interact_text()?;

    let is_cloud = dialoguer::Confirm::new()
        .with_prompt("  Is this Atlassian Cloud (yes) or Data Center (no)?")
        .default(true)
        .interact()?;

    let auth = if is_cloud {
        let email: String = dialoguer::Input::new()
            .with_prompt("  Atlassian account email")
            .interact_text()?;
        let api_token: String = dialoguer::Password::new()
            .with_prompt(
                "  API token (https://id.atlassian.com/manage-profile/security/api-tokens)",
            )
            .interact()?;
        AuthConfig::Cloud { email, api_token }
    } else {
        let pat: String = dialoguer::Password::new()
            .with_prompt("  Personal Access Token")
            .interact()?;
        AuthConfig::DataCenter { pat }
    };

    let projects_str: String = dialoguer::Input::new()
        .with_prompt("  Project keys (comma-separated, e.g. PROJ,ENG)")
        .interact_text()?;
    let projects: Vec<String> = projects_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

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
    println!("  --- Confluence source ---");
    let name: String = dialoguer::Input::new()
        .with_prompt("  Source name (unique identifier)")
        .with_initial_text("confluence-cloud")
        .interact_text()?;

    let url: String = dialoguer::Input::new()
        .with_prompt("  Confluence URL")
        .with_initial_text("https://your-org.atlassian.net/wiki")
        .interact_text()?;

    let is_cloud = dialoguer::Confirm::new()
        .with_prompt("  Is this Atlassian Cloud (yes) or Data Center (no)?")
        .default(true)
        .interact()?;

    let auth = if is_cloud {
        let email: String = dialoguer::Input::new()
            .with_prompt("  Atlassian account email")
            .interact_text()?;
        let api_token: String = dialoguer::Password::new()
            .with_prompt("  API token")
            .interact()?;
        AuthConfig::Cloud { email, api_token }
    } else {
        let pat: String = dialoguer::Password::new()
            .with_prompt("  Personal Access Token")
            .interact()?;
        AuthConfig::DataCenter { pat }
    };

    let spaces_str: String = dialoguer::Input::new()
        .with_prompt("  Space keys (comma-separated, e.g. ENG,DOCS)")
        .interact_text()?;
    let spaces: Vec<String> = spaces_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

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

    let shapes = [
        "All in Azure (ingest + MCP both as Azure Container Apps)",
        "Ingest on-prem + MCP in Azure",
        "Custom (configure each deployment manually)",
    ];

    let chosen = dialoguer::Select::new()
        .with_prompt("Deployment shape")
        .items(&shapes)
        .default(0)
        .interact()?;

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
