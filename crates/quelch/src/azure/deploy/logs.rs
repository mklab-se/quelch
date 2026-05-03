/// Wrapper around `az containerapp logs show` for tailing Container App logs.
///
/// Entry point: [`tail`].
///
/// The `az containerapp logs show --follow` flag instructs `az` to stream
/// logs to stdout. This module inherits `az`'s stdout/stderr directly so
/// the operator sees live output in their terminal.
use std::process::Command;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur while tailing Container App logs.
#[derive(Debug, thiserror::Error)]
pub enum LogsError {
    #[error("az command failed: {0}")]
    AzFailed(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Tail logs from a Container App.
///
/// This shells out to:
/// ```text
/// az containerapp logs show \
///   --name <container_app_name> \
///   --resource-group <resource_group> \
///   --tail <tail> \
///   [--follow] \
///   [--since <since>]
/// ```
///
/// When `follow` is `true`, `az` streams logs and does not exit until the user
/// presses Ctrl-C. The process inherits stdin/stdout/stderr so the operator
/// sees live output directly.
///
/// `since` accepts the same duration strings that `az` accepts, e.g. `"1h"`,
/// `"30m"`, `"2023-01-01T00:00:00Z"`.
pub fn tail(
    container_app_name: &str,
    resource_group: &str,
    tail: usize,
    follow: bool,
    since: Option<&str>,
) -> Result<(), LogsError> {
    let tail_str = tail.to_string();
    let mut args = vec![
        "containerapp",
        "logs",
        "show",
        "--name",
        container_app_name,
        "--resource-group",
        resource_group,
        "--tail",
        &tail_str,
    ];

    if follow {
        args.push("--follow");
    }

    // Capture `since` as an owned String so it lives long enough for the
    // args slice to hold a reference.
    let since_owned;
    if let Some(s) = since {
        since_owned = s.to_string();
        args.push("--since");
        args.push(&since_owned);
    }

    // Inherit stdin/stdout/stderr so the operator sees streaming output
    // directly. Do not buffer.
    let status = Command::new("az").args(&args).status()?;

    if !status.success() {
        return Err(LogsError::AzFailed(format!(
            "logs command failed for Container App '{container_app_name}'"
        )));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_error_formats_correctly() {
        let err = LogsError::AzFailed("app not found".to_string());
        assert!(err.to_string().contains("az command failed"));
        assert!(err.to_string().contains("app not found"));
    }
}
