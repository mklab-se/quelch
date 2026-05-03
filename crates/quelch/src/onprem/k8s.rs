/// Kubernetes manifest artefact writer for on-prem deployments.
///
/// Writes a minimal set of Kubernetes resources for a pull-only ingest worker
/// (no ingress, no service — ingest is pull-only):
/// - `Deployment.yaml`
/// - `ConfigMap.yaml`
/// - `Secret.example.yaml`
/// - `kustomization.yaml`
/// - `README.md`
use super::GenerateError;
use crate::config::Config;
use std::path::{Path, PathBuf};

/// Write Kubernetes artefacts to `output_dir`.
///
/// Returns the list of paths written (not including common artefacts).
pub fn write(
    _sliced: &Config,
    deployment_name: &str,
    output_dir: &Path,
) -> Result<Vec<PathBuf>, GenerateError> {
    let mut written = Vec::new();

    let deployment_path = output_dir.join("Deployment.yaml");
    std::fs::write(&deployment_path, deployment_yaml(deployment_name))?;
    written.push(deployment_path);

    let configmap_path = output_dir.join("ConfigMap.yaml");
    std::fs::write(&configmap_path, configmap_yaml(deployment_name))?;
    written.push(configmap_path);

    let secret_path = output_dir.join("Secret.example.yaml");
    std::fs::write(&secret_path, secret_example_yaml(deployment_name))?;
    written.push(secret_path);

    let kustomization_path = output_dir.join("kustomization.yaml");
    std::fs::write(&kustomization_path, kustomization_yaml(deployment_name))?;
    written.push(kustomization_path);

    let readme_path = output_dir.join("README.md");
    std::fs::write(&readme_path, readme(deployment_name))?;
    written.push(readme_path);

    Ok(written)
}

fn deployment_yaml(deployment_name: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let safe_name = deployment_name.to_lowercase().replace('_', "-");
    format!(
        r#"apiVersion: apps/v1
kind: Deployment
metadata:
  name: quelch-{safe_name}
  labels:
    app: quelch-{safe_name}
spec:
  replicas: 1
  selector:
    matchLabels:
      app: quelch-{safe_name}
  template:
    metadata:
      labels:
        app: quelch-{safe_name}
    spec:
      containers:
        - name: ingest
          image: ghcr.io/mklab-se/quelch:{version}
          command: ["ingest", "--config", "/etc/quelch/quelch.yaml", "--deployment", "{deployment_name}"]
          envFrom:
            - secretRef:
                name: quelch-{safe_name}-env
          volumeMounts:
            - name: config
              mountPath: /etc/quelch
              readOnly: true
      volumes:
        - name: config
          configMap:
            name: quelch-{safe_name}-config
"#
    )
}

fn configmap_yaml(deployment_name: &str) -> String {
    let safe_name = deployment_name.to_lowercase().replace('_', "-");
    format!(
        r#"apiVersion: v1
kind: ConfigMap
metadata:
  name: quelch-{safe_name}-config
data:
  quelch.yaml: |
    # Paste the contents of effective-config.yaml here.
    # Alternatively, generate this ConfigMap from the file:
    #   kubectl create configmap quelch-{safe_name}-config --from-file=quelch.yaml=effective-config.yaml
"#
    )
}

fn secret_example_yaml(deployment_name: &str) -> String {
    let safe_name = deployment_name.to_lowercase().replace('_', "-");
    format!(
        r#"# Example Secret for quelch-{safe_name}.
# Fill in the base64-encoded values (echo -n "value" | base64) and apply:
#   kubectl apply -f Secret.example.yaml
#
# Or use kubectl directly:
#   kubectl create secret generic quelch-{safe_name}-env \
#     --from-literal=JIRA_INTERNAL_PAT=<value> \
#     --from-literal=COSMOS_ENDPOINT=<value> \
#     --from-literal=COSMOS_KEY=<value>
apiVersion: v1
kind: Secret
metadata:
  name: quelch-{safe_name}-env
type: Opaque
stringData:
  # Source credentials
  JIRA_INTERNAL_PAT: "<your-jira-pat>"
  # Azure Cosmos DB
  COSMOS_ENDPOINT: "https://YOUR-COSMOS-ACCOUNT.documents.azure.com:443/"
  COSMOS_KEY: "<your-cosmos-key>"
  # Optional
  # HTTPS_PROXY: ""
"#
    )
}

