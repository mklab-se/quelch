/// V2 configuration schema for Quelch.
///
/// Every field corresponds to a section or sub-field documented in
/// `docs/configuration.md`. The structs are plain data containers;
/// validation logic lives in `validate.rs`.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Top-level config
// ---------------------------------------------------------------------------

/// Root configuration loaded from `quelch.yaml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub azure: AzureConfig,
    #[serde(default)]
    pub cosmos: CosmosConfig,
    #[serde(default)]
    pub search: SearchConfig,
    pub ai: AiConfig,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    #[serde(default)]
    pub ingest: IngestConfig,
    #[serde(default)]
    pub deployments: Vec<DeploymentConfig>,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub rigg: RiggConfig,
    #[serde(default)]
    pub state: StateConfig,
}

// ---------------------------------------------------------------------------
// Azure
// ---------------------------------------------------------------------------

impl Config {
    /// Return the effective resource group for the Cosmos DB account.
    /// Falls back to `azure.resource_group` if no override is set.
    pub fn cosmos_resource_group(&self) -> &str {
        self.cosmos
            .account_resource_group
            .as_deref()
            .unwrap_or(&self.azure.resource_group)
    }

    /// Return the effective resource group for the AI Search service.
    pub fn search_resource_group(&self) -> &str {
        self.search
            .service_resource_group
            .as_deref()
            .unwrap_or(&self.azure.resource_group)
    }

    /// Return the effective resource group for the AI provider (Foundry
    /// project / Azure OpenAI account).
    pub fn ai_resource_group(&self) -> &str {
        self.ai
            .resource_group
            .as_deref()
            .unwrap_or(&self.azure.resource_group)
    }

    /// Return the effective resource group for the Container Apps environment.
    pub fn container_apps_env_resource_group(&self) -> &str {
        self.azure
            .resources
            .container_apps_env_resource_group
            .as_deref()
            .unwrap_or(&self.azure.resource_group)
    }

    /// Return the effective resource group for the Application Insights component.
    pub fn application_insights_resource_group(&self) -> &str {
        self.azure
            .resources
            .application_insights_resource_group
            .as_deref()
            .unwrap_or(&self.azure.resource_group)
    }

    /// Return the effective resource group for the Key Vault.
    pub fn key_vault_resource_group(&self) -> &str {
        self.azure
            .resources
            .key_vault_resource_group
            .as_deref()
            .unwrap_or(&self.azure.resource_group)
    }
}

/// Azure subscription, resource group, region, and resource naming config.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AzureConfig {
    pub subscription_id: String,
    pub resource_group: String,
    pub region: String,
    #[serde(default)]
    pub naming: NamingConfig,
    #[serde(default)]
    pub skip_role_assignments: bool,
    /// Names of the existing Azure resources Quelch references but does not
    /// provision (Container Apps environment, App Insights, Key Vault).
    /// Cosmos account and AI Search service have their own dedicated config
    /// blocks (`cosmos.account` / `search.service`).
    #[serde(default)]
    pub resources: AzureExistingResources,
}

/// References to pre-existing Azure resources that Quelch needs to bind to
/// but does not provision itself. Each field defaults to a name derived from
/// `naming.prefix` + `naming.environment` if left unset.
///
/// Each resource also has an optional `_resource_group` sibling that
/// overrides [`AzureConfig::resource_group`] for that resource — useful when
/// shared resources (e.g. a Foundry project owned by another team) live in
/// a different resource group than the rest of the deployment.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AzureExistingResources {
    pub container_apps_env: Option<String>,
    pub container_apps_env_resource_group: Option<String>,
    pub application_insights: Option<String>,
    pub application_insights_resource_group: Option<String>,
    pub key_vault: Option<String>,
    pub key_vault_resource_group: Option<String>,
}

/// Resource naming prefixes used when Quelch auto-generates Azure resource names.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct NamingConfig {
    pub prefix: Option<String>,
    pub environment: Option<String>,
}

// ---------------------------------------------------------------------------
// Cosmos DB
// ---------------------------------------------------------------------------

