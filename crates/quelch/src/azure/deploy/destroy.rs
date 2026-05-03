/// Wrapper around `az containerapp delete` for removing a deployment.
///
/// Entry point: [`run`].
use std::path::Path;
use std::process::Command;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur while destroying a Container App deployment.
#[derive(Debug, thiserror::Error)]
pub enum DestroyError {
    #[error("az command failed: {0}")]
    AzFailed(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Delete a Container App and all its revisions.
///
/// Equivalent to:
/// ```text
/// az containerapp delete \
///   --name <app_name> \
///   --resource-group <resource_group> \
///   --yes
/// ```
///
/// The `--yes` flag bypasses the `az`-side interactive confirmation (the
/// Quelch-side confirmation is handled by the CLI handler before calling
/// this function).
pub fn run(app_name: &str, resource_group: &str) -> Result<(), DestroyError> {
    let status = Command::new("az")
        .args([
            "containerapp",
            "delete",
            "--name",
            app_name,
            "--resource-group",
            resource_group,
            "--yes",
        ])
        .status()?;

    if !status.success() {
        return Err(DestroyError::AzFailed(format!(
            "containerapp delete failed for '{app_name}'"
        )));
    }

    Ok(())
}

/// Clean up the last-apply snapshot file if it exists.
pub fn remove_snapshot(snapshot_path: &Path) {
    // Best-effort — ignore errors.
    let _ = std::fs::remove_file(snapshot_path);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destroy_error_formats_correctly() {
        let err = DestroyError::AzFailed("containerapp not found".to_string());
        assert!(err.to_string().contains("az command failed"));
        assert!(err.to_string().contains("containerapp not found"));
    }
}
