/// Common artefacts written for every on-prem target:
/// - `effective-config.yaml` — sliced config for the deployment
/// - `.env.example` — every env var referenced in the sliced config
/// - (README is target-specific)
use super::GenerateError;
use crate::config::Config;
use std::path::{Path, PathBuf};

/// Write `effective-config.yaml` and `.env.example` into `output_dir`.
///
/// Returns the list of paths written.
pub fn write_common(sliced: &Config, output_dir: &Path) -> Result<Vec<PathBuf>, GenerateError> {
    let mut written = Vec::new();

    // --- effective-config.yaml ---
    let effective_yaml = serde_yaml::to_string(sliced)?;
    let effective_path = output_dir.join("effective-config.yaml");
    std::fs::write(&effective_path, effective_yaml)?;
    written.push(effective_path);

    // --- .env.example ---
    let env_example = build_env_example(sliced);
    let env_path = output_dir.join(".env.example");
    std::fs::write(&env_path, env_example)?;
    written.push(env_path);

    Ok(written)
}

/// Extract all `${VAR_NAME}` references from the YAML representation of the
/// sliced config and build an `.env.example` file.
pub fn build_env_example(sliced: &Config) -> String {
    let yaml = serde_yaml::to_string(sliced).unwrap_or_default();
    let vars = extract_env_vars(&yaml);

    let mut lines = Vec::new();
    lines.push("# Environment variables required by this Quelch deployment.".to_string());
    lines.push("# Copy to .env and fill in the values.".to_string());
    lines.push(String::new());

    if vars.is_empty() {
        lines.push("# (No environment variable references found in config)".to_string());
    } else {
        // Group: source credentials first, then infra.
        let source_vars: Vec<_> = vars
            .iter()
            .filter(|v| {
                let u = v.to_uppercase();
                u.contains("PAT")
                    || u.contains("TOKEN")
                    || u.contains("API_TOKEN")
                    || u.contains("EMAIL")
                    || u.contains("JIRA")
                    || u.contains("CONFLUENCE")
            })
            .cloned()
            .collect();
        let infra_vars: Vec<_> = vars
            .iter()
            .filter(|v| !source_vars.contains(v))
            .cloned()
            .collect();

        if !source_vars.is_empty() {
            lines.push("# Source-system credentials".to_string());
            for v in &source_vars {
                lines.push(format!("{v}="));
            }
            lines.push(String::new());
        }
        if !infra_vars.is_empty() {
            lines.push("# Infrastructure".to_string());
            for v in &infra_vars {
                lines.push(format!("{v}="));
            }
            lines.push(String::new());
        }
    }

    // Always suggest COSMOS vars since the worker needs them at runtime.
    lines.push("# Cosmos DB (worker writes here at runtime)".to_string());
    lines
        .push("# COSMOS_ENDPOINT=https://YOUR-COSMOS-ACCOUNT.documents.azure.com:443/".to_string());
    lines.push("# COSMOS_KEY=".to_string());
    lines.push(String::new());

    lines.push("# Optional".to_string());
    lines.push("# HTTPS_PROXY=".to_string());

    lines.join("\n")
}

/// Parse `${VAR_NAME}` patterns from a YAML string and return the unique
/// variable names in the order they first appear.
pub fn extract_env_vars(yaml: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut vars = Vec::new();

    let mut chars = yaml.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut name = String::new();
            for c in chars.by_ref() {
                if c == '}' {
                    break;
                }
                name.push(c);
            }
            let name = name.trim().to_string();
            if !name.is_empty() && seen.insert(name.clone()) {
                vars.push(name);
            }
        }
    }

    vars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_env_vars_finds_refs() {
        let yaml = r#"
subscription_id: "${AZURE_SUB}"
api_token: "${JIRA_API_TOKEN}"
pat: "${JIRA_API_TOKEN}"
"#;
        let vars = extract_env_vars(yaml);
        // JIRA_API_TOKEN appears twice but should only appear once.
        assert_eq!(vars.len(), 2);
        assert!(vars.contains(&"AZURE_SUB".to_string()));
        assert!(vars.contains(&"JIRA_API_TOKEN".to_string()));
    }

    #[test]
    fn extract_env_vars_empty_when_none() {
        let vars = extract_env_vars("plain: value\nno_refs: here\n");
        assert!(vars.is_empty());
    }

    #[test]
    fn build_env_example_smoke() {
        let yaml = r#"
azure:
  subscription_id: "${AZURE_SUBSCRIPTION_ID}"
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
    name: j
    url: "https://example.atlassian.net"
    auth:
      email: "user@example.com"
      api_token: "${JIRA_API_TOKEN}"
    projects: ["X"]
deployments: []
"#;
        let cfg: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
        let example = build_env_example(&cfg);
        assert!(example.contains("JIRA_API_TOKEN="));
        assert!(example.contains("COSMOS_ENDPOINT"));
    }
}