/// Cosmos DB account, database, container layout, and throughput config.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CosmosConfig {
    pub account: Option<String>,
    /// Override `azure.resource_group` for the Cosmos account lookup.
    /// Useful when the Cosmos account is shared across teams and lives
    /// in a separate resource group.
    pub account_resource_group: Option<String>,
    #[serde(default = "default_cosmos_database")]
    pub database: String,
    #[serde(default)]
    pub containers: CosmosContainersDefaults,
    #[serde(default = "default_meta_container")]
    pub meta_container: String,
    #[serde(default)]
    pub throughput: CosmosThroughput,
}

impl Default for CosmosConfig {
    fn default() -> Self {
        Self {
            account: None,
            account_resource_group: None,
            database: default_cosmos_database(),
            containers: CosmosContainersDefaults::default(),
            meta_container: default_meta_container(),
            throughput: CosmosThroughput::default(),
        }
    }
}

fn default_cosmos_database() -> String {
    "quelch".to_string()
}

fn default_meta_container() -> String {
    "quelch-meta".to_string()
}

/// Default Cosmos container names for each source-type entity class.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CosmosContainersDefaults {
    #[serde(default = "default_jira_issues")]
    pub jira_issues: String,
    #[serde(default = "default_confluence_pages")]
    pub confluence_pages: String,
    #[serde(default = "default_jira_sprints")]
    pub jira_sprints: String,
    #[serde(default = "default_jira_fix_versions")]
    pub jira_fix_versions: String,
    #[serde(default = "default_jira_projects")]
    pub jira_projects: String,
    #[serde(default = "default_confluence_spaces")]
    pub confluence_spaces: String,
}

impl Default for CosmosContainersDefaults {
    fn default() -> Self {
        Self {
            jira_issues: default_jira_issues(),
            confluence_pages: default_confluence_pages(),
            jira_sprints: default_jira_sprints(),
            jira_fix_versions: default_jira_fix_versions(),
            jira_projects: default_jira_projects(),
            confluence_spaces: default_confluence_spaces(),
        }
    }
}

fn default_jira_issues() -> String {
    "jira-issues".to_string()
}
fn default_confluence_pages() -> String {
    "confluence-pages".to_string()
}
fn default_jira_sprints() -> String {
    "jira-sprints".to_string()
}
fn default_jira_fix_versions() -> String {
    "jira-fix-versions".to_string()
}
fn default_jira_projects() -> String {
    "jira-projects".to_string()
}
fn default_confluence_spaces() -> String {
    "confluence-spaces".to_string()
}

/// Cosmos throughput mode and provisioned RU/s.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CosmosThroughput {
    #[serde(default = "default_throughput_mode")]
    pub mode: String,
    pub ru_per_second: Option<u32>,
}

impl Default for CosmosThroughput {
    fn default() -> Self {
        Self {
            mode: default_throughput_mode(),
            ru_per_second: None,
        }
    }
}

fn default_throughput_mode() -> String {
    "serverless".to_string()
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// Azure AI Search service configuration (name, SKU, indexer schedule).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchConfig {
    pub service: Option<String>,
    /// Override `azure.resource_group` for the AI Search service lookup.
    pub service_resource_group: Option<String>,
    #[serde(default = "default_search_sku")]
    pub sku: String,
    #[serde(default)]
    pub indexer: IndexerConfig,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            service: None,
            service_resource_group: None,
            sku: default_search_sku(),
            indexer: IndexerConfig::default(),
        }
    }
}

fn default_search_sku() -> String {
    "basic".to_string()
}

/// Indexer-level configuration (schedule and high-water-mark field).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IndexerConfig {
    #[serde(default)]
    pub schedule: IndexerSchedule,
    #[serde(default = "default_hwm_field")]
    pub high_water_mark_field: String,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            schedule: IndexerSchedule::default(),
            high_water_mark_field: default_hwm_field(),
        }
    }
}

fn default_hwm_field() -> String {
    "updated".to_string()
}

/// ISO 8601 duration string controlling how often the indexer runs.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IndexerSchedule {
    #[serde(default = "default_indexer_interval")]
    pub interval: String,
}

