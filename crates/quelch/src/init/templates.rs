/// Built-in config templates for `quelch init --from-template`.
///
/// Templates use `${ENV_VAR}` placeholders for credentials — the user fills
/// them in their shell or in a `.env` file.
use crate::config::*;
use std::collections::HashMap;

/// Return a template config by name.
///
/// # Errors
/// Returns an error if the name is not recognised.
pub fn template_for(name: &str) -> anyhow::Result<Config> {
    match name {
        "minimal" => Ok(minimal_template()),
        "multi-source" => Ok(multi_source_template()),
        "distributed" => Ok(distributed_template()),
        other => anyhow::bail!(
            "unknown template '{}'. Available: minimal, multi-source, distributed",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Minimal: one Jira Cloud source, ingest + MCP both in Azure.
// ---------------------------------------------------------------------------

/// Minimal template: one Jira Cloud source, Azure ingest + Azure MCP.
pub fn minimal_template() -> Config {
    Config {
        azure: AzureConfig {
            subscription_id: "${AZURE_SUBSCRIPTION_ID}".to_string(),
            resource_group: "rg-quelch-prod".to_string(),
            region: "swedencentral".to_string(),
            naming: NamingConfig {
                prefix: Some("quelch".to_string()),
                environment: Some("prod".to_string()),
            },
            skip_role_assignments: false,
        },
        cosmos: CosmosConfig::default(),
        search: SearchConfig::default(),
        ai: AiConfig {
            provider: AiProvider::Foundry,
            endpoint: "https://${FOUNDRY_PROJECT}.cognitiveservices.azure.com".to_string(),
            embedding: AiEmbeddingConfig {
                deployment: "text-embedding-3-large".to_string(),
                dimensions: 3072,
            },
            chat: AiChatConfig {
                deployment: "gpt-4.1-mini".to_string(),
                model_name: "gpt-4.1-mini".to_string(),
                retrieval_reasoning_effort: ReasoningEffort::Low,
                output_mode: OutputMode::AnswerSynthesis,
            },
        },
        sources: vec![SourceConfig::Jira(JiraSourceConfig {
            name: "jira-cloud".to_string(),
            url: "https://${JIRA_HOST}.atlassian.net".to_string(),
            auth: AuthConfig::Cloud {
                email: "${JIRA_EMAIL}".to_string(),
                api_token: "${JIRA_API_TOKEN}".to_string(),
            },
            projects: vec!["PROJ".to_string()],
            container: None,
            companion_containers: CompanionContainersConfig::default(),
            fields: HashMap::new(),
        })],
        ingest: IngestConfig::default(),
        deployments: vec![
            DeploymentConfig {
                name: "ingest".to_string(),
                role: DeploymentRole::Ingest,
                target: DeploymentTarget::Azure,
                sources: Some(vec![DeploymentSource {
                    source: "jira-cloud".to_string(),
                    projects: None,
                    spaces: None,
                }]),
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
                expose: Some(vec!["jira_issues".to_string()]),
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
        ],
        mcp: McpConfig {
            data_sources: {
                let mut m = HashMap::new();
                m.insert(
                    "jira_issues".to_string(),
                    McpDataSourceSpec {
                        kind: "jira_issue".to_string(),
                        backed_by: vec![BackedBy {
                            container: "jira-issues".to_string(),
                        }],
                    },
                );
                m
            },
            ..McpConfig::default()
        },
        rigg: RiggConfig::default(),
        state: StateConfig::default(),
    }
}

// ---------------------------------------------------------------------------
// Multi-source: Jira + Confluence, both in Azure.
// ---------------------------------------------------------------------------

/// Multi-source template: Jira Cloud + Confluence Cloud, both Azure.
pub fn multi_source_template() -> Config {
    let mut base = minimal_template();

    base.sources
        .push(SourceConfig::Confluence(ConfluenceSourceConfig {
            name: "confluence-cloud".to_string(),
            url: "https://${JIRA_HOST}.atlassian.net/wiki".to_string(),
            auth: AuthConfig::Cloud {
                email: "${JIRA_EMAIL}".to_string(),
                api_token: "${JIRA_API_TOKEN}".to_string(),
            },
            spaces: vec!["ENG".to_string()],
            container: None,
            companion_containers: CompanionContainersConfig::default(),
        }));

    // Add confluence source to the ingest deployment.
    if let Some(dep) = base.deployments.iter_mut().find(|d| d.name == "ingest")
        && let Some(ref mut sources) = dep.sources
    {
        sources.push(DeploymentSource {
            source: "confluence-cloud".to_string(),
            projects: None,
            spaces: None,
        });
    }

    // Add confluence_pages to the MCP deployment's expose list.
    if let Some(dep) = base.deployments.iter_mut().find(|d| d.name == "mcp")
        && let Some(ref mut expose) = dep.expose
    {
        expose.push("confluence_pages".to_string());
    }

    // Add confluence_pages to MCP data sources.
    base.mcp.data_sources.insert(
        "confluence_pages".to_string(),
        McpDataSourceSpec {
            kind: "confluence_page".to_string(),
            backed_by: vec![BackedBy {
                container: "confluence-pages".to_string(),
            }],
        },
    );

    base
}

// ---------------------------------------------------------------------------
// Distributed: Jira DC on-prem + Confluence Cloud in Azure.
// ---------------------------------------------------------------------------

/// Distributed template: Jira Data Center on-prem + Confluence Cloud in Azure.
pub fn distributed_template() -> Config {
    let mut base = multi_source_template();

    // Replace the Jira source with a Data Center source.
    base.sources[0] = SourceConfig::Jira(JiraSourceConfig {
        name: "jira-dc".to_string(),
        url: "https://jira.internal.${INTERNAL_DOMAIN}".to_string(),
        auth: AuthConfig::DataCenter {
            pat: "${JIRA_DC_PAT}".to_string(),
        },
        projects: vec!["PROJ".to_string()],
        container: None,
        companion_containers: CompanionContainersConfig::default(),
        fields: HashMap::new(),
    });

    // Add an on-prem ingest deployment for DC Jira.
    base.deployments.insert(
        0,
        DeploymentConfig {
            name: "ingest-onprem".to_string(),
            role: DeploymentRole::Ingest,
            target: DeploymentTarget::Onprem,
            sources: Some(vec![DeploymentSource {
                source: "jira-dc".to_string(),
                projects: None,
                spaces: None,
            }]),
            expose: None,
            azure: None,
            auth: None,
        },
    );

    // Update the Azure ingest to only use Confluence.
    if let Some(dep) = base.deployments.iter_mut().find(|d| d.name == "ingest") {
        dep.sources = Some(vec![DeploymentSource {
            source: "confluence-cloud".to_string(),
            projects: None,
            spaces: None,
        }]);
    }

    // Update ingest source reference to use jira-dc.
    if let Some(dep) = base
        .deployments
        .iter_mut()
        .find(|d| d.name == "ingest-onprem")
        && let Some(ref mut srcs) = dep.sources
        && let Some(s) = srcs.first_mut()
    {
        s.source = "jira-dc".to_string();
    }

    base
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::validate;

    #[test]
    fn template_for_minimal_returns_valid_config() {
        let cfg = template_for("minimal").unwrap();
        // The template has env var placeholders, but structural validation
        // (which doesn't resolve env vars) should still pass.
        validate::run(&cfg).expect("minimal template must pass structural validation");
    }

    #[test]
    fn template_for_multi_source_returns_valid_config() {
        let cfg = template_for("multi-source").unwrap();
        validate::run(&cfg).expect("multi-source template must pass structural validation");
    }

    #[test]
    fn template_for_distributed_returns_valid_config() {
        let cfg = template_for("distributed").unwrap();
        validate::run(&cfg).expect("distributed template must pass structural validation");
    }

    #[test]
    fn template_for_unknown_returns_error() {
        let err = template_for("bogus").unwrap_err();
        assert!(
            err.to_string().contains("bogus"),
            "error must mention the unknown name"
        );
    }

    #[test]
    fn minimal_template_has_one_jira_source() {
        let cfg = minimal_template();
        assert_eq!(cfg.sources.len(), 1);
        assert!(matches!(cfg.sources[0], SourceConfig::Jira(_)));
    }

    #[test]
    fn minimal_template_has_ingest_and_mcp_deployments() {
        let cfg = minimal_template();
        assert!(cfg.deployments.iter().any(|d| d.name == "ingest"));
        assert!(cfg.deployments.iter().any(|d| d.name == "mcp"));
    }

    #[test]
    fn distributed_template_has_onprem_deployment() {
        let cfg = distributed_template();
        assert!(
            cfg.deployments
                .iter()
                .any(|d| matches!(d.target, DeploymentTarget::Onprem))
        );
    }

    #[test]
    fn minimal_template_serializes_to_valid_yaml() {
        let cfg = minimal_template();
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        let reparsed: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(reparsed.sources.len(), cfg.sources.len());
    }
}
