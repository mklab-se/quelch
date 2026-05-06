use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub azure: AzureConfig,
    pub source_connections: Vec<SourceConnection>,
    pub instances: Vec<InstanceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AzureConfig {
    pub cosmos: CosmosConfig,
    #[serde(default)]
    pub search: Option<SearchConfig>,
    #[serde(default)]
    pub ai: Option<AiConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CosmosConfig {
    #[serde(default)]
    pub subscription_id: Option<String>,
    #[serde(default)]
    pub resource_group: Option<String>,
    #[serde(default)]
    pub account: Option<String>,
    pub endpoint: String,
    pub database: String,
    #[serde(default)]
    pub containers: ContainerLayout,
    #[serde(default = "default_meta_container")]
    pub meta_container: String,
}

fn default_meta_container() -> String {
    "quelch-meta".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ContainerLayout {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira_issues: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira_sprints: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira_fix_versions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira_projects: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confluence_pages: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confluence_spaces: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SearchConfig {
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiConfig {
    pub provider: AiProvider,
    pub endpoint: String,
    pub embedding: AiEmbedding,
    pub chat: AiChat,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    Foundry,
    AzureOpenai,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiEmbedding {
    pub deployment: String,
    pub dimensions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiChat {
    pub deployment: String,
    pub model_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SourceConnection {
    pub name: String,
    #[serde(rename = "type")]
    pub source_type: SourceType,
    pub base_url: String,
    pub auth: SourceAuth,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spaces: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    Jira,
    Confluence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceAuth {
    Pat { token: String },
    Basic { email: String, token: String },
}

impl SourceAuth {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstanceConfig {
    pub name: String,
    #[serde(flatten)]
    pub spec: InstanceSpec,
}

impl InstanceConfig {
    pub fn kind(&self) -> InstanceKind {
        match self.spec {
            InstanceSpec::Ingest(_) => InstanceKind::Ingest,
            InstanceSpec::Mcp(_) => InstanceKind::Mcp,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstanceKind {
    Ingest,
    Mcp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstanceSpec {
    Ingest(IngestInstance),
    Mcp(McpInstance),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct IngestInstance {
    pub connections: Vec<String>,
    #[serde(with = "humantime_serde")]
    pub cycle_interval: std::time::Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct McpInstance {
    pub expose: Vec<String>,
    pub api_key: String,
    pub knowledge_base: String,
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