impl Default for IndexerSchedule {
    fn default() -> Self {
        Self {
            interval: default_indexer_interval(),
        }
    }
}

fn default_indexer_interval() -> String {
    "PT15M".to_string()
}

// ---------------------------------------------------------------------------
// AI provider (embedding + chat models)
// ---------------------------------------------------------------------------

/// Reference to an existing AI model provider — either an Azure OpenAI account
/// or a Microsoft Foundry project — that hosts both the embedding deployment
/// (used by the AI Search vectorizer / skillset) and the chat-completion
/// deployment (used by the Knowledge Base for query planning and answer
/// synthesis).
///
/// The on-the-wire JSON shape Azure AI Search emits is identical for both
/// providers; the `provider` field exists so `quelch init` knows which `az`
/// command surface to query when discovering deployments.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AiConfig {
    pub provider: AiProvider,
    pub endpoint: String,
    /// Override `azure.resource_group` for the AI provider lookup. Common in
    /// enterprise setups where a single Foundry project / Azure OpenAI
    /// account is shared across many workloads in a separate resource group.
    pub resource_group: Option<String>,
    pub embedding: AiEmbeddingConfig,
    pub chat: AiChatConfig,
}

/// Which Azure surface holds the model deployments referenced by [`AiConfig`].
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    /// Classic Azure OpenAI account (`Microsoft.CognitiveServices/accounts`
    /// with `kind=OpenAI`).
    AzureOpenai,
    /// Microsoft Foundry project (`Microsoft.MachineLearningServices/workspaces`
    /// with `kind=Project`).
    Foundry,
}

/// Embedding deployment that the AI Search vectorizer / skillset will call.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AiEmbeddingConfig {
    pub deployment: String,
    pub dimensions: u32,
}

/// Chat-completion deployment that the Knowledge Base uses for agentic
/// retrieval (query planning + optional answer synthesis).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AiChatConfig {
    pub deployment: String,
    pub model_name: String,
    #[serde(default)]
    pub retrieval_reasoning_effort: ReasoningEffort,
    #[serde(default)]
    pub output_mode: OutputMode,
}

/// `retrievalReasoningEffort.kind` for the Knowledge Base.
///
/// `Minimal` skips the LLM (vector + keyword + semantic only). `Low` is the
/// portal default. `Medium` enables follow-up subqueries.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Minimal,
    #[default]
    Low,
    Medium,
}

/// Knowledge Base `outputMode`.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum OutputMode {
    /// LLM-formulated natural-language answer with citations. Portal default.
    #[serde(rename = "answerSynthesis")]
    #[default]
    AnswerSynthesis,
    /// Raw ranked search results, no LLM-side composition.
    #[serde(rename = "extractedData")]
    ExtractedData,
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

/// A source instance — either Jira or Confluence. Uses `type:` tag to
/// discriminate, following v1 convention.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum SourceConfig {
    #[serde(rename = "jira")]
    Jira(JiraSourceConfig),
    #[serde(rename = "confluence")]
    Confluence(ConfluenceSourceConfig),
}

impl SourceConfig {
    /// Returns the unique name of this source instance.
    pub fn name(&self) -> &str {
        match self {
            SourceConfig::Jira(j) => &j.name,
            SourceConfig::Confluence(c) => &c.name,
        }
    }
}

/// Jira source instance configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JiraSourceConfig {
    pub name: String,
    pub url: String,
    pub auth: AuthConfig,
    pub projects: Vec<String>,
    pub container: Option<String>,
    #[serde(default)]
    pub companion_containers: CompanionContainersConfig,
    #[serde(default)]
    pub fields: HashMap<String, String>,
}

/// Confluence source instance configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConfluenceSourceConfig {
    pub name: String,
    pub url: String,
    pub auth: AuthConfig,
    pub spaces: Vec<String>,
    pub container: Option<String>,
    #[serde(default)]
    pub companion_containers: CompanionContainersConfig,
}

