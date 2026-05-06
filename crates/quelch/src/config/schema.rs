//! Strongly-typed YAML schema for `quelch.yaml`.
//!
//! All public types in this module participate in (de)serialisation. Field
//! defaults and `skip_serializing_if` keep the on-disk representation tidy:
//! e.g. an unset `account` is omitted rather than rendered as `account: null`.
//!
//! See [`Config`] for the entry point. The wider validation pipeline lives in
//! [`super::validate`], and the per-instance slice helper in [`super::slice`].

use serde::{Deserialize, Serialize};

/// Top-level Quelch configuration.
///
/// One file describes the whole deployment: which Azure resources to talk to,
/// which external sources to ingest from, and which Quelch processes
/// (instances) operate on what.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Azure resources Quelch uses (Cosmos DB, AI Search, AI provider).
    pub azure: AzureConfig,
    /// Named external-source connections (Jira, Confluence) that ingest
    /// instances pull from.
    pub source_connections: Vec<SourceConnection>,
    /// Quelch processes (Q-Ingest, Q-MCP) declared by this config.
    pub instances: Vec<InstanceConfig>,
}

/// Azure-side dependencies. Quelch references these but does not provision
/// them — the operator creates them up front.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AzureConfig {
    /// Cosmos DB account that holds all ingested documents and cursor state.
    pub cosmos: CosmosConfig,
    /// Azure AI Search service that indexes Cosmos. Required for MCP
    /// instances; optional for ingest-only setups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<SearchConfig>,
    /// AI model provider (Foundry or Azure OpenAI) for embedding + chat.
    /// Required by `quelch azure apply` to author the AI Search KB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<AiConfig>,
}

/// Cosmos DB account, database, and container layout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CosmosConfig {
    /// Azure subscription ID containing the account. Optional — only needed
    /// when `quelch validate` / `quelch azure apply` shell out to `az`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_id: Option<String>,
    /// Resource group containing the account. Optional — see
    /// `subscription_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_group: Option<String>,
    /// Account name (the `*.documents.azure.com` host stem). Optional —
    /// inferable from `endpoint` for most checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Cosmos document endpoint, e.g. `https://acct.documents.azure.com`.
    pub endpoint: String,
    /// Database name within the account (default convention: `quelch`).
    pub database: String,
    /// Optional container-name overrides. Each unset name falls back to the
    /// canonical default in [`crate::cosmos::CosmosClient`].
    #[serde(default)]
    pub containers: ContainerLayout,
    /// Container that stores cursor + heartbeat state shared by all
    /// instances. Defaults to `quelch-meta`.
    #[serde(default = "default_meta_container")]
    pub meta_container: String,
}

fn default_meta_container() -> String {
    "quelch-meta".to_string()
}

/// Per-data-source container-name overrides.
///
/// Any field left `None` falls back to the canonical default
/// (`jira-issues`, `confluence-pages`, etc.).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ContainerLayout {
    /// Override for the `jira_issues` container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira_issues: Option<String>,
    /// Override for the `jira_sprints` container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira_sprints: Option<String>,
    /// Override for the `jira_fix_versions` container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira_fix_versions: Option<String>,
    /// Override for the `jira_projects` container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira_projects: Option<String>,
    /// Override for the `confluence_pages` container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confluence_pages: Option<String>,
    /// Override for the `confluence_spaces` container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confluence_spaces: Option<String>,
}

/// Azure AI Search service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SearchConfig {
    /// Endpoint URL, e.g. `https://srv.search.windows.net`.
    pub endpoint: String,
}

/// AI model provider used by the AI Search KB and skillset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiConfig {
    /// Which provider this AI account is.
    pub provider: AiProvider,
    /// Provider endpoint URL.
    pub endpoint: String,
    /// Embedding deployment used by the skillset / vectorizer.
    pub embedding: AiEmbedding,
    /// Chat / LLM deployment used by the KB for query planning + answer
    /// synthesis.
    pub chat: AiChat,
}

/// AI model provider kind.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    /// Microsoft Foundry project (Cognitive Services, kind=AIServices).
    Foundry,
    /// Azure OpenAI account (Cognitive Services, kind=OpenAI).
    AzureOpenai,
}

/// Embedding model deployment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiEmbedding {
    /// Deployment name (the user-chosen alias inside the Foundry / OpenAI
    /// account).
    pub deployment: String,
    /// Output dimensions of the embedding model.
    pub dimensions: u32,
}

/// Chat / LLM model deployment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiChat {
    /// Deployment name.
    pub deployment: String,
    /// Underlying model identifier (e.g. `gpt-5-mini`). Required by AI
    /// Search agentic retrieval.
    pub model_name: String,
}

/// One named external source (Jira / Confluence) Quelch ingests from.
///
/// Each connection bundles `(base_url, credential, projects/spaces)` so that
/// instances can reference it by `name` from `instances[].connections`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SourceConnection {
    /// Connection name, referenced from `instances[].connections`.
    pub name: String,
    /// Source kind (Jira or Confluence).
    #[serde(rename = "type")]
    pub source_type: SourceType,
    /// Base URL of the source instance.
    pub base_url: String,
    /// Authentication credential.
    pub auth: SourceAuth,
    /// Jira project keys to ingest. Required for Jira connections; ignored
    /// for Confluence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<String>,
    /// Confluence space keys to ingest. Required for Confluence connections;
    /// ignored for Jira.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spaces: Vec<String>,
}

