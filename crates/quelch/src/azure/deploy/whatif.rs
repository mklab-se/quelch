/// Wrapper around `az deployment group what-if` that parses its JSON output
/// into a structured [`WhatIfReport`].
///
/// The actual `az` shell-out is in [`run`]; the parser is exposed separately as
/// [`parse_whatif_json`] so it can be unit-tested without an `az` installation.
use std::path::Path;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The structured result of an `az deployment group what-if` run.
#[derive(Debug)]
pub struct WhatIfReport {
    /// Resources that would be created.
    pub creates: Vec<ResourceChange>,
    /// Resources that would be modified.
    pub modifies: Vec<ResourceChange>,
    /// Resources that would be deleted.
    pub deletes: Vec<ResourceChange>,
    /// Resources that are already up-to-date.
    pub unchanged: Vec<ResourceChange>,
    /// Raw JSON from `az`, preserved for debugging.
    pub raw_json: serde_json::Value,
}

/// A single resource change entry from the what-if output.
#[derive(Debug, Clone)]
pub struct ResourceChange {
    /// e.g. `"Microsoft.App/containerApps"`
    pub resource_type: String,
    /// Friendly name portion of the resource ID, e.g. `"quelch-prod-mcp"`.
    pub resource_id: String,
    /// Field-level diffs — only populated for [`WhatIfReport::modifies`].
    pub field_changes: Vec<FieldChange>,
}