/// Auth for either Cloud (email + api_token) or Data Center (pat).
///
/// Untagged — serde tries Cloud first (requires both `email` and `api_token`),
/// then DataCenter (requires `pat`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AuthConfig {
    /// Atlassian Cloud: Basic auth with email and API token.
    Cloud { email: String, api_token: String },
    /// On-premises Data Center: PAT Bearer auth.
    DataCenter { pat: String },
}

impl AuthConfig {
    /// Build the Authorization header value for this auth config.
    ///
    /// Used by source connectors (`sources/jira.rs`, `sources/confluence.rs`).
    pub fn authorization_header(&self) -> String {
        use base64::Engine;
        match self {
            AuthConfig::Cloud { email, api_token } => {
                let credentials = format!("{email}:{api_token}");
                let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
                format!("Basic {encoded}")
            }
            AuthConfig::DataCenter { pat } => {
                format!("Bearer {pat}")
            }
        }
    }

    /// Returns `true` if this is a Cloud (email + api_token) auth config.
    pub fn is_cloud(&self) -> bool {
        matches!(self, AuthConfig::Cloud { .. })
    }
}

/// Per-source container name overrides for companion collections.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CompanionContainersConfig {
    pub sprints: Option<String>,
    pub fix_versions: Option<String>,
    pub projects: Option<String>,
    pub spaces: Option<String>,
}

// ---------------------------------------------------------------------------
// Ingest
// ---------------------------------------------------------------------------

/// Global ingest worker behaviour knobs.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IngestConfig {
    #[serde(default = "default_poll_interval")]
    pub poll_interval: String,
    #[serde(default = "default_safety_lag_minutes")]
    pub safety_lag_minutes: u32,
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
    #[serde(default = "default_reconcile_every")]
    pub reconcile_every: u32,
    #[serde(default = "default_max_cycle_duration")]
    pub max_cycle_duration: String,
    #[serde(default = "default_max_concurrent_per_source")]
    pub max_concurrent_per_source: u32,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            poll_interval: default_poll_interval(),
            safety_lag_minutes: default_safety_lag_minutes(),
            batch_size: default_batch_size(),
            reconcile_every: default_reconcile_every(),
            max_cycle_duration: default_max_cycle_duration(),
            max_concurrent_per_source: default_max_concurrent_per_source(),
            max_retries: default_max_retries(),
        }
    }
}

fn default_poll_interval() -> String {
    "300s".to_string()
}
fn default_safety_lag_minutes() -> u32 {
    2
}
fn default_batch_size() -> u32 {
    100
}
fn default_reconcile_every() -> u32 {
    12
}
fn default_max_cycle_duration() -> String {
    "30m".to_string()
}
fn default_max_concurrent_per_source() -> u32 {
    1
}
fn default_max_retries() -> u32 {
    5
}

// ---------------------------------------------------------------------------
// Deployments
// ---------------------------------------------------------------------------

/// A named deployment — either an ingest worker or an MCP server instance.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeploymentConfig {
    pub name: String,
    pub role: DeploymentRole,
    pub target: DeploymentTarget,
    pub sources: Option<Vec<DeploymentSource>>,
    pub expose: Option<Vec<String>>,
    pub azure: Option<DeploymentAzureConfig>,
    pub auth: Option<DeploymentAuthConfig>,
}

/// Runtime role of a deployment.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentRole {
    Ingest,
    Mcp,
}

/// Where a deployment runs.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentTarget {
    Azure,
    Onprem,
}

/// A source slice within a deployment, optionally scoped to specific subsources.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeploymentSource {
    pub source: String,
    pub projects: Option<Vec<String>>,
    pub spaces: Option<Vec<String>>,
}

/// Azure-specific deployment configuration (Container App sizing).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeploymentAzureConfig {
    pub container_app: ContainerAppSpec,
}

/// Container App resource sizing and replica counts.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ContainerAppSpec {
    pub cpu: Option<f64>,
    pub memory: Option<String>,
    pub min_replicas: Option<u32>,
    pub max_replicas: Option<u32>,
}

/// Auth mode for an MCP deployment.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeploymentAuthConfig {
    pub mode: McpAuthMode,
}

/// Authentication mode for the MCP server.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAuthMode {
    ApiKey,
    Entra,
}