/// External-source kind.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    /// Atlassian Jira (Cloud or Data Center).
    Jira,
    /// Atlassian Confluence (Cloud or Data Center).
    Confluence,
}

/// Source-side authentication credential.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceAuth {
    /// Personal Access Token bearer auth (Data Center / Server flavour).
    Pat {
        /// PAT value or `${ENV_VAR}` placeholder.
        token: String,
    },
    /// HTTP Basic auth with email + API token (Atlassian Cloud flavour).
    Basic {
        /// Atlassian account email.
        email: String,
        /// API token value or `${ENV_VAR}` placeholder.
        token: String,
    },
}

impl SourceAuth {
    /// Render this credential as the `Authorization:` header value to send
    /// with HTTP requests to the source.
    pub fn authorization_header(&self) -> String {
        use base64::Engine;
        match self {
            SourceAuth::Pat { token } => format!("Bearer {token}"),
            SourceAuth::Basic { email, token } => {
                let credentials = format!("{email}:{token}");
                let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
                format!("Basic {encoded}")
            }
        }
    }
}

/// One declared Quelch process (Q-Ingest or Q-MCP).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstanceConfig {
    /// Instance name. Used as a CLI argument (`--instance <name>`) and as the
    /// owner identifier on Cosmos cursors.
    pub name: String,
    /// The kind-specific spec (ingest vs. mcp).
    #[serde(flatten)]
    pub spec: InstanceSpec,
}

impl InstanceConfig {
    /// Convenience accessor for the kind discriminator.
    pub fn kind(&self) -> InstanceKind {
        match self.spec {
            InstanceSpec::Ingest(_) => InstanceKind::Ingest,
            InstanceSpec::Mcp(_) => InstanceKind::Mcp,
        }
    }
}

/// Discriminator for the two instance flavours.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstanceKind {
    /// Q-Ingest: pulls from sources, writes to Cosmos.
    Ingest,
    /// Q-MCP: serves the agent-facing MCP API.
    Mcp,
}

/// Per-kind instance spec, tagged on the YAML by `kind:`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstanceSpec {
    /// Ingest worker spec.
    Ingest(IngestInstance),
    /// MCP server spec.
    Mcp(McpInstance),
}

/// Ingest-instance configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct IngestInstance {
    /// Names of the source connections this instance pulls from.
    pub connections: Vec<String>,
    /// How often to run a full ingest cycle (humantime — e.g. `5m`, `1h`).
    #[serde(with = "humantime_serde")]
    pub cycle_interval: std::time::Duration,
}

/// MCP-instance configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct McpInstance {
    /// Logical data-source names to expose through the MCP API
    /// (e.g. `jira_issues`, `confluence_pages`).
    pub expose: Vec<String>,
    /// Static API key clients must present, or `${ENV_VAR}` placeholder.
    pub api_key: String,
    /// AI Search Knowledge Base name to back agentic retrieval.
    pub knowledge_base: String,
    /// Listen address (`host:port`) for the MCP server.
    pub listen: String,
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_master_yaml_with_instances_and_connections() {
        let yaml = r#"
azure:
  cosmos:
    subscription_id: "00000000-0000-0000-0000-000000000000"
    resource_group: rg-quelch
    account: acct
    endpoint: https://acct.documents.azure.com
    database: quelch
    containers:
      jira_issues: jira-issues
      jira_sprints: jira-sprints
      jira_fix_versions: jira-fix-versions
      jira_projects: jira-projects
      confluence_pages: confluence-pages
      confluence_spaces: confluence-spaces
    meta_container: quelch-meta
  search:
    endpoint: https://srv.search.windows.net
  ai:
    provider: foundry
    endpoint: https://ai.example
    embedding: { deployment: text-embedding-3-large, dimensions: 3072 }
    chat: { deployment: gpt-5-mini, model_name: gpt-5-mini }

source_connections:
  - name: jira-x
    type: jira
    base_url: https://jira.internal/
    auth: { kind: pat, token: "T1" }
    projects: [DO]
  - name: jira-y
    type: jira
    base_url: https://jira.internal/
    auth: { kind: pat, token: "T2" }
    projects: [EMMA]

instances:
  - name: ingest-internal
    kind: ingest
    connections: [jira-x, jira-y]
    cycle_interval: 5m
  - name: mcp-prod
    kind: mcp
    expose: [jira_issues]
    api_key: "K"
    knowledge_base: kb
    listen: 0.0.0.0:8080
"#;
        let cfg: super::Config = serde_yaml::from_str(yaml).expect("parses");
        assert_eq!(cfg.source_connections.len(), 2);
        assert_eq!(cfg.instances.len(), 2);
        let ingest = cfg
            .instances
            .iter()
            .find(|i| i.name == "ingest-internal")
            .unwrap();
        assert!(matches!(ingest.kind(), super::InstanceKind::Ingest));
        let connections = match &ingest.spec {
            super::InstanceSpec::Ingest(i) => &i.connections,
            _ => panic!("wrong variant"),
        };
        assert_eq!(
            connections,
            &vec!["jira-x".to_string(), "jira-y".to_string()]
        );
    }
}
