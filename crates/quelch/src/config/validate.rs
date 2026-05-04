/// Config validation rules.
///
/// Three invariants are checked:
/// 1. Every source referenced in a deployment exists in `sources`.
/// 2. Every `(source, subsource)` pair appears in at most one ingest deployment.
/// 3. Every name in any `expose:` list is defined in `mcp.data_sources` (explicit
///    or auto-derived via [`super::data_sources::resolve`]).
use super::{Config, ConfigError, DeploymentRole, SourceConfig, data_sources};
use std::collections::{HashMap, HashSet};

/// Run all validation rules against `config`.
pub fn run(config: &Config) -> Result<(), ConfigError> {
    validate_sources_referenced(config)?;
    validate_disjoint_subsources(config)?;
    validate_expose_resolves(config)?;
    validate_no_bicep_unsafe_chars(config)?;
    Ok(())
}

/// Reject config values that are interpolated raw into generated Bicep
/// templates and could break the template if they contain certain characters.
///
/// Bicep string literals are single-quoted, so an apostrophe (`'`) in any
/// interpolated value would terminate the literal early and let the rest of
/// the value be parsed as Bicep code. Backslashes can escape the closing
/// quote in some downstream tooling. We reject both at config-load time —
/// these values are resource names (Azure has its own naming rules anyway,
/// and Azure resource names cannot contain `'` or `\`), so this only
/// enforces what's already true.
fn validate_no_bicep_unsafe_chars(config: &Config) -> Result<(), ConfigError> {
    let check = |label: &str, value: &str| -> Result<(), ConfigError> {
        if value.contains('\'') || value.contains('\\') {
            return Err(ConfigError::Validation(format!(
                "{label} '{value}' contains a quote or backslash; \
                 Quelch interpolates these into generated Bicep and \
                 they would break the template"
            )));
        }
        Ok(())
    };

    check("azure.resource_group", &config.azure.resource_group)?;
    check("azure.region", &config.azure.region)?;
    if let Some(ref prefix) = config.azure.naming.prefix {
        check("azure.naming.prefix", prefix)?;
    }
    if let Some(ref env) = config.azure.naming.environment {
        check("azure.naming.environment", env)?;
    }
    if let Some(ref account) = config.cosmos.account {
        check("cosmos.account", account)?;
    }
    check("cosmos.database", &config.cosmos.database)?;
    check("cosmos.meta_container", &config.cosmos.meta_container)?;
    if let Some(ref service) = config.search.service {
        check("search.service", service)?;
    }
    for (kind, container_name) in &[
        ("jira_issues", &config.cosmos.containers.jira_issues),
        (
            "confluence_pages",
            &config.cosmos.containers.confluence_pages,
        ),
        ("jira_sprints", &config.cosmos.containers.jira_sprints),
        (
            "jira_fix_versions",
            &config.cosmos.containers.jira_fix_versions,
        ),
        ("jira_projects", &config.cosmos.containers.jira_projects),
        (
            "confluence_spaces",
            &config.cosmos.containers.confluence_spaces,
        ),
    ] {
        check(&format!("cosmos.containers.{kind}"), container_name)?;
    }
    for src in &config.sources {
        check("source name", src.name())?;
    }
    for dep in &config.deployments {
        check("deployment name", &dep.name)?;
    }
    Ok(())
}

/// Every source name referenced in a deployment must be defined in `sources`.
fn validate_sources_referenced(config: &Config) -> Result<(), ConfigError> {
    let defined: HashSet<&str> = config.sources.iter().map(|s| s.name()).collect();

    for deployment in &config.deployments {
        let Some(ref sources) = deployment.sources else {
            continue;
        };
        for ds in sources {
            if !defined.contains(ds.source.as_str()) {
                return Err(ConfigError::Validation(format!(
                    "deployment '{}' references source '{}' which is not defined in sources",
                    deployment.name, ds.source
                )));
            }
        }
    }
    Ok(())
}