// ---------------------------------------------------------------------------
// MCP
// ---------------------------------------------------------------------------

/// Global MCP server configuration: logical data sources, search backend,
/// and server-level defaults.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpConfig {
    #[serde(default)]
    pub data_sources: HashMap<String, McpDataSourceSpec>,
    pub search: Option<McpSearchConfig>,
    #[serde(default = "default_mcp_default_top")]
    pub default_top: u32,
    #[serde(default = "default_mcp_max_top")]
    pub max_top: u32,
    #[serde(default = "default_mcp_query_timeout")]
    pub query_timeout: String,
    #[serde(default = "default_mcp_search_timeout")]
    pub search_timeout: String,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            data_sources: HashMap::new(),
            search: None,
            default_top: default_mcp_default_top(),
            max_top: default_mcp_max_top(),
            query_timeout: default_mcp_query_timeout(),
            search_timeout: default_mcp_search_timeout(),
        }
    }
}

fn default_mcp_default_top() -> u32 {
    25
}
fn default_mcp_max_top() -> u32 {
    100
}
fn default_mcp_query_timeout() -> String {
    "30s".to_string()
}
fn default_mcp_search_timeout() -> String {
    "20s".to_string()
}

/// A logical MCP data source backed by one or more physical Cosmos containers.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpDataSourceSpec {
    pub kind: String,
    pub backed_by: Vec<BackedBy>,
}

/// A physical Cosmos container backing a logical data source.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct BackedBy {
    pub container: String,
}

/// MCP `search` tool backend options.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct McpSearchConfig {
    #[serde(default)]
    pub disable_agentic: bool,
    pub knowledge_base: Option<String>,
}

// ---------------------------------------------------------------------------
// Rigg
// ---------------------------------------------------------------------------

/// Configuration for the embedded rigg library that manages AI Search internals.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RiggConfig {
    #[serde(default = "default_rigg_dir")]
    pub dir: String,
    #[serde(default)]
    pub ownership: RiggOwnership,
}

impl Default for RiggConfig {
    fn default() -> Self {
        Self {
            dir: default_rigg_dir(),
            ownership: RiggOwnership::default(),
        }
    }
}

fn default_rigg_dir() -> String {
    "./rigg".to_string()
}

/// Whether Quelch owns the rigg output directory or the user does.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RiggOwnership {
    #[default]
    Generated,
    ManagedByUser,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Cursor and worker-state storage backend config.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct StateConfig {
    #[serde(default)]
    pub backend: StateBackend,
    pub local_path: Option<String>,
}

/// Which backend stores ingest cursors.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StateBackend {
    #[default]
    Cosmos,
    LocalFile,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let yaml = r#"
azure:
  subscription_id: "sub-123"
  resource_group: "rg-test"
  region: "swedencentral"
cosmos:
  database: "quelch"
search:
  sku: "basic"
ai:
  provider: foundry
  endpoint: "https://test-foundry.cognitiveservices.azure.com"
  embedding:
    deployment: "text-embedding-3-large"
    dimensions: 3072
  chat:
    deployment: "gpt-5-mini"
    model_name: "gpt-5-mini"
sources:
  - type: jira
    name: jira-cloud
    url: "https://example.atlassian.net"
    auth:
      email: "u@example.com"
      api_token: "tok"
    projects: ["DO"]
