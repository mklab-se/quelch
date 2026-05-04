//! `quelch mcp-key set / rotate / show` — manage the Q-MCP API key.
//!
//! Q-MCP reads the expected API key from `QUELCH_MCP_API_KEY` at startup.
//! Where that value lives depends on the deployment target:
//!
//! - **`target: azure`** — Container App secret reference pointing at a Key
//!   Vault secret named `quelch-mcp-api-key`. We read/write the secret via
//!   `az keyvault secret set / show` against the deployment's Key Vault.
//! - **`target: onprem`** — env var supplied by the on-prem operator's
//!   secret store (`.env`, systemd EnvironmentFile, k8s `Secret`, etc.). We
//!   don't try to remote-write into that — instead we generate a value and
//!   print instructions for the operator to apply locally.
//!
//! Restart of the Container App revision after a `set` / `rotate` is
//! best-effort. Container Apps auto-rolls when secret values change, but it
//! takes a minute; we issue an explicit `revision restart` so the new value
//! is live by the time the command returns.

use anyhow::{Context, Result, anyhow};
use rand::RngCore;
use rand::rngs::OsRng;
use std::process::Command;

use crate::config::{Config, DeploymentConfig, DeploymentRole, DeploymentTarget};

/// Canonical name of the Key Vault secret that holds the Q-MCP API key.
const KV_SECRET_NAME: &str = "quelch-mcp-api-key";

/// `set` — store a value (auto-generated unless `value` is `Some`).
pub async fn set(
    config: &Config,
    deployment_name: &str,
    value: Option<String>,
    quiet: bool,
) -> Result<()> {
    let deployment = find_mcp_deployment(config, deployment_name)?;
    let key = match value {
        Some(v) => v,
        None => generate_key()?,
    };
    write_key(config, deployment, &key).await?;
    if !quiet {
        announce_key(&key);
    }
    Ok(())
}

/// `rotate` — generate a fresh key and replace the stored value.
pub async fn rotate(config: &Config, deployment_name: &str, quiet: bool) -> Result<()> {
    let deployment = find_mcp_deployment(config, deployment_name)?;
    let key = generate_key()?;
    write_key(config, deployment, &key).await?;
    if !quiet {
        announce_key(&key);
    }
    Ok(())
}

