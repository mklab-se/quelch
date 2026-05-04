/// Systemd unit file artefact writer for on-prem deployments.
use super::GenerateError;
use crate::config::Config;
use std::path::{Path, PathBuf};

/// Write systemd artefacts to `output_dir`.
///
/// Files written:
/// - `quelch-<deployment>.service` — the systemd unit
/// - `quelch-<deployment>.env.example` — per-deployment env example (mirrors common `.env.example`)
/// - `README.md`
///
/// Returns the list of paths written (not including common artefacts).
pub fn write(
    _sliced: &Config,
    deployment_name: &str,
    output_dir: &Path,
) -> Result<Vec<PathBuf>, GenerateError> {
    let mut written = Vec::new();

    let service_path = output_dir.join(format!("quelch-{deployment_name}.service"));
    std::fs::write(&service_path, service_unit(deployment_name))?;
    written.push(service_path);

    // Systemd gets a deployment-specific env file name.
    let env_path = output_dir.join(format!("quelch-{deployment_name}.env.example"));
    std::fs::write(
        &env_path,
        "# Copy this to /etc/quelch/quelch-{}.env and fill in the values.\n# See .env.example for the full list.\n",
    )?;
    written.push(env_path);

    let readme_path = output_dir.join("README.md");
    std::fs::write(&readme_path, readme(deployment_name))?;
    written.push(readme_path);

    Ok(written)
}

fn service_unit(deployment_name: &str) -> String {
    format!(
        r#"[Unit]
Description=Quelch ingest worker ({deployment_name})
After=network.target

[Service]
Type=simple
EnvironmentFile=/etc/quelch/quelch-{deployment_name}.env
ExecStart=/usr/local/bin/quelch ingest --config /etc/quelch/{deployment_name}-config.yaml --deployment {deployment_name}
Restart=on-failure
RestartSec=5
User=quelch
Group=quelch

[Install]
WantedBy=multi-user.target
"#
    )
}

fn readme(deployment_name: &str) -> String {
    format!(
        r#"# Quelch — systemd on-prem deployment: {deployment_name}

## Prerequisites

- A Linux host with systemd.
- The `quelch` binary installed at `/usr/local/bin/quelch`.
  Download from: https://github.com/mklab-se/quelch/releases
- A `quelch` user and group created on the host:
  ```sh
  sudo useradd --system --no-create-home --shell /sbin/nologin quelch
  ```

## Setup

1. Copy `effective-config.yaml` to `/etc/quelch/{deployment_name}-config.yaml`:
   ```sh
   sudo mkdir -p /etc/quelch
   sudo cp effective-config.yaml /etc/quelch/{deployment_name}-config.yaml
   sudo chown root:quelch /etc/quelch/{deployment_name}-config.yaml
   sudo chmod 640 /etc/quelch/{deployment_name}-config.yaml
   ```

2. Copy `.env.example` to `/etc/quelch/quelch-{deployment_name}.env` and fill in values:
   ```sh
   sudo cp .env.example /etc/quelch/quelch-{deployment_name}.env
   sudo chown root:quelch /etc/quelch/quelch-{deployment_name}.env
   sudo chmod 640 /etc/quelch/quelch-{deployment_name}.env
   # Edit with your credentials
   sudo nano /etc/quelch/quelch-{deployment_name}.env
   ```

3. Install the unit file:
   ```sh
   sudo cp quelch-{deployment_name}.service /etc/systemd/system/
   sudo systemctl daemon-reload
   sudo systemctl enable quelch-{deployment_name}
   sudo systemctl start quelch-{deployment_name}
   ```

4. Check status:
   ```sh
   systemctl status quelch-{deployment_name}
   journalctl -u quelch-{deployment_name} -f
   ```

## Updating the binary

```sh
sudo systemctl stop quelch-{deployment_name}
# Replace /usr/local/bin/quelch with the new binary
sudo systemctl start quelch-{deployment_name}
```
"#
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn sample_config() -> Config {
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
    name: jira-test
    url: "https://jira.internal.example"
    auth:
      pat: "my-pat"
    projects: ["INT"]
deployments:
  - name: ingest-dc
    role: ingest
    target: onprem
    sources:
      - source: jira-test
"#;
        serde_yaml::from_str(yaml).expect("sample config must parse")
    }

    #[test]
    fn writes_expected_files() {
        let dir = tempfile::tempdir().unwrap();
        let config = sample_config();
        write(&config, "ingest-dc", dir.path()).unwrap();

        assert!(dir.path().join("quelch-ingest-dc.service").exists());
        assert!(dir.path().join("quelch-ingest-dc.env.example").exists());
        assert!(dir.path().join("README.md").exists());
    }

    #[test]
    fn service_unit_contains_deployment_name() {
        let unit = service_unit("my-worker");
        assert!(unit.contains("Description=Quelch ingest worker (my-worker)"));
        assert!(unit.contains("--deployment my-worker"));
        assert!(unit.contains("EnvironmentFile=/etc/quelch/quelch-my-worker.env"));
    }

    #[test]
    fn service_unit_has_restart_policy() {
        let unit = service_unit("test");
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("RestartSec=5"));
    }

    #[test]
    fn service_unit_runs_as_quelch_user() {
        let unit = service_unit("test");
        assert!(unit.contains("User=quelch"));
        assert!(unit.contains("Group=quelch"));
    }
}