deployments:
  - name: ingest
    role: ingest
    target: azure
    sources:
      - source: jira-cloud
  - name: mcp
    role: mcp
    target: azure
    expose: ["jira_issues"]
    auth:
      mode: "api_key"
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.azure.region, "swedencentral");
        assert_eq!(config.deployments.len(), 2);
        assert!(matches!(config.ai.provider, AiProvider::Foundry));
        assert_eq!(config.ai.embedding.dimensions, 3072);
        assert_eq!(
            config.ai.chat.retrieval_reasoning_effort,
            ReasoningEffort::Low
        );
    }

    #[test]
    fn parses_full_config() {
        let yaml = r#"
azure:
  subscription_id: "sub-456"
  resource_group: "rg-prod"
  region: "swedencentral"
  naming:
    prefix: "quelch"
    environment: "prod"

cosmos:
  account: "quelch-prod-cosmos"
  database: "quelch"
  containers:
    jira_issues: "jira-issues"
    confluence_pages: "confluence-pages"
    jira_sprints: "jira-sprints"
    jira_fix_versions: "jira-fix-versions"
    jira_projects: "jira-projects"
    confluence_spaces: "confluence-spaces"
  meta_container: "quelch-meta"
  throughput:
    mode: "provisioned"
    ru_per_second: 1000

search:
  service: "quelch-prod-search"
  sku: "standard"
  indexer:
    schedule:
      interval: "PT15M"
    high_water_mark_field: "updated"

ai:
  provider: azure_openai
  endpoint: "https://prod.openai.azure.com"
  embedding:
    deployment: "text-embedding-3-large"
    dimensions: 3072
  chat:
    deployment: "gpt-5-mini"
    model_name: "gpt-5-mini"
    retrieval_reasoning_effort: medium
    output_mode: answerSynthesis

sources:
  - type: jira
    name: jira-cloud
    url: "https://example.atlassian.net"
    auth:
      email: "user@example.com"
      api_token: "tok"
    projects: ["DO", "PROD"]
    container: "jira-issues-cloud"
    companion_containers:
      sprints: "jira-sprints-cloud"
      fix_versions: "jira-fix-versions-cloud"
      projects: "jira-projects-cloud"
    fields:
      story_points: "customfield_10016"
  - type: confluence
    name: confluence-cloud
    url: "https://example.atlassian.net/wiki"
    auth:
      email: "user@example.com"
      api_token: "tok"
    spaces: ["ENG"]
    container: "confluence-pages-cloud"
    companion_containers:
      spaces: "confluence-spaces-cloud"
  - type: jira
    name: jira-dc
    url: "https://jira.internal.example"
    auth:
      pat: "my-pat"
    projects: ["INT"]

ingest:
  poll_interval: "300s"
  safety_lag_minutes: 2
  batch_size: 100
  reconcile_every: 12
  max_cycle_duration: "30m"
  max_concurrent_per_source: 1
  max_retries: 5

deployments:
  - name: ingest-azure
    role: ingest
    target: azure
    azure:
      container_app:
        cpu: 0.5
        memory: "1.0Gi"
        min_replicas: 1
        max_replicas: 1
    sources:
      - source: jira-cloud
      - source: confluence-cloud
  - name: ingest-onprem
    role: ingest
    target: onprem
    sources:
      - source: jira-dc
        projects: ["INT"]
  - name: mcp-azure
    role: mcp
    target: azure
    azure:
      container_app:
        cpu: 1.0
        memory: "2.0Gi"
        min_replicas: 0
        max_replicas: 5
    expose:
      - jira_issues
      - confluence_pages
    auth:
      mode: "api_key"

mcp:
  data_sources:
    jira_issues:
      kind: jira_issue
      backed_by:
        - container: jira-issues-cloud
  search:
    disable_agentic: false
    knowledge_base: "quelch-prod-kb"
  default_top: 25
  max_top: 100
  query_timeout: "30s"
  search_timeout: "20s"

rigg:
  dir: "./rigg"
  ownership: "generated"

state:
  backend: "cosmos"
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.sources.len(), 3);
        assert_eq!(config.deployments.len(), 3);
        assert_eq!(config.cosmos.throughput.ru_per_second, Some(1000));

        // Check Jira auth variants
        if let SourceConfig::Jira(j) = &config.sources[0] {
            assert!(matches!(j.auth, AuthConfig::Cloud { .. }));
            assert_eq!(
                j.fields.get("story_points").map(String::as_str),
                Some("customfield_10016")
            );
        } else {
            panic!("expected Jira source");
        }

        if let SourceConfig::Jira(j) = &config.sources[2] {
            assert!(matches!(j.auth, AuthConfig::DataCenter { .. }));
        } else {
            panic!("expected Jira source");
        }

        // Check MCP data sources
        assert!(config.mcp.data_sources.contains_key("jira_issues"));
    }
}
