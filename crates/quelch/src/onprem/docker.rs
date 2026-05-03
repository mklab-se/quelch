/// Docker Compose artefact writer for on-prem deployments.
use super::GenerateError;
use crate::config::Config;
use std::path::{Path, PathBuf};

/// Write Docker Compose artefacts to `output_dir`.
///
/// Files written:
/// - `docker-compose.yaml`
/// - `README.md`
///
/// Returns the list of paths written (not including common artefacts).
pub fn write(
    _sliced: &Config,
    deployment_name: &str,
    output_dir: &Path,
) -> Result<Vec<PathBuf>, GenerateError> {
    let mut written = Vec::new();

    let compose_path = output_dir.join("docker-compose.yaml");
    std::fs::write(&compose_path, docker_compose(deployment_name))?;
    written.push(compose_path);

    let readme_path = output_dir.join("README.md");
    std::fs::write(&readme_path, readme(deployment_name))?;
    written.push(readme_path);

    Ok(written)
}

fn docker_compose(deployment_name: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    format!(
        r#"version: '3.8'

services:
  ingest:
    image: ghcr.io/mklab-se/quelch:{version}
    restart: unless-stopped
    env_file: .env
    volumes:
      - ./effective-config.yaml:/etc/quelch/quelch.yaml:ro
    command: ["ingest", "--config", "/etc/quelch/quelch.yaml", "--deployment", "{deployment_name}"]
"#
    )
}

fn readme(deployment_name: &str) -> String {
    format!(
        r#"# Quelch — Docker on-prem deployment: {deployment_name}

## Prerequisites

- Docker and Docker Compose installed on the host.
- The host must be able to reach your Jira / Confluence URLs.
- The host must be able to reach your Azure Cosmos DB endpoint.

## Setup

1. Copy `.env.example` to `.env` and fill in the values:
   ```sh
   cp .env.example .env
   # Edit .env with your credentials
   ```

2. Review `effective-config.yaml` — this is the sliced config for this deployment.
   It is mounted read-only into the container; do not edit while the container is running.

3. Start the worker:
   ```sh
   docker compose up -d
   ```

4. Check logs:
   ```sh
   docker compose logs -f
   ```

## Updating

Pull the latest image and restart:
```sh
docker compose pull
docker compose up -d
```

## Stopping

```sh
docker compose down
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
openai:
  endpoint: "https://test.openai.azure.com"
  embedding_deployment: "text-embedding-3-large"
  embedding_dimensions: 3072
sources:
  - type: jira
    name: jira-test
    url: "https://example.atlassian.net"
    auth:
      email: "user@example.com"
      api_token: "tok"
    projects: ["TEST"]
deployments:
  - name: ingest-onprem
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
        write(&config, "ingest-onprem", dir.path()).unwrap();

        assert!(dir.path().join("docker-compose.yaml").exists());
        assert!(dir.path().join("README.md").exists());
    }

    #[test]
    fn compose_contains_deployment_name() {
        let compose = docker_compose("my-ingest");
        assert!(compose.contains("--deployment"));
        assert!(compose.contains("my-ingest"));
    }

    #[test]
    fn compose_contains_image_tag() {
        let compose = docker_compose("test");
        assert!(compose.contains("ghcr.io/mklab-se/quelch:"));
        // Image tag should contain a version number (not empty).
        let version = env!("CARGO_PKG_VERSION");
        assert!(compose.contains(version));
    }

    #[test]
    fn compose_is_valid_yaml() {
        let compose = docker_compose("test");
        serde_yaml::from_str::<serde_yaml::Value>(&compose)
            .expect("docker-compose.yaml must be valid YAML");
    }
}
