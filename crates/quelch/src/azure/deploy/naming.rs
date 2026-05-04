/// Resource name computation helpers.
///
/// All Azure resources managed by Quelch follow the naming convention:
/// `{prefix}-{environment}-{deployment_name}[-{suffix}]`
///
/// Use [`container_app_name`] to get the full Container App name for a
/// deployment, or [`azure_resource_name`] for generic resource names.
use crate::config::Config;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute the Container App name for a deployment.
///
/// Pattern: `{prefix}-{environment}-{deployment_name}`
///
/// # Examples
/// ```
/// // prefix="quelch", environment="prod", deployment="ingest"
/// // → "quelch-prod-ingest"
/// ```
pub fn container_app_name(config: &Config, deployment_name: &str) -> String {
    azure_resource_name(config, deployment_name, None)
}

/// Compute a generic Azure resource name for a deployment.
///
/// Pattern: `{prefix}-{environment}-{deployment_name}[-{suffix}]`
///
/// If `suffix` is `None`, the suffix segment is omitted.
pub fn azure_resource_name(config: &Config, deployment_name: &str, suffix: Option<&str>) -> String {
    let prefix = config.azure.naming.prefix.as_deref().unwrap_or("quelch");
    let env = config.azure.naming.environment.as_deref().unwrap_or("prod");

    match suffix {
        Some(s) => format!("{prefix}-{env}-{deployment_name}-{s}"),
        None => format!("{prefix}-{env}-{deployment_name}"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn minimal_config(prefix: Option<&str>, environment: Option<&str>) -> Config {
        let prefix_yaml = prefix
            .map(|p| format!("    prefix: \"{p}\""))
            .unwrap_or_default();
        let env_yaml = environment
            .map(|e| format!("    environment: \"{e}\""))
            .unwrap_or_default();
        let yaml = format!(
            r#"
azure:
  subscription_id: "sub-test"
  resource_group: "rg-test"
  region: "swedencentral"
  naming:
{prefix_yaml}
{env_yaml}
ai:
  provider: azure_openai
  endpoint: "https://test.openai.azure.com"
  embedding:
    deployment: "text-embedding-3-large"
    dimensions: 3072
  chat:
    deployment: "gpt-4.1-mini"
    model_name: "gpt-4.1-mini"
sources: []
deployments: []
"#
        );
        serde_yaml::from_str(&yaml).unwrap()
    }

    #[test]
    fn container_app_name_uses_defaults_when_naming_absent() {
        let cfg = minimal_config(None, None);
        let name = container_app_name(&cfg, "ingest");
        assert_eq!(name, "quelch-prod-ingest");
    }

    #[test]
    fn container_app_name_uses_configured_prefix_and_env() {
        let cfg = minimal_config(Some("myapp"), Some("staging"));
        let name = container_app_name(&cfg, "worker");
        assert_eq!(name, "myapp-staging-worker");
    }

    #[test]
    fn azure_resource_name_with_suffix() {
        let cfg = minimal_config(Some("quelch"), Some("prod"));
        let name = azure_resource_name(&cfg, "ingest", Some("id"));
        assert_eq!(name, "quelch-prod-ingest-id");
    }

    #[test]
    fn azure_resource_name_without_suffix() {
        let cfg = minimal_config(Some("quelch"), Some("prod"));
        let name = azure_resource_name(&cfg, "mcp", None);
        assert_eq!(name, "quelch-prod-mcp");
    }
}
