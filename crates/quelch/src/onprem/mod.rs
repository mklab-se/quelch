/// On-premises deployment artefact generator.
///
/// `quelch generate-deployment <name> --target [docker|systemd|k8s]` dispatches here.
pub mod common;
pub mod docker;
pub mod k8s;
pub mod systemd;

use crate::config::Config;
use std::path::Path;
use thiserror::Error;

/// Which on-prem flavour to generate.
#[derive(Debug, Clone, Copy)]
pub enum OnpremTarget {
    Docker,
    Systemd,
    K8s,
}

/// What was written during a generate call.
pub struct GenerateOutcome {
    /// All file paths that were written, relative to the output directory.
    pub written: Vec<std::path::PathBuf>,
}

/// Errors that can occur during deployment artefact generation.
#[derive(Debug, Error)]
pub enum GenerateError {
    #[error("config: {0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("template: {0}")]
    Template(String),
    #[error("serialization: {0}")]
    Serialization(#[from] serde_yaml::Error),
}

/// Generate on-prem deployment artefacts for the named deployment.
///
/// # Steps
/// 1. Slice the config for the named deployment.
/// 2. Warn if the deployment is not `target: onprem` (but allow it — the artefacts are still useful).
/// 3. Dispatch to the appropriate target writer.
/// 4. Always write `effective-config.yaml`, `.env.example`, and `README.md`.
///
/// # Errors
/// Returns [`GenerateError`] on config errors, I/O errors, or template errors.
pub fn generate(
    config: &Config,
    deployment_name: &str,
    target: OnpremTarget,
    output_dir: &Path,
) -> Result<GenerateOutcome, GenerateError> {
    // 1. Slice config for the named deployment.
    let sliced = crate::config::slice::for_deployment(config, deployment_name)?;

    // 2. Warn if not onprem target.
    let dep = &sliced.deployments[0];
    if !matches!(dep.target, crate::config::DeploymentTarget::Onprem) {
        tracing::warn!(
            deployment = %deployment_name,
            "deployment target is not 'onprem'; artefacts are still useful for testing or migration"
        );
    }

    std::fs::create_dir_all(output_dir)?;

    // 3. Common artefacts.
    let common_paths = common::write_common(&sliced, output_dir)?;

    // 4. Target-specific artefacts.
    let target_paths = match target {
        OnpremTarget::Docker => docker::write(&sliced, deployment_name, output_dir)?,
        OnpremTarget::Systemd => systemd::write(&sliced, deployment_name, output_dir)?,
        OnpremTarget::K8s => k8s::write(&sliced, deployment_name, output_dir)?,
    };

    let written = common_paths.into_iter().chain(target_paths).collect();
    Ok(GenerateOutcome { written })
}
