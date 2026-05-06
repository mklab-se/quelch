/// Wrappers around `az search indexer` commands.
///
/// Provides run, reset, and status operations on Azure AI Search Indexers
/// by shelling out to the `az` CLI.
///
/// Entry points: [`run`], [`reset`], [`status`].
use std::process::Command;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Status information for a single Azure AI Search indexer.
#[derive(Debug, Clone)]
pub struct IndexerStatus {
    /// Indexer name.
    pub name: String,
    /// Last execution result status (e.g. `"success"`, `"transientFailure"`).
    pub last_result: Option<String>,
    /// Timestamp of the last run.
    pub last_run_at: Option<chrono::DateTime<chrono::Utc>>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur while operating Azure AI Search indexers.
#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    #[error("az command failed: {0}")]
    AzFailed(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Trigger an immediate run of the named indexer.
///
/// Equivalent to: `az search indexer run --service-name X --name Y`
pub fn run(search_service: &str, indexer_name: &str) -> Result<(), IndexerError> {
    let status = Command::new("az")
        .args([
            "search",
            "indexer",
            "run",
            "--service-name",
            search_service,
            "--name",
            indexer_name,
        ])
        .status()?;

    if !status.success() {
        return Err(IndexerError::AzFailed(format!(
            "indexer run failed for '{indexer_name}'"
        )));
    }

    Ok(())
}

/// Reset the named indexer (forces a full re-index on the next run).
///
/// Equivalent to: `az search indexer reset --service-name X --name Y`
pub fn reset(search_service: &str, indexer_name: &str) -> Result<(), IndexerError> {
    let status = Command::new("az")
        .args([
            "search",
            "indexer",
            "reset",
            "--service-name",
            search_service,
            "--name",
            indexer_name,
        ])
        .status()?;

    if !status.success() {
        return Err(IndexerError::AzFailed(format!(
            "indexer reset failed for '{indexer_name}'"
        )));
    }

    Ok(())
}

/// List all indexers and their execution status.
///
/// Equivalent to: `az search indexer list --service-name X`
///
/// The JSON shape varies between Azure CLI versions; this function handles
/// the common case of a top-level array with `name` and
/// `lastResult.status` / `lastResult.endTime` fields.
pub fn status(search_service: &str) -> Result<Vec<IndexerStatus>, IndexerError> {
    let output = Command::new("az")
        .args([
            "search",
            "indexer",
            "list",
            "--service-name",
            search_service,
        ])
        .output()?;

    if !output.status.success() {
        return Err(IndexerError::AzFailed(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;

    let items = match json.as_array() {
        Some(arr) => arr.clone(),
        None => {
            // Some az versions nest the list under a `value` key.
            json.get("value")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        }
    };

    let mut result = Vec::with_capacity(items.len());
    for item in &items {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if name.is_empty() {
            continue;
        }

        let last_result = item
            .pointer("/lastResult/status")
            .and_then(|v| v.as_str())
            .map(String::from);

        let last_run_at = item
            .pointer("/lastResult/endTime")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<chrono::DateTime<chrono::Utc>>().ok());

        result.push(IndexerStatus {
            name,
            last_result,
            last_run_at,
        });
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parses_flat_array() {
        let json = serde_json::json!([
            {
                "name": "jira-issues",
                "lastResult": {
                    "status": "success",
                    "endTime": "2026-04-30T10:00:00Z"
                }
            },
            {
                "name": "confluence-pages",
                "lastResult": {
                    "status": "transientFailure",
                    "endTime": "2026-04-30T09:00:00Z"
                }
            }
        ]);

        // Simulate the parsing logic without shelling out.
        let items = json.as_array().unwrap();
        let parsed: Vec<IndexerStatus> = items
            .iter()
            .filter_map(|item| {
                let name = item.get("name")?.as_str()?.to_string();
                let last_result = item
                    .pointer("/lastResult/status")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let last_run_at = item
                    .pointer("/lastResult/endTime")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<chrono::DateTime<chrono::Utc>>().ok());
                Some(IndexerStatus {
                    name,
                    last_result,
                    last_run_at,
                })
            })
            .collect();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "jira-issues");
        assert_eq!(parsed[0].last_result.as_deref(), Some("success"));
        assert!(parsed[0].last_run_at.is_some());
        assert_eq!(parsed[1].name, "confluence-pages");
        assert_eq!(parsed[1].last_result.as_deref(), Some("transientFailure"));
    }

    #[test]
    fn status_parses_value_wrapped_response() {
        // Some az CLI versions return `{ "value": [...] }`.
        let json = serde_json::json!({
            "value": [
                { "name": "my-indexer", "lastResult": null }
            ]
        });

        let items = json.get("value").and_then(|v| v.as_array()).unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get("name").unwrap().as_str(), Some("my-indexer"));
    }

    #[test]
    fn indexer_error_formats_correctly() {
        let err = IndexerError::AzFailed("failed to run indexer 'x'".to_string());
        assert!(err.to_string().contains("az command failed"));
        assert!(err.to_string().contains("failed to run indexer 'x'"));
    }
}
