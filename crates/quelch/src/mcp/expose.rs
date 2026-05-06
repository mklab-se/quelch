use std::collections::HashMap;

use crate::config::Config;
use crate::config::data_sources::ResolvedDataSource;
use crate::mcp::error::McpError;

pub struct ExposeResolver {
    exposed: HashMap<String, ResolvedDataSource>,
}

impl ExposeResolver {
    pub fn from_sliced(_config: &Config, _instance_name: &str) -> Result<Self, McpError> {
        todo!("phase 7: rewire ExposeResolver against the new instances schema")
    }

    pub fn from_map(exposed: HashMap<String, ResolvedDataSource>) -> Self {
        Self { exposed }
    }

    pub fn resolve(&self, data_source: &str) -> Result<&ResolvedDataSource, McpError> {
        self.exposed
            .get(data_source)
            .ok_or_else(|| McpError::Forbidden(data_source.into()))
    }

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
}