/// Each `(source, subsource)` pair must appear in at most one ingest deployment.
///
/// A `DeploymentSource` without an explicit `projects`/`spaces` list means "all
/// subsources" of that source.  For the disjoint check we treat "all subsources"
/// as a special sentinel: if one deployment claims all subsources of a source and
/// another deployment also references that source (with or without a subset), that
/// is an overlap.
fn validate_disjoint_subsources(config: &Config) -> Result<(), ConfigError> {
    // Map from source name → list of (deployment_name, subsource_key)
    // where subsource_key is either a specific project/space or "ALL".
    let mut claimed: HashMap<&str, Vec<(&str, String)>> = HashMap::new();

    for deployment in &config.deployments {
        if !matches!(deployment.role, DeploymentRole::Ingest) {
            continue;
        }
        let Some(ref sources) = deployment.sources else {
            continue;
        };
        for ds in sources {
            let source_name = ds.source.as_str();
            let source_def = config.sources.iter().find(|s| s.name() == source_name);

            // Collect explicit subsource keys, or "ALL" if none specified.
            let subsources: Vec<String> = match (ds.projects.as_ref(), ds.spaces.as_ref()) {
                (Some(projects), _) if !projects.is_empty() => projects.clone(),
                (_, Some(spaces)) if !spaces.is_empty() => spaces.clone(),
                _ => {
                    // No explicit subset — derive from source definition or use sentinel.
                    match source_def {
                        Some(SourceConfig::Jira(j)) if !j.projects.is_empty() => j.projects.clone(),
                        Some(SourceConfig::Confluence(c)) if !c.spaces.is_empty() => {
                            c.spaces.clone()
                        }
                        _ => vec!["ALL".to_string()],
                    }
                }
            };

            let entry = claimed.entry(source_name).or_default();
            for sub in subsources {
                // Check if this subsource is already claimed.
                for (prev_dep, prev_sub) in entry.iter() {
                    let overlap = prev_sub == "ALL" || sub == "ALL" || prev_sub == &sub;
                    if overlap {
                        return Err(ConfigError::Validation(format!(
                            "subsource '{}' of source '{}' appears in both deployment '{}' \
                             and deployment '{}' — each (source, subsource) pair must appear \
                             in at most one ingest deployment",
                            sub, source_name, prev_dep, deployment.name
                        )));
                    }
                }
                entry.push((deployment.name.as_str(), sub));
            }
        }
    }
    Ok(())
}

/// Every name in any `expose:` list must be resolvable — either explicitly
/// defined in `mcp.data_sources` or auto-derivable from the configured sources.
fn validate_expose_resolves(config: &Config) -> Result<(), ConfigError> {
    // Compute the full resolved set once (explicit overrides OR auto-derived).
    let resolved = data_sources::resolve(config);

    for deployment in &config.deployments {
        let Some(ref expose) = deployment.expose else {
            continue;
        };
        for name in expose {
            if !resolved.contains_key(name) {
                return Err(ConfigError::Validation(format!(
                    "deployment '{}' exposes '{}' which is not defined in mcp.data_sources \
                     and cannot be auto-derived from the configured sources",
                    deployment.name, name
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn rejects_overlapping_subsources() {
        let yaml = include_str!("../../tests/fixtures/config_overlapping.yaml");
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        let err = run(&cfg).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("DO"), "expected 'DO' in error: {msg}");
        assert!(
            msg.contains("appears in"),
            "expected 'appears in' in error: {msg}"
        );
    }

    #[test]
    fn rejects_undefined_expose() {
        let yaml = include_str!("../../tests/fixtures/config_undefined_expose.yaml");
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        let err = run(&cfg).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("no_such_source"),
            "expected 'no_such_source' in error: {msg}"
        );
    }

    #[test]
    fn rejects_undefined_source_in_deployment() {
        let yaml = include_str!("../../tests/fixtures/config_undefined_source.yaml");
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        let err = run(&cfg).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("ghost-source"),
            "expected 'ghost-source' in error: {msg}"
        );
    }

    #[test]
    fn accepts_valid_config() {
        let yaml = include_str!("../../tests/fixtures/quelch.minimal.yaml");
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        run(&cfg).expect("valid config should pass validation");
    }

    /// Regression test: a config with `expose: [jira_issues]` and no explicit
    /// `mcp.data_sources` must pass validation (relies on auto-derivation).
    #[test]
    fn accepts_expose_with_auto_derived_data_sources() {
        let yaml = r#"
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
    deployment: "gpt-5-mini"
    model_name: "gpt-5-mini"
sources:
  - type: jira
    name: jira-cloud
    url: "https://cloud.atlassian.net"
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
    expose:
      - jira_issues
    auth:
      mode: "api_key"
# No mcp.data_sources — relies on auto-derivation.
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        run(&cfg).expect("expose with auto-derived data sources should pass validation");
    }
}
