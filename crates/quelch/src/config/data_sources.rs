/// Auto-derivation of `mcp.data_sources` from configured sources and cosmos defaults.
///
/// When `config.mcp.data_sources` is non-empty, returns it verbatim (as a
/// [`ResolvedDataSource`] map). When it is empty, derives one entry per kind
/// from the sources in the config and the cosmos container defaults.
///
/// See `docs/configuration.md` — section "Auto-derived data_sources" for the
/// full derivation table.
use std::collections::HashMap;

use super::{BackedBy, Config, SourceConfig};

/// A resolved logical data source: its kind and the physical containers backing it.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedDataSource {
    /// The entity kind (e.g. `"jira_issue"`, `"confluence_page"`).
    pub kind: String,
    /// The physical Cosmos containers that back this data source.
    pub backed_by: Vec<BackedBy>,
}

/// Resolve the effective `data_sources` map for a config.
///
/// If `config.mcp.data_sources` is non-empty, it is returned verbatim
/// (converted to [`ResolvedDataSource`]).  Otherwise, one entry per kind is
/// derived from the configured sources and the `cosmos.containers` defaults.
pub fn resolve(config: &Config) -> HashMap<String, ResolvedDataSource> {
    if !config.mcp.data_sources.is_empty() {
        // Explicit override — use verbatim.
        return config
            .mcp
            .data_sources
            .iter()
            .map(|(name, spec)| {
                (
                    name.clone(),
                    ResolvedDataSource {
                        kind: spec.kind.clone(),
                        backed_by: spec.backed_by.clone(),
                    },
                )
            })
            .collect();
    }

    // Auto-derive from sources + cosmos defaults.
    let cosmos = &config.cosmos.containers;
    let mut jira_issues: Vec<BackedBy> = Vec::new();
    let mut jira_sprints: Vec<BackedBy> = Vec::new();
    let mut jira_fix_versions: Vec<BackedBy> = Vec::new();
    let mut jira_projects: Vec<BackedBy> = Vec::new();
    let mut confluence_pages: Vec<BackedBy> = Vec::new();
    let mut confluence_spaces: Vec<BackedBy> = Vec::new();

    for source in &config.sources {
        match source {
            SourceConfig::Jira(j) => {
                // Primary container (issues).
                let issues_container = j
                    .container
                    .clone()
                    .unwrap_or_else(|| cosmos.jira_issues.clone());
                jira_issues.push(BackedBy {
                    container: issues_container,
                });

                // Companion: sprints.
                let sprints_container = j
                    .companion_containers
                    .sprints
                    .clone()
                    .unwrap_or_else(|| cosmos.jira_sprints.clone());
                jira_sprints.push(BackedBy {
                    container: sprints_container,
                });

                // Companion: fix_versions.
                let fix_versions_container = j
                    .companion_containers
                    .fix_versions
                    .clone()
                    .unwrap_or_else(|| cosmos.jira_fix_versions.clone());
                jira_fix_versions.push(BackedBy {
                    container: fix_versions_container,
                });

                // Companion: projects.
                let projects_container = j
                    .companion_containers
                    .projects
                    .clone()
                    .unwrap_or_else(|| cosmos.jira_projects.clone());
                jira_projects.push(BackedBy {
                    container: projects_container,
                });
            }
            SourceConfig::Confluence(c) => {
                // Primary container (pages).
                let pages_container = c
                    .container
                    .clone()
                    .unwrap_or_else(|| cosmos.confluence_pages.clone());
                confluence_pages.push(BackedBy {
                    container: pages_container,
                });

                // Companion: spaces.
                let spaces_container = c
                    .companion_containers
                    .spaces
                    .clone()
                    .unwrap_or_else(|| cosmos.confluence_spaces.clone());
                confluence_spaces.push(BackedBy {
                    container: spaces_container,
                });
            }
        }
    }

    let mut map = HashMap::new();

    if !jira_issues.is_empty() {
        map.insert(
            "jira_issues".to_string(),
            ResolvedDataSource {
                kind: "jira_issue".to_string(),
                backed_by: jira_issues,
            },
        );
    }
    if !jira_sprints.is_empty() {
        map.insert(
            "jira_sprints".to_string(),
            ResolvedDataSource {
                kind: "jira_sprint".to_string(),
                backed_by: jira_sprints,
            },
        );
    }
    if !jira_fix_versions.is_empty() {
        map.insert(
            "jira_fix_versions".to_string(),
            ResolvedDataSource {
                kind: "jira_fix_version".to_string(),
                backed_by: jira_fix_versions,
            },
        );
    }
    if !jira_projects.is_empty() {
        map.insert(
            "jira_projects".to_string(),
            ResolvedDataSource {
                kind: "jira_project".to_string(),
                backed_by: jira_projects,
            },
        );
    }
    if !confluence_pages.is_empty() {
        map.insert(
            "confluence_pages".to_string(),
            ResolvedDataSource {
                kind: "confluence_page".to_string(),
                backed_by: confluence_pages,
            },
        );
    }
    if !confluence_spaces.is_empty() {
        map.insert(
            "confluence_spaces".to_string(),
            ResolvedDataSource {
                kind: "confluence_space".to_string(),
                backed_by: confluence_spaces,
            },
        );
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn parse(yaml: &str) -> Config {
        serde_yaml::from_str(yaml).expect("yaml must parse")
    }

    const BASE: &str = r#"
azure:
  subscription_id: "sub-test"
  resource_group: "rg-test"
  region: "swedencentral"
cosmos:
  database: "quelch"
ai:
  provider: azure_openai
  endpoint: "https://test.openai.azure.com"
  embedding:
    deployment: "text-embedding-3-large"
    dimensions: 3072
  chat:
    deployment: "gpt-4.1-mini"
    model_name: "gpt-4.1-mini"
"#;

    #[test]
    fn derives_jira_issues_from_two_sources() {
        let yaml = format!(
            r#"{BASE}
sources:
  - type: jira
    name: jira-cloud
    url: "https://cloud.atlassian.net"
    auth:
      email: "u@example.com"
      api_token: "tok"
    projects: ["DO"]
  - type: jira
    name: jira-dc
    url: "https://jira.internal"
    auth:
      pat: "my-pat"
    projects: ["INT"]
deployments: []
"#
        );
        let cfg = parse(&yaml);
        let resolved = resolve(&cfg);

        let issues = resolved
            .get("jira_issues")
            .expect("jira_issues must be derived");
        assert_eq!(issues.kind, "jira_issue");
        assert_eq!(issues.backed_by.len(), 2);

        let containers: Vec<&str> = issues
            .backed_by
            .iter()
            .map(|b| b.container.as_str())
            .collect();
        assert!(
            containers.contains(&"jira-issues"),
            "expected default container 'jira-issues', got {containers:?}"
        );
    }

    #[test]
    fn derives_companion_containers_with_overrides() {
        let yaml = format!(
            r#"{BASE}
sources:
  - type: jira
    name: jira-cloud
    url: "https://cloud.atlassian.net"
    auth:
      email: "u@example.com"
      api_token: "tok"
    projects: ["DO"]
    companion_containers:
      sprints: "jira-sprints-cloud"
      fix_versions: "jira-fix-versions-cloud"
      projects: "jira-projects-cloud"
deployments: []
"#
        );
        let cfg = parse(&yaml);
        let resolved = resolve(&cfg);

        let sprints = resolved
            .get("jira_sprints")
            .expect("jira_sprints must be derived");
        assert_eq!(sprints.kind, "jira_sprint");
        assert_eq!(sprints.backed_by[0].container, "jira-sprints-cloud");

        let fix_versions = resolved
            .get("jira_fix_versions")
            .expect("jira_fix_versions must be derived");
        assert_eq!(
            fix_versions.backed_by[0].container,
            "jira-fix-versions-cloud"
        );

        let projects = resolved
            .get("jira_projects")
            .expect("jira_projects must be derived");
        assert_eq!(projects.backed_by[0].container, "jira-projects-cloud");
    }

    #[test]
    fn derives_confluence_sources() {
        let yaml = format!(
            r#"{BASE}
sources:
  - type: confluence
    name: confluence-cloud
    url: "https://cloud.atlassian.net/wiki"
    auth:
      email: "u@example.com"
      api_token: "tok"
    spaces: ["ENG"]
    companion_containers:
      spaces: "confluence-spaces-cloud"
deployments: []
"#
        );
        let cfg = parse(&yaml);
        let resolved = resolve(&cfg);

        let pages = resolved
            .get("confluence_pages")
            .expect("confluence_pages must be derived");
        assert_eq!(pages.kind, "confluence_page");
        // Uses cosmos default because no container override.
        assert_eq!(pages.backed_by[0].container, "confluence-pages");

        let spaces = resolved
            .get("confluence_spaces")
            .expect("confluence_spaces must be derived");
        assert_eq!(spaces.backed_by[0].container, "confluence-spaces-cloud");
    }

    #[test]
    fn explicit_override_used_verbatim() {
        let yaml = format!(
            r#"{BASE}
sources:
  - type: jira
    name: jira-cloud
    url: "https://cloud.atlassian.net"
    auth:
      email: "u@example.com"
      api_token: "tok"
    projects: ["DO"]
deployments: []
mcp:
  data_sources:
    my_custom_source:
      kind: jira_issue
      backed_by:
        - container: custom-container
"#
        );
        let cfg = parse(&yaml);
        let resolved = resolve(&cfg);

        // Only the explicit entry should appear — no auto-derivation.
        assert_eq!(resolved.len(), 1);
        let custom = resolved
            .get("my_custom_source")
            .expect("my_custom_source must be present");
        assert_eq!(custom.kind, "jira_issue");
        assert_eq!(custom.backed_by[0].container, "custom-container");

        // Auto-derived jira_issues must NOT be present.
        assert!(
            !resolved.contains_key("jira_issues"),
            "jira_issues should not be auto-derived when explicit data_sources is set"
        );
    }

    #[test]
    fn empty_sources_produces_empty_map() {
        let yaml = format!(
            r#"{BASE}
sources: []
deployments: []
"#
        );
        let cfg = parse(&yaml);
        let resolved = resolve(&cfg);
        assert!(resolved.is_empty());
    }
}
