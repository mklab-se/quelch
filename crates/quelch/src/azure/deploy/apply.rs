/// Wrapper around `az deployment group create` that applies a Bicep template.
///
/// Entry point: [`run`].
use std::path::Path;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The outcome of a successful `az deployment group create` call.
#[derive(Debug)]
pub struct ApplyOutcome {
    /// The `provisioningState` field from the ARM response, e.g. `"Succeeded"`.
    pub provisioning_state: String,
    /// Raw JSON body returned by `az`.
    pub raw: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur while running `az deployment group create`.
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("az command failed (exit {code}): {stderr}")]
    AzFailed { code: i32, stderr: String },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run `az deployment group create` to apply a Bicep template.
///
/// This shells out to the `az` CLI, parses its JSON output, and returns the
/// provisioning state. It will fail if `az` is not installed or the user is
/// not logged in.
///
/// The `--no-prompt` flag prevents `az` from asking for parameter values
/// interactively; callers must supply all required parameters in the template
/// or via `--parameters`.
pub fn run(resource_group: &str, bicep_path: &Path) -> Result<ApplyOutcome, ApplyError> {
    let output = std::process::Command::new("az")
        .args([
            "deployment",
            "group",
            "create",
            "--resource-group",
            resource_group,
            "--template-file",
            bicep_path.to_str().unwrap_or(""),
            "--no-prompt",
            "true",
            "--output",
            "json",
        ])
        .output()?;

    if !output.status.success() {
        return Err(ApplyError::AzFailed {
            code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let result: serde_json::Value = serde_json::from_slice(&output.stdout)?;

    let provisioning_state = result
        .pointer("/properties/provisioningState")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();

    Ok(ApplyOutcome {
        provisioning_state,
        raw: result,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that `ApplyError::AzFailed` formats correctly.
    #[test]
    fn apply_error_formats_code_and_stderr() {
        let err = ApplyError::AzFailed {
            code: 1,
            stderr: "some error".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("exit 1"), "error must mention exit code");
        assert!(msg.contains("some error"), "error must include stderr");
    }
}