fn kustomization_yaml(deployment_name: &str) -> String {
    let safe_name = deployment_name.to_lowercase().replace('_', "-");
    format!(
        r#"# Apply all resources: kubectl apply -k .
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization

resources:
  - ConfigMap.yaml
  - Deployment.yaml

# Apply the Secret separately (it is not checked in):
#   kubectl apply -f Secret.example.yaml  # after filling in values
#
# Or reference it here if you manage secrets in-cluster:
# secretGenerator:
#   - name: quelch-{safe_name}-env
#     envs:
#       - .env
"#
    )
}

fn readme(deployment_name: &str) -> String {
    let safe_name = deployment_name.to_lowercase().replace('_', "-");
    format!(
        r#"# Quelch — Kubernetes on-prem deployment: {deployment_name}

## Prerequisites

- A Kubernetes cluster with `kubectl` configured.
- The cluster nodes can reach your Jira / Confluence URLs.
- The cluster nodes can reach your Azure Cosmos DB endpoint.

## Files

| File | Purpose |
|---|---|
| `Deployment.yaml` | The ingest worker Deployment (1 replica) |
| `ConfigMap.yaml` | Mounts `effective-config.yaml` into the pod |
| `Secret.example.yaml` | Template for credential Secret — **fill in and apply manually** |
| `kustomization.yaml` | Ties resources together for `kubectl apply -k .` |
| `effective-config.yaml` | The sliced quelch config for this deployment |

## Setup

1. Fill in `ConfigMap.yaml` with the contents of `effective-config.yaml`,
   or create it directly:
   ```sh
   kubectl create configmap quelch-{safe_name}-config \
     --from-file=quelch.yaml=effective-config.yaml
   ```

2. Create the Secret with your credentials:
   ```sh
   kubectl create secret generic quelch-{safe_name}-env \
     --from-literal=JIRA_INTERNAL_PAT=<value> \
     --from-literal=COSMOS_ENDPOINT=https://YOUR-COSMOS.documents.azure.com:443/ \
     --from-literal=COSMOS_KEY=<value>
   ```

3. Apply the Deployment:
   ```sh
   kubectl apply -k .
   ```

4. Check the worker:
   ```sh
   kubectl get pods -l app=quelch-{safe_name}
   kubectl logs -l app=quelch-{safe_name} -f
   ```

## Updating

```sh
kubectl rollout restart deployment/quelch-{safe_name}
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
    url: "https://jira.internal.example"
    auth:
      pat: "my-pat"
    projects: ["INT"]
deployments:
  - name: ingest-k8s
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
        write(&config, "ingest-k8s", dir.path()).unwrap();

        assert!(dir.path().join("Deployment.yaml").exists());
        assert!(dir.path().join("ConfigMap.yaml").exists());
        assert!(dir.path().join("Secret.example.yaml").exists());
        assert!(dir.path().join("kustomization.yaml").exists());
        assert!(dir.path().join("README.md").exists());
    }

    #[test]
    fn deployment_yaml_contains_deployment_name() {
        let yaml = deployment_yaml("my-worker");
        assert!(yaml.contains("--deployment"));
        assert!(yaml.contains("my-worker"));
        assert!(yaml.contains("ghcr.io/mklab-se/quelch:"));
    }

    #[test]
    fn deployment_yaml_is_valid_yaml() {
        let yaml = deployment_yaml("test");
        serde_yaml::from_str::<serde_yaml::Value>(&yaml)
            .expect("Deployment.yaml must be valid YAML");
    }

    #[test]
    fn configmap_yaml_is_valid_yaml() {
        let yaml = configmap_yaml("test");
        serde_yaml::from_str::<serde_yaml::Value>(&yaml)
            .expect("ConfigMap.yaml must be valid YAML");
    }

    #[test]
    fn kustomization_yaml_is_valid_yaml() {
        let yaml = kustomization_yaml("test");
        serde_yaml::from_str::<serde_yaml::Value>(&yaml)
            .expect("kustomization.yaml must be valid YAML");
    }
}
