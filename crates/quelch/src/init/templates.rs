//! Built-in config templates for `quelch init --non-interactive --from-template`.
//!
//! Templates use `${ENV_VAR}` placeholders for credentials — the user fills
//! them in their shell or in a `.env` file before running `quelch validate`,
//! `quelch ingest`, etc.
//!
//! All templates are designed to round-trip: `serde_yaml::to_string` →
//! `serde_yaml::from_str` → [`crate::config::validate::validate`]. The unit
//! tests below exercise that pipeline for every template.

use std::time::Duration;

use crate::config::schema::{
    AiChat, AiConfig, AiEmbedding, AiProvider, AzureConfig, Config, ContainerLayout, CosmosConfig,
    IngestInstance, InstanceConfig, InstanceSpec, McpInstance, SearchConfig, SourceAuth,
    SourceConnection, SourceType,
};

/// Return a built-in template config by name.
///
/// # Errors
/// Returns an error if `name` is not one of the known templates.
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

/// Names of all built-in templates, in canonical order.
pub const BUILTIN_TEMPLATES: &[&str] = &["minimal", "multi-source", "distributed"];

// ---------------------------------------------------------------------------
// Minimal: one Jira Cloud source connection, one ingest + one MCP instance.
// ---------------------------------------------------------------------------

/// Minimal template: a single Jira source feeding a single ingest instance,
/// plus one MCP instance exposing `jira_issues`.
pub fn minimal_template() -> Config {
    Config {
        azure: AzureConfig {
            cosmos: CosmosConfig {
                subscription_id: Some("${AZURE_SUBSCRIPTION_ID}".to_string()),
                resource_group: Some("rg-quelch-prod".to_string()),
                account: Some("my-cosmos".to_string()),
                endpoint: "https://my-cosmos.documents.azure.com".to_string(),
                database: "quelch".to_string(),
                containers: ContainerLayout::default(),
                meta_container: "quelch-meta".to_string(),
            },
            search: Some(SearchConfig {
                endpoint: "https://my-search.search.windows.net".to_string(),
            }),
            ai: Some(AiConfig {
                provider: AiProvider::Foundry,
                endpoint: "https://my-foundry.cognitiveservices.azure.com".to_string(),
                embedding: AiEmbedding {
                    deployment: "text-embedding-3-large".to_string(),
                    dimensions: 3072,
                },
                chat: AiChat {
                    deployment: "gpt-5-mini".to_string(),
                    model_name: "gpt-5-mini".to_string(),
                },
            }),
        },
        source_connections: vec![SourceConnection {
            name: "jira-cloud".to_string(),
            source_type: SourceType::Jira,
            base_url: "https://your-org.atlassian.net".to_string(),
            auth: SourceAuth::Basic {
                email: "${JIRA_EMAIL}".to_string(),
                token: "${JIRA_API_TOKEN}".to_string(),
            },
            projects: vec!["PROJ".to_string()],
            spaces: vec![],
        }],
        instances: vec![
            InstanceConfig {
                name: "ingest-jira".to_string(),
                spec: InstanceSpec::Ingest(IngestInstance {
                    connections: vec!["jira-cloud".to_string()],
                    cycle_interval: Duration::from_secs(5 * 60),
                }),
            },
            InstanceConfig {
                name: "mcp-prod".to_string(),
                spec: InstanceSpec::Mcp(McpInstance {
                    expose: vec!["jira_issues".to_string()],
                    api_key: "${QUELCH_MCP_API_KEY}".to_string(),
                    knowledge_base: "quelch-prod-kb".to_string(),
                    listen: "0.0.0.0:8080".to_string(),
                }),
            },
        ],
    }
}

// ---------------------------------------------------------------------------
// Multi-source: Jira + Confluence, single combined ingest, MCP exposes both.
// ---------------------------------------------------------------------------

/// Multi-source template: Jira Cloud + Confluence Cloud, both ingested by a
/// single ingest instance, MCP exposes pages and issues.
pub fn multi_source_template() -> Config {
    let mut base = minimal_template();

    base.source_connections.push(SourceConnection {
        name: "confluence-cloud".to_string(),
        source_type: SourceType::Confluence,
        base_url: "https://your-org.atlassian.net/wiki".to_string(),
        auth: SourceAuth::Basic {
            email: "${JIRA_EMAIL}".to_string(),
            token: "${JIRA_API_TOKEN}".to_string(),
        },
        projects: vec![],
        spaces: vec!["ENG".to_string()],
    });

    // Replace the single-connection ingest with one that uses both.
    if let Some(inst) = base.instances.iter_mut().find(|i| i.name == "ingest-jira")
        && let InstanceSpec::Ingest(ref mut spec) = inst.spec
    {
        spec.connections.push("confluence-cloud".to_string());
    }

    // Expose confluence_pages on the MCP instance too.
    if let Some(inst) = base.instances.iter_mut().find(|i| i.name == "mcp-prod")
        && let InstanceSpec::Mcp(ref mut spec) = inst.spec
    {
        spec.expose.push("confluence_pages".to_string());
    }

    base
}

