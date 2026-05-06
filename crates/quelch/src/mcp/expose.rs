//! Maps a Q-MCP instance's `expose:` list to resolved data sources.
//!
//! `from_sliced` reads the named instance from a `Config` (typically a slice
//! produced by `slice_for_instance`), calls [`crate::config::data_sources::resolve`]
//! to enumerate all candidate data sources, and filters the result to only
//! those listed in the instance's `expose:` block.

use std::collections::HashMap;

use crate::config::Config;
use crate::config::data_sources::{ResolvedDataSource, resolve as resolve_data_sources};
use crate::config::schema::InstanceSpec;
use crate::mcp::error::McpError;

/// Wraps the set of data sources a single MCP instance is allowed to serve.
///
/// Constructed from an `expose:` list on an `mcp` instance; lookups by
/// data-source name return [`McpError::Forbidden`] for anything outside it.
#[derive(Debug)]
pub struct ExposeResolver {
    exposed: HashMap<String, ResolvedDataSource>,
}

impl ExposeResolver {
    /// Build an `ExposeResolver` from a config and the name of the MCP
    /// instance whose `expose:` list to honour.
    ///
    /// Resolves all candidate data sources from the master config, then
    /// keeps only the ones the named instance explicitly exposes. Returns
    /// [`McpError::Internal`] if `instance_name` is missing or refers to a
    /// non-MCP instance.
    pub fn from_sliced(config: &Config, instance_name: &str) -> Result<Self, McpError> {
        let inst = config
            .instances
            .iter()
            .find(|i| i.name == instance_name)
            .ok_or_else(|| {
                McpError::Internal(format!("instance '{instance_name}' not found in config"))
            })?;

        let expose = match &inst.spec {
            InstanceSpec::Mcp(m) => &m.expose,
            InstanceSpec::Ingest(_) => {
                return Err(McpError::Internal(format!(
                    "instance '{instance_name}' is an ingest instance, expected mcp"
                )));
            }
        };

        let all = resolve_data_sources(config);
        let exposed: HashMap<String, ResolvedDataSource> = all
            .into_iter()
            .filter(|(name, _)| expose.iter().any(|e| e == name))
            .collect();

        Ok(Self { exposed })
    }

    /// Construct an `ExposeResolver` directly from a pre-built map (test helper).
    pub fn from_map(exposed: HashMap<String, ResolvedDataSource>) -> Self {
        Self { exposed }
    }

    /// Look up an exposed data source by its logical name.
    ///
    /// Returns [`McpError::Forbidden`] if the name is not in the expose set.
    pub fn resolve(&self, data_source: &str) -> Result<&ResolvedDataSource, McpError> {
        self.exposed
            .get(data_source)
            .ok_or_else(|| McpError::Forbidden(data_source.into()))
    }

    /// Return the full exposed map (used by `list_sources`).
    pub fn list_all(&self) -> &HashMap<String, ResolvedDataSource> {
        &self.exposed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::data_sources::BackedBy;

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

    const FIXTURE: &str = r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
source_connections:
  - name: jira-x
    type: jira
    base_url: https://jira.internal
    auth: { kind: pat, token: T }
    projects: [DO]
  - name: conf-x
    type: confluence
    base_url: https://conf.internal
    auth: { kind: pat, token: T }
    spaces: [ENG]
instances:
  - name: ingest
    kind: ingest
    connections: [jira-x, conf-x]
    cycle_interval: 5m
  - name: mcp-jira-only
    kind: mcp
    expose: [jira_issues]
    api_key: K
    knowledge_base: kb
    listen: 0.0.0.0:8080
"#;

    #[test]
    fn from_sliced_keeps_only_exposed_sources() {
        let cfg: Config = serde_yaml::from_str(FIXTURE).unwrap();
        let r = ExposeResolver::from_sliced(&cfg, "mcp-jira-only").unwrap();
        assert_eq!(r.list_all().len(), 1);
        assert!(r.resolve("jira_issues").is_ok());
        assert!(r.resolve("confluence_pages").is_err());
        assert!(r.resolve("jira_sprints").is_err());
    }

    #[test]
    fn from_sliced_unknown_instance_errors() {
        let cfg: Config = serde_yaml::from_str(FIXTURE).unwrap();
        let err = ExposeResolver::from_sliced(&cfg, "ghost").unwrap_err();
        assert!(matches!(err, McpError::Internal(msg) if msg.contains("ghost")));
    }

    #[test]
    fn from_sliced_rejects_ingest_instance() {
        let cfg: Config = serde_yaml::from_str(FIXTURE).unwrap();
        let err = ExposeResolver::from_sliced(&cfg, "ingest").unwrap_err();
        assert!(matches!(err, McpError::Internal(msg) if msg.contains("ingest")));
    }
}
