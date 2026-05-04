//! Exposure resolver: enforces the deployment's `expose:` list.
//!
//! MCP deployments declare which logical data sources they expose. Any tool
//! call referencing an unexposed source gets `McpError::Forbidden`.

use std::collections::{HashMap, HashSet};

use crate::config::Config;
use crate::config::data_sources::{ResolvedDataSource, resolve as resolve_all};
use crate::mcp::error::McpError;

/// Resolves logical data-source names to their physical containers, filtered
/// by the deployment's `expose:` list.
pub struct ExposeResolver {
    /// Logical data-source name → resolved (kind + backing containers).
    /// Already filtered by the deployment's `expose:` list.
    exposed: HashMap<String, ResolvedDataSource>,
}

impl ExposeResolver {
    /// Build from the sliced (per-deployment) config.
    ///
    /// The named MCP deployment's `expose:` list selects which data sources
    /// are visible. Sources not listed in `expose:` are silently excluded and
    /// return `McpError::Forbidden` if queried.
    pub fn from_sliced(config: &Config, deployment_name: &str) -> Result<Self, McpError> {
        let dep = config
            .deployments
            .iter()
            .find(|d| d.name == deployment_name)
            .ok_or_else(|| {
                McpError::Internal(format!(
                    "deployment '{deployment_name}' not found in sliced config"
                ))
            })?;

        if dep.role != crate::config::DeploymentRole::Mcp {
            return Err(McpError::Internal(
                "ExposeResolver requires an mcp deployment".into(),
            ));
        }

        let exposed_names: HashSet<&str> =
            dep.expose.iter().flatten().map(String::as_str).collect();

        let all = resolve_all(config);
        let exposed = all
            .into_iter()
            .filter(|(name, _)| exposed_names.contains(name.as_str()))
            .collect();

        Ok(Self { exposed })
    }

    /// Build directly from a pre-computed map (useful for tests).
    pub fn from_map(exposed: HashMap<String, ResolvedDataSource>) -> Self {
        Self { exposed }
    }

    /// Resolve a data-source name to its `ResolvedDataSource`.
    ///
    /// Returns `McpError::Forbidden` if the name is not in the exposure list.
    pub fn resolve(&self, data_source: &str) -> Result<&ResolvedDataSource, McpError> {
        self.exposed
            .get(data_source)
            .ok_or_else(|| McpError::Forbidden(data_source.into()))
    }

    /// Return all exposed data sources.
    pub fn list_all(&self) -> &HashMap<String, ResolvedDataSource> {
        &self.exposed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BackedBy;

    fn make_expose(names: &[&str]) -> ExposeResolver {
        let mut map = HashMap::new();
        for name in names {
            map.insert(
                name.to_string(),
                ResolvedDataSource {
                    kind: format!("{name}_kind"),
                    backed_by: vec![BackedBy {
                        container: format!("{name}-container"),
                    }],
                },
            );
        }
        ExposeResolver::from_map(map)
    }

    #[test]
    fn resolves_exposed_source() {
        let expose = make_expose(&["jira_issues"]);
        let resolved = expose.resolve("jira_issues").unwrap();
        assert_eq!(resolved.kind, "jira_issues_kind");
    }

    #[test]
    fn returns_forbidden_for_unexposed_source() {
        let expose = make_expose(&["jira_issues"]);
        let err = expose.resolve("confluence_pages").unwrap_err();
        assert!(matches!(err, McpError::Forbidden(name) if name == "confluence_pages"));
    }

    #[test]
    fn list_all_returns_only_exposed() {
        let expose = make_expose(&["jira_issues", "jira_sprints"]);
        assert_eq!(expose.list_all().len(), 2);
    }

    #[test]
    fn from_sliced_filters_by_deployment_expose_list() {
        let yaml = r#"
azure:
  subscription_id: "sub"
  resource_group: "rg"
  region: "swedencentral"
cosmos:
  database: "quelch"
ai:
  provider: azure_openai
  endpoint: "https://x.openai.azure.com"
  embedding:
    deployment: "te"
    dimensions: 1536
  chat:
    deployment: "gpt-4.1-mini"
    model_name: "gpt-4.1-mini"
sources:
  - type: jira
    name: jira-cloud
    url: "https://example.atlassian.net"
    auth:
      email: "u@example.com"
      api_token: "tok"
    projects: ["DO"]
deployments:
  - name: mcp-test
    role: mcp
    target: azure
    expose:
      - jira_issues
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let expose = ExposeResolver::from_sliced(&config, "mcp-test").unwrap();

        // jira_issues should be exposed
        assert!(expose.resolve("jira_issues").is_ok());

        // jira_sprints derived but not in expose list → forbidden
        assert!(matches!(
            expose.resolve("jira_sprints").unwrap_err(),
            McpError::Forbidden(_)
        ));
    }
}