// ---------------------------------------------------------------------------
// Distributed: separate ingest instances for Jira (Data Center) and
// Confluence (Cloud). The user can run them on different hosts.
// ---------------------------------------------------------------------------

/// Distributed template: a Data Center Jira and a Cloud Confluence, each
/// ingested by its own instance (so they can be hosted in different network
/// zones — typically a Data Center Jira on-prem and a Cloud Confluence
/// alongside Q-MCP in Azure).
pub fn distributed_template() -> Config {
    let mut base = minimal_template();

    // Replace the single Jira Cloud connection with a Jira Data Center
    // (PAT-only) connection.
    base.source_connections[0] = SourceConnection {
        name: "jira-dc".to_string(),
        source_type: SourceType::Jira,
        base_url: "https://jira.internal.example".to_string(),
        auth: SourceAuth::Pat {
            token: "${JIRA_DC_PAT}".to_string(),
        },
        projects: vec!["PROJ".to_string()],
        spaces: vec![],
    };

    // Add a Cloud Confluence as a second connection.
    base.source_connections.push(SourceConnection {
        name: "confluence-cloud".to_string(),
        source_type: SourceType::Confluence,
        base_url: "https://your-org.atlassian.net/wiki".to_string(),
        auth: SourceAuth::Basic {
            email: "${JIRA_EMAIL}".to_string(),
            token: "${JIRA_API_TOKEN}".to_string(),
        },
        projects: vec![],
        spaces: vec!["ENG".to_string()],
    });

    // Replace the single ingest with two — one per source.
    base.instances.retain(|i| i.name != "ingest-jira");
    base.instances.insert(
        0,
        InstanceConfig {
            name: "ingest-jira-dc".to_string(),
            spec: InstanceSpec::Ingest(IngestInstance {
                connections: vec!["jira-dc".to_string()],
                cycle_interval: Duration::from_secs(5 * 60),
            }),
        },
    );
    base.instances.insert(
        1,
        InstanceConfig {
            name: "ingest-confluence-cloud".to_string(),
            spec: InstanceSpec::Ingest(IngestInstance {
                connections: vec!["confluence-cloud".to_string()],
                cycle_interval: Duration::from_secs(10 * 60),
            }),
        },
    );

    // Expose confluence_pages on the MCP instance too.
    if let Some(inst) = base.instances.iter_mut().find(|i| i.name == "mcp-prod")
        && let InstanceSpec::Mcp(ref mut spec) = inst.spec
    {
        spec.expose.push("confluence_pages".to_string());
    }

    base
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::validate::validate;

    fn round_trip(cfg: &Config) -> Config {
        let yaml = serde_yaml::to_string(cfg).expect("template serializes");
        serde_yaml::from_str(&yaml).expect("template re-parses")
    }

    #[test]
    fn minimal_template_round_trips_and_validates() {
        let cfg = minimal_template();
        let again = round_trip(&cfg);
        validate(&again).expect("minimal template validates");
        assert_eq!(again.source_connections.len(), 1);
        assert_eq!(again.instances.len(), 2);
    }

    #[test]
    fn multi_source_template_round_trips_and_validates() {
        let cfg = multi_source_template();
        let again = round_trip(&cfg);
        validate(&again).expect("multi-source template validates");
        assert_eq!(again.source_connections.len(), 2);
        // Combined ingest instance still references both.
        let ingest = again
            .instances
            .iter()
            .find(|i| i.name == "ingest-jira")
            .expect("ingest-jira present");
        let connections = match &ingest.spec {
            InstanceSpec::Ingest(s) => &s.connections,
            _ => panic!("not an ingest"),
        };
        assert_eq!(connections.len(), 2);
    }

    #[test]
    fn distributed_template_round_trips_and_validates() {
        let cfg = distributed_template();
        let again = round_trip(&cfg);
        validate(&again).expect("distributed template validates");
        assert_eq!(again.source_connections.len(), 2);
        // Two ingest instances (one per source) plus one MCP.
        let ingest_count = again
            .instances
            .iter()
            .filter(|i| matches!(i.spec, InstanceSpec::Ingest(_)))
            .count();
        assert_eq!(ingest_count, 2);
    }

    #[test]
    fn template_for_returns_known_templates() {
        for name in BUILTIN_TEMPLATES {
            template_for(name).unwrap_or_else(|_| panic!("template '{name}' missing"));
        }
    }

    #[test]
    fn template_for_unknown_returns_error() {
        let err = template_for("does-not-exist").unwrap_err();
        assert!(err.to_string().contains("does-not-exist"));
    }
}
