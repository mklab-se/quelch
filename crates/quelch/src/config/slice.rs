/// Effective-config slicing: extract the sub-`Config` needed by one deployment.
///
/// See `docs/configuration.md` — section "Slicing per deployment".
use super::{Config, ConfigError, DeploymentRole, data_sources};

/// Return a sub-[`Config`] containing only what the named deployment needs.
///
/// # Rules
/// - `azure`, `cosmos`, `search`, `openai`, `ingest`, `rigg`, `state` are kept verbatim.
/// - `deployments`: only the named one is kept.
/// - `sources`: filtered to those the deployment actually needs.
///   - Ingest: those listed in `deployment.sources`.
///   - MCP: those whose primary container appears in any resolved `data_sources`
///     entry that is in the deployment's `expose` list.
/// - `mcp.data_sources`: for MCP deployments, filtered to the `expose` list;
///   for ingest deployments, cleared.
///
/// # Errors
/// Returns [`ConfigError::DeploymentNotFound`] if no deployment with the given name exists.
pub fn for_deployment(config: &Config, name: &str) -> Result<Config, ConfigError> {
    let deployment = config
        .deployments
        .iter()
        .find(|d| d.name == name)
        .ok_or_else(|| ConfigError::DeploymentNotFound(name.to_string()))?
        .clone();

    // Resolve data_sources (explicit or auto-derived).
    let resolved = data_sources::resolve(config);

    let (filtered_sources, filtered_data_sources) = match deployment.role {
        DeploymentRole::Ingest => {
            // Keep only sources listed in deployment.sources.
            let referenced: std::collections::HashSet<&str> = deployment
                .sources
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(|ds| ds.source.as_str())
                .collect();

            let sources = config
                .sources
                .iter()
                .filter(|s| referenced.contains(s.name()))
                .cloned()
                .collect();

            (sources, std::collections::HashMap::new())
        }
        DeploymentRole::Mcp => {
            // Determine which data-source names are exposed.
            let expose: std::collections::HashSet<&str> = deployment
                .expose
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(String::as_str)
                .collect();

            // Collect the physical containers for the exposed data sources.
            let mut exposed_containers: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut sliced_data_sources = std::collections::HashMap::new();

            for ds_name in &expose {
                if let Some(resolved_ds) = resolved.get(*ds_name) {
                    for b in &resolved_ds.backed_by {
                        exposed_containers.insert(b.container.clone());
                    }
                    // Add to sliced data_sources (from explicit map if present,
                    // otherwise synthesise from resolved).
                    if let Some(spec) = config.mcp.data_sources.get(*ds_name) {
                        sliced_data_sources.insert((*ds_name).to_string(), spec.clone());
                    } else {
                        // Auto-derived — synthesise a McpDataSourceSpec.
                        let spec = super::McpDataSourceSpec {
                            kind: resolved_ds.kind.clone(),
                            backed_by: resolved_ds.backed_by.clone(),
                        };
                        sliced_data_sources.insert((*ds_name).to_string(), spec);
                    }
                }
            }

            // Keep sources whose primary container is in the exposed set.
            let sources = config
                .sources
                .iter()
                .filter(|s| {
                    let primary = match s {
                        super::SourceConfig::Jira(j) => j
                            .container
                            .clone()
                            .unwrap_or_else(|| config.cosmos.containers.jira_issues.clone()),
                        super::SourceConfig::Confluence(c) => c
                            .container
                            .clone()
                            .unwrap_or_else(|| config.cosmos.containers.confluence_pages.clone()),
                    };
                    exposed_containers.contains(&primary)
                })
                .cloned()
                .collect();

            (sources, sliced_data_sources)
        }
    };

    let mut mcp = config.mcp.clone();
    mcp.data_sources = filtered_data_sources;

    Ok(Config {
        azure: config.azure.clone(),
        cosmos: config.cosmos.clone(),
        search: config.search.clone(),
        ai: config.ai.clone(),
        ingest: config.ingest.clone(),
        rigg: config.rigg.clone(),
        state: config.state.clone(),
        deployments: vec![deployment],
        sources: filtered_sources,
        mcp,
    })
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
    fn slice_for_mcp_excludes_other_sources() {
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
  - type: confluence
    name: confluence-cloud
    url: "https://cloud.atlassian.net/wiki"
    auth:
      email: "u@example.com"
      api_token: "tok"
    spaces: ["ENG"]
deployments:
  - name: mcp
    role: mcp
    target: azure
    expose:
      - jira_issues
    auth:
      mode: "api_key"
"#
        );
        let cfg = parse(&yaml);
        let sliced = for_deployment(&cfg, "mcp").expect("mcp deployment must exist");

        // Only the jira source should be included (not confluence).
        assert_eq!(sliced.sources.len(), 1);
        assert_eq!(sliced.sources[0].name(), "jira-cloud");

        // Only the exposed data source.
        assert!(sliced.mcp.data_sources.contains_key("jira_issues"));
        assert!(!sliced.mcp.data_sources.contains_key("confluence_pages"));

        // Only one deployment.
        assert_eq!(sliced.deployments.len(), 1);
        assert_eq!(sliced.deployments[0].name, "mcp");
    }

    #[test]
    fn slice_for_ingest_excludes_other_sources() {
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
  - type: confluence
    name: confluence-cloud
    url: "https://cloud.atlassian.net/wiki"
    auth:
      email: "u@example.com"
      api_token: "tok"
    spaces: ["ENG"]
deployments:
  - name: ingest-jira
    role: ingest
    target: azure
    sources:
      - source: jira-cloud
  - name: ingest-confluence
    role: ingest
    target: azure
    sources:
      - source: confluence-cloud
"#
        );
        let cfg = parse(&yaml);
        let sliced =
            for_deployment(&cfg, "ingest-jira").expect("ingest-jira deployment must exist");

        assert_eq!(sliced.sources.len(), 1);
        assert_eq!(sliced.sources[0].name(), "jira-cloud");
        assert_eq!(sliced.deployments.len(), 1);
        assert_eq!(sliced.deployments[0].name, "ingest-jira");

        // Ingest deployments have no mcp.data_sources.
        assert!(sliced.mcp.data_sources.is_empty());
    }

    #[test]
    fn deployment_not_found_returns_error() {
        let yaml = format!(
            r#"{BASE}
sources: []
deployments: []
"#
        );
        let cfg = parse(&yaml);
        let err = for_deployment(&cfg, "nonexistent").unwrap_err();
        assert!(
            err.to_string().contains("nonexistent"),
            "error should mention the missing deployment name: {err}"
        );
    }

    #[test]
    fn slice_for_mcp_with_auto_derived_data_sources() {
        // No explicit mcp.data_sources — relies on auto-derivation.
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
  - type: confluence
    name: confluence-cloud
    url: "https://cloud.atlassian.net/wiki"
    auth:
      email: "u@example.com"
      api_token: "tok"
    spaces: ["ENG"]
deployments:
  - name: mcp
    role: mcp
    target: azure
    expose:
      - jira_issues
      - confluence_pages
    auth:
      mode: "api_key"
"#
        );
        let cfg = parse(&yaml);
        let sliced = for_deployment(&cfg, "mcp").expect("mcp deployment must exist");

        // Both sources included since both are exposed.
        assert_eq!(sliced.sources.len(), 2);
        assert!(sliced.mcp.data_sources.contains_key("jira_issues"));
        assert!(sliced.mcp.data_sources.contains_key("confluence_pages"));
    }
}