/// `show` — print the currently-stored value (Azure only — for on-prem the
/// value lives wherever the operator put it, we can't read it back).
pub async fn show(config: &Config, deployment_name: &str) -> Result<()> {
    let deployment = find_mcp_deployment(config, deployment_name)?;
    match deployment.target {
        DeploymentTarget::Azure => {
            let kv_name = key_vault_name(config);
            let key = read_kv_secret(&kv_name)
                .with_context(|| format!("read {KV_SECRET_NAME} from {kv_name}"))?;
            println!("{key}");
            Ok(())
        }
        DeploymentTarget::Onprem => {
            anyhow::bail!(
                "deployment '{}' has target: onprem — the API key lives in your local \
                 secret store (env / .env / k8s Secret) and Quelch can't read it back \
                 over the network. Check whichever store you used when running \
                 `quelch mcp-key set` originally.",
                deployment.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn find_mcp_deployment<'a>(config: &'a Config, name: &str) -> Result<&'a DeploymentConfig> {
    let deployment = config
        .deployments
        .iter()
        .find(|d| d.name == name)
        .ok_or_else(|| anyhow!("deployment '{name}' not found in config"))?;
    if !matches!(deployment.role, DeploymentRole::Mcp) {
        anyhow::bail!(
            "deployment '{name}' has role '{:?}' — `mcp-key` only applies to role: mcp deployments",
            deployment.role
        );
    }
    Ok(deployment)
}

fn generate_key() -> Result<String> {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    use base64::Engine;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

async fn write_key(config: &Config, deployment: &DeploymentConfig, key: &str) -> Result<()> {
    match deployment.target {
        DeploymentTarget::Azure => {
            let kv_name = key_vault_name(config);
            write_kv_secret(&kv_name, key)
                .with_context(|| format!("write {KV_SECRET_NAME} to {kv_name}"))?;
            // Best-effort revision restart so the running Container App sees
            // the new value within seconds rather than waiting for the next
            // auto-rollout.
            if let Err(e) = restart_container_app_revision(config, &deployment.name) {
                eprintln!("warning: secret stored, but Container App revision restart failed: {e}");
                eprintln!(
                    "         the new value will roll out automatically within a few minutes."
                );
            }
            Ok(())
        }
        DeploymentTarget::Onprem => {
            print_onprem_instructions(&deployment.name, key);
            Ok(())
        }
    }
}

fn key_vault_name(config: &Config) -> String {
    let prefix = config.azure.naming.prefix.as_deref().unwrap_or("quelch");
    let env = config.azure.naming.environment.as_deref().unwrap_or("prod");
    config
        .azure
        .resources
        .key_vault
        .clone()
        .unwrap_or_else(|| format!("{prefix}-{env}-kv"))
}

fn announce_key(key: &str) {
    println!("Q-MCP API key:");
    println!("  {key}");
    println!();
    println!("Configure your agent with this value as the bearer token.");
}

// ---------------------------------------------------------------------------
// `az` shell-outs
// ---------------------------------------------------------------------------

fn write_kv_secret(vault: &str, value: &str) -> Result<()> {
    let output = Command::new("az")
        .args([
            "keyvault",
            "secret",
            "set",
            "--vault-name",
            vault,
            "--name",
            KV_SECRET_NAME,
            "--value",
            value,
            "--output",
            "none",
        ])
        .output()
        .context("`az keyvault secret set` failed to spawn — is the Azure CLI installed?")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("`az keyvault secret set` failed: {stderr}");
    }
    Ok(())
}

fn read_kv_secret(vault: &str) -> Result<String> {
    let output = Command::new("az")
        .args([
            "keyvault",
            "secret",
            "show",
            "--vault-name",
            vault,
            "--name",
            KV_SECRET_NAME,
            "--query",
            "value",
            "-o",
            "tsv",
        ])
        .output()
        .context("`az keyvault secret show` failed to spawn — is the Azure CLI installed?")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("`az keyvault secret show` failed: {stderr}");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn restart_container_app_revision(config: &Config, deployment_name: &str) -> Result<()> {
    let prefix = config.azure.naming.prefix.as_deref().unwrap_or("quelch");
    let env = config.azure.naming.environment.as_deref().unwrap_or("prod");
    let app_name = format!("{prefix}-{env}-{deployment_name}");
    let rg = &config.azure.resource_group;

    // List the active revision name.
    let list = Command::new("az")
        .args([
            "containerapp",
            "revision",
            "list",
            "--resource-group",
            rg,
            "--name",
            &app_name,
            "--query",
            "[?properties.active].name | [0]",
            "-o",
            "tsv",
        ])
        .output()
        .context("`az containerapp revision list` failed to spawn")?;
    if !list.status.success() {
        anyhow::bail!(
            "list active revision: {}",
            String::from_utf8_lossy(&list.stderr)
        );
    }
    let revision = String::from_utf8_lossy(&list.stdout).trim().to_string();
    if revision.is_empty() {
        anyhow::bail!("no active revision found for Container App '{app_name}'");
    }

    let restart = Command::new("az")
        .args([
            "containerapp",
            "revision",
            "restart",
            "--resource-group",
            rg,
            "--name",
            &app_name,
            "--revision",
            &revision,
            "--output",
            "none",
        ])
        .output()
        .context("`az containerapp revision restart` failed to spawn")?;
    if !restart.status.success() {
        anyhow::bail!(
            "restart revision '{revision}': {}",
            String::from_utf8_lossy(&restart.stderr)
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// On-prem flow
// ---------------------------------------------------------------------------

fn print_onprem_instructions(deployment_name: &str, key: &str) {
    println!("Generated Q-MCP API key for deployment '{deployment_name}':");
    println!();
    println!("  {key}");
    println!();
    println!("This deployment has `target: onprem` — Quelch can't write it to your");
    println!("local secret store from here. Apply it to whichever supervisor runs Q-MCP:");
    println!();
    println!("  # docker compose:");
    println!("  echo \"QUELCH_MCP_API_KEY={key}\" >> .env");
    println!("  docker compose up -d --force-recreate quelch-mcp");
    println!();
    println!("  # systemd:");
    println!("  sudo sed -i \"s|QUELCH_MCP_API_KEY=.*|QUELCH_MCP_API_KEY={key}|\" \\");
    println!("    /etc/quelch/quelch-mcp-{deployment_name}.env");
    println!("  sudo systemctl restart quelch-mcp-{deployment_name}");
    println!();
    println!("  # kubernetes:");
    println!("  kubectl create secret generic quelch-mcp-secrets \\");
    println!("    --from-literal=QUELCH_MCP_API_KEY=\"{key}\" \\");
    println!("    --dry-run=client -o yaml | kubectl apply -f -");
    println!("  kubectl rollout restart deploy/quelch-mcp");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_key_is_44_base64_chars_for_32_random_bytes() {
        let k = generate_key().unwrap();
        // 32 bytes encoded as standard base64 is 44 chars (incl. padding).
        assert_eq!(k.len(), 44);
        assert!(
            k.ends_with('='),
            "standard base64 of 32 bytes ends with padding"
        );
    }

    #[test]
    fn generate_key_returns_distinct_values() {
        let a = generate_key().unwrap();
        let b = generate_key().unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn key_vault_name_uses_explicit_override_when_set() {
        let cfg: Config = serde_yaml::from_str(
            r#"
azure:
  subscription_id: "s"
  resource_group: "rg"
  region: "swedencentral"
  naming:
    prefix: "p"
    environment: "e"
  resources:
    key_vault: "explicit-kv"
ai:
  provider: azure_openai
  endpoint: "https://x.openai.azure.com"
  embedding:
    deployment: "te"
    dimensions: 1536
  chat:
    deployment: "gpt-5-mini"
    model_name: "gpt-5-mini"
deployments: []
"#,
        )
        .unwrap();
        assert_eq!(key_vault_name(&cfg), "explicit-kv");
    }

    #[test]
    fn key_vault_name_falls_back_to_naming_convention() {
        let cfg: Config = serde_yaml::from_str(
            r#"
azure:
  subscription_id: "s"
  resource_group: "rg"
  region: "swedencentral"
  naming:
    prefix: "myapp"
    environment: "stage"
ai:
  provider: azure_openai
  endpoint: "https://x.openai.azure.com"
  embedding:
    deployment: "te"
    dimensions: 1536
  chat:
    deployment: "gpt-5-mini"
    model_name: "gpt-5-mini"
deployments: []
"#,
        )
        .unwrap();
        assert_eq!(key_vault_name(&cfg), "myapp-stage-kv");
    }

    #[test]
    fn find_mcp_deployment_rejects_ingest_role() {
        let cfg: Config = serde_yaml::from_str(
            r#"
azure:
  subscription_id: "s"
  resource_group: "rg"
  region: "swedencentral"
ai:
  provider: azure_openai
  endpoint: "https://x.openai.azure.com"
  embedding:
    deployment: "te"
    dimensions: 1536
  chat:
    deployment: "gpt-5-mini"
    model_name: "gpt-5-mini"
sources:
  - type: jira
    name: jira-cloud
    url: "https://example.atlassian.net"
    auth:
      email: "u@example.com"
      api_token: "t"
    projects: ["X"]
deployments:
  - name: ingest-only
    role: ingest
    target: onprem
    sources:
      - source: jira-cloud
"#,
        )
        .unwrap();
        let err = find_mcp_deployment(&cfg, "ingest-only").unwrap_err();
        assert!(err.to_string().contains("only applies to role: mcp"));
    }

    #[test]
    fn find_mcp_deployment_returns_clear_error_when_missing() {
        let cfg: Config = serde_yaml::from_str(
            r#"
azure:
  subscription_id: "s"
  resource_group: "rg"
  region: "swedencentral"
ai:
  provider: azure_openai
  endpoint: "https://x.openai.azure.com"
  embedding:
    deployment: "te"
    dimensions: 1536
  chat:
    deployment: "gpt-5-mini"
    model_name: "gpt-5-mini"
deployments: []
"#,
        )
        .unwrap();
        let err = find_mcp_deployment(&cfg, "ghost").unwrap_err();
        assert!(err.to_string().contains("ghost"));
        assert!(err.to_string().contains("not found"));
    }
}