/// A single field-level change within a modify.
#[derive(Debug, Clone)]
pub struct FieldChange {
    /// Dotted JSON path to the changed property.
    pub path: String,
    /// Before value.
    pub from: serde_json::Value,
    /// After value.
    pub to: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can arise from running or parsing what-if output.
#[derive(Debug, thiserror::Error)]
pub enum WhatIfError {
    #[error("az command failed (exit {code}): {stderr}")]
    AzFailed { code: i32, stderr: String },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Shell-out entry point
// ---------------------------------------------------------------------------

/// Run `az deployment group what-if` and return a structured report.
///
/// This shells out to the `az` CLI and parses its JSON output. It will fail
/// if `az` is not installed or the user is not logged in.
///
/// The `--no-pretty-print` flag instructs `az` to emit raw JSON instead of
/// coloured human-readable output, which we then parse.
pub fn run(resource_group: &str, bicep_path: &Path) -> Result<WhatIfReport, WhatIfError> {
    let output = std::process::Command::new("az")
        .args([
            "deployment",
            "group",
            "what-if",
            "--resource-group",
            resource_group,
            "--template-file",
            bicep_path.to_str().unwrap_or(""),
            "--no-pretty-print",
        ])
        .output()?;

    if !output.status.success() {
        return Err(WhatIfError::AzFailed {
            code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    parse_whatif_json(&json)
}

// ---------------------------------------------------------------------------
// Pure JSON parser — unit-testable without `az`
// ---------------------------------------------------------------------------

/// Parse the JSON value produced by `az deployment group what-if
/// --no-pretty-print` into a [`WhatIfReport`].
///
/// The top-level shape is:
/// ```json
/// { "changes": [ { "changeType": "Create", "resourceId": "...", "delta": [...] } ] }
/// ```
///
/// Known `changeType` values: `Create`, `Modify`, `Delete`, `NoChange`,
/// `Ignore`, `Deploy`, `Unsupported`.
pub fn parse_whatif_json(json: &serde_json::Value) -> Result<WhatIfReport, WhatIfError> {
    let changes = json
        .get("changes")
        .and_then(|v| v.as_array())
        .ok_or_else(|| WhatIfError::Parse("missing top-level 'changes' array".to_string()))?;

    let mut report = WhatIfReport {
        creates: Vec::new(),
        modifies: Vec::new(),
        deletes: Vec::new(),
        unchanged: Vec::new(),
        raw_json: json.clone(),
    };

    for entry in changes {
        let change_type = entry
            .get("changeType")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        let resource_id_full = entry
            .get("resourceId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Extract the short resource name from the full ARM resource ID.
        // ARM IDs look like: /subscriptions/{sub}/resourceGroups/{rg}/providers/{type}/{name}
        let resource_name = extract_resource_name(&resource_id_full);

        // Extract the resource type from the ARM ID.
        let resource_type = extract_resource_type(&resource_id_full);

        let field_changes = if change_type == "Modify" {
            parse_delta(entry)
        } else {
            Vec::new()
        };

        let change = ResourceChange {
            resource_type,
            resource_id: resource_name,
            field_changes,
        };

        match change_type {
            "Create" => report.creates.push(change),
            "Modify" => report.modifies.push(change),
            "Delete" => report.deletes.push(change),
            "NoChange" => report.unchanged.push(change),
            // Ignore, Deploy, Unsupported — treat as unchanged for display purposes.
            _ => report.unchanged.push(change),
        }
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Delta parser (for Modify entries)
// ---------------------------------------------------------------------------

fn parse_delta(entry: &serde_json::Value) -> Vec<FieldChange> {
    let Some(delta) = entry.get("delta").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    let mut changes = Vec::new();
    for item in delta {
        // Each delta item has a `path`, and optionally `before`/`after`.
        let path = item
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if path.is_empty() {
            continue;
        }

        let from = item
            .get("before")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let to = item
            .get("after")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        changes.push(FieldChange { path, from, to });
    }
    changes
}

// ---------------------------------------------------------------------------
// ARM ID helpers
// ---------------------------------------------------------------------------

/// Extract the leaf resource name from an ARM resource ID.
///
/// `/subscriptions/x/resourceGroups/y/providers/Microsoft.App/containerApps/my-app`
/// → `"my-app"`
fn extract_resource_name(id: &str) -> String {
    id.split('/').next_back().unwrap_or(id).to_string()
}

/// Extract the resource type from an ARM resource ID, e.g.
/// `"Microsoft.App/containerApps"`.
fn extract_resource_type(id: &str) -> String {
    // ARM IDs: .../providers/{namespace}/{type}/{name}
    // We look for the segment after "providers".
    let parts: Vec<&str> = id.split('/').collect();
    let providers_pos = parts
        .iter()
        .position(|&s| s.eq_ignore_ascii_case("providers"));
    if let Some(pos) = providers_pos {
        // namespace at pos+1, type at pos+2
        if pos + 2 < parts.len() {
            return format!("{}/{}", parts[pos + 1], parts[pos + 2]);
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn load_fixture(name: &str) -> serde_json::Value {
        let path = std::path::Path::new("tests/fixtures").join(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("cannot parse fixture {name}: {e}"))
    }

    #[test]
    fn parses_create_change() {
        let json = load_fixture("whatif_sample.json");
        let report = parse_whatif_json(&json).unwrap();
        assert_eq!(report.creates.len(), 1, "expected one create");
        assert_eq!(
            report.creates[0].resource_type,
            "Microsoft.App/containerApps"
        );
        assert_eq!(report.creates[0].resource_id, "quelch-prod-mcp");
        assert!(
            report.creates[0].field_changes.is_empty(),
            "creates should have no field changes"
        );
    }

    #[test]
    fn parses_modify_with_field_changes() {
        let json = load_fixture("whatif_sample.json");
        let report = parse_whatif_json(&json).unwrap();
        assert_eq!(report.modifies.len(), 1, "expected one modify");
        let m = &report.modifies[0];
        assert!(
            !m.field_changes.is_empty(),
            "modify should have field changes"
        );
        let fc = &m.field_changes[0];
        assert_eq!(fc.path, "properties.throughput.mode");
    }

    #[test]
    fn parses_delete_change() {
        let json = load_fixture("whatif_sample.json");
        let report = parse_whatif_json(&json).unwrap();
        assert_eq!(report.deletes.len(), 1, "expected one delete");
        assert_eq!(
            report.deletes[0].resource_type,
            "Microsoft.DocumentDB/databaseAccounts"
        );
    }

    #[test]
    fn parses_nochange_entry() {
        let json = load_fixture("whatif_sample.json");
        let report = parse_whatif_json(&json).unwrap();
        assert_eq!(report.unchanged.len(), 1, "expected one unchanged");
        assert_eq!(
            report.unchanged[0].resource_type,
            "Microsoft.Search/searchServices"
        );
    }

    #[test]
    fn propagates_parse_error_for_missing_changes_key() {
        let json = serde_json::json!({ "notChanges": [] });
        let err = parse_whatif_json(&json).unwrap_err();
        assert!(matches!(err, WhatIfError::Parse(_)));
    }

    #[test]
    fn extract_resource_name_from_arm_id() {
        let id =
            "/subscriptions/sub/resourceGroups/rg/providers/Microsoft.App/containerApps/my-app";
        assert_eq!(extract_resource_name(id), "my-app");
    }

    #[test]
    fn extract_resource_type_from_arm_id() {
        let id =
            "/subscriptions/sub/resourceGroups/rg/providers/Microsoft.App/containerApps/my-app";
        assert_eq!(extract_resource_type(id), "Microsoft.App/containerApps");
    }
}
