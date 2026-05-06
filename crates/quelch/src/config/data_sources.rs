//! Resolve logical data-source names to their backing Cosmos containers.
//!
//! A *logical data source* (`jira_issues`, `confluence_pages`, etc.) is what
//! agents see through the MCP server. A *physical container* is what Cosmos
//! actually stores. This module resolves one to the other from a parsed
//! config.
//!
//! Resolution is driven by the kinds of source connections present:
//! - any `jira` connection contributes `jira_issues`, `jira_sprints`,
//!   `jira_fix_versions`, and `jira_projects`;
//! - any `confluence` connection contributes `confluence_pages` and
//!   `confluence_spaces`.
//!
//! Container names come from `azure.cosmos.containers` overrides, falling
//! back to the canonical defaults (`jira-issues`, `confluence-pages`, etc.).

use std::collections::HashMap;

use super::Config;
use super::schema::{ContainerLayout, SourceType};

/// A resolved logical data source: its kind and the physical container(s)
/// backing it.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedDataSource {
    /// The entity kind (e.g. `"jira_issue"`, `"confluence_page"`).
    pub kind: String,
    /// The physical Cosmos container(s) that back this data source.
    pub backed_by: Vec<BackedBy>,
}

/// A single physical Cosmos container backing a logical data source.
#[derive(Debug, Clone, PartialEq)]
pub struct BackedBy {
    /// Cosmos container name.
    pub container: String,
}

/// Resolve the effective `data_sources` map for a config.
///
/// Inspects `config.source_connections` to determine which kinds of data
/// sources exist, then consults `config.azure.cosmos.containers` (with the
/// canonical defaults) to map each one to a Cosmos container.
pub fn resolve(config: &Config) -> HashMap<String, ResolvedDataSource> {
    let cc = &config.azure.cosmos.containers;
    let mut has_jira = false;
    let mut has_confluence = false;
    for c in &config.source_connections {
        match c.source_type {
            SourceType::Jira => has_jira = true,
            SourceType::Confluence => has_confluence = true,
        }
    }

    let mut map = HashMap::new();

    if has_jira {
        map.insert(
            "jira_issues".into(),
            ResolvedDataSource {
                kind: "jira_issue".into(),
                backed_by: vec![BackedBy {
                    container: container_for("jira_issues", cc),
                }],
            },
        );
        map.insert(
            "jira_sprints".into(),
            ResolvedDataSource {
                kind: "jira_sprint".into(),
                backed_by: vec![BackedBy {
                    container: container_for("jira_sprints", cc),
                }],
            },
        );
        map.insert(
            "jira_fix_versions".into(),
            ResolvedDataSource {
                kind: "jira_fix_version".into(),
                backed_by: vec![BackedBy {
                    container: container_for("jira_fix_versions", cc),
                }],
            },
        );
        map.insert(
            "jira_projects".into(),
            ResolvedDataSource {
                kind: "jira_project".into(),
                backed_by: vec![BackedBy {
                    container: container_for("jira_projects", cc),
                }],
            },
        );
    }

    if has_confluence {
        map.insert(
            "confluence_pages".into(),
            ResolvedDataSource {
                kind: "confluence_page".into(),
                backed_by: vec![BackedBy {
                    container: container_for("confluence_pages", cc),
                }],
            },
        );
        map.insert(
            "confluence_spaces".into(),
            ResolvedDataSource {
                kind: "confluence_space".into(),
                backed_by: vec![BackedBy {
                    container: container_for("confluence_spaces", cc),
                }],
            },
        );
    }

    map
}

/// Look up the configured container name for a logical data-source key,
/// falling back to the canonical default.
fn container_for(name: &str, cc: &ContainerLayout) -> String {
    match name {
        "jira_issues" => cc
            .jira_issues
            .clone()
            .unwrap_or_else(|| "jira-issues".into()),
        "jira_sprints" => cc
            .jira_sprints
            .clone()
            .unwrap_or_else(|| "jira-sprints".into()),
        "jira_fix_versions" => cc
            .jira_fix_versions
            .clone()
            .unwrap_or_else(|| "jira-fix-versions".into()),
        "jira_projects" => cc
            .jira_projects
            .clone()
            .unwrap_or_else(|| "jira-projects".into()),
        "confluence_pages" => cc
            .confluence_pages
            .clone()
            .unwrap_or_else(|| "confluence-pages".into()),
        "confluence_spaces" => cc
            .confluence_spaces
            .clone()
            .unwrap_or_else(|| "confluence-spaces".into()),
        // Defensive fallback — only exercised if a caller adds a new key.
        other => other.replace('_', "-"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Config {
        serde_yaml::from_str(yaml).expect("yaml parses")
    }

    const JIRA_ONLY: &str = r#"
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
instances: []
"#;

    const CONFLUENCE_ONLY: &str = r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
source_connections:
  - name: conf-x
    type: confluence
    base_url: https://conf.internal
    auth: { kind: pat, token: T }
    spaces: [ENG]
instances: []
"#;

    const BOTH_WITH_OVERRIDE: &str = r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
    containers:
      jira_issues: my-jira-issues
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
instances: []
"#;

    #[test]
    fn resolve_jira_only_yields_four_data_sources() {
        let cfg = parse(JIRA_ONLY);
        let resolved = resolve(&cfg);
        assert!(resolved.contains_key("jira_issues"));
        assert!(resolved.contains_key("jira_sprints"));
        assert!(resolved.contains_key("jira_fix_versions"));
        assert!(resolved.contains_key("jira_projects"));
        assert!(!resolved.contains_key("confluence_pages"));
        assert_eq!(resolved["jira_issues"].kind, "jira_issue");
        assert_eq!(
            resolved["jira_issues"].backed_by[0].container,
            "jira-issues"
        );
    }

    #[test]
    fn resolve_confluence_only_yields_two_data_sources() {
        let cfg = parse(CONFLUENCE_ONLY);
        let resolved = resolve(&cfg);
        assert!(resolved.contains_key("confluence_pages"));
        assert!(resolved.contains_key("confluence_spaces"));
        assert!(!resolved.contains_key("jira_issues"));
    }

    #[test]
    fn resolve_honours_container_override() {
        let cfg = parse(BOTH_WITH_OVERRIDE);
        let resolved = resolve(&cfg);
        assert_eq!(
            resolved["jira_issues"].backed_by[0].container,
            "my-jira-issues"
        );
        // Other defaults are untouched.
        assert_eq!(
            resolved["confluence_pages"].backed_by[0].container,
            "confluence-pages"
        );
    }

    #[test]
    fn resolve_empty_sources_returns_empty_map() {
        let cfg = parse(
            r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
source_connections: []
instances: []
"#,
        );
        let resolved = resolve(&cfg);
        assert!(resolved.is_empty());
    }
}
