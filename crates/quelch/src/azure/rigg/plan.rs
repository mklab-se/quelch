/// Compute a plan by diffing local rigg files against live Azure AI Search resources.
///
/// # Overview
///
/// `run` reads every YAML file under `rigg_dir/{subdir}/*.yaml`, fetches the
/// equivalent resources from Azure via the [`RiggApiAdapter`] trait, and
/// produces a [`PlanReport`] classifying each resource as **create**,
/// **update**, **delete**, or **unchanged**.
///
/// # Field-level diff
///
/// For update entries, [`ResourceDiff`] contains [`FieldChange`] values with
/// dotted JSON-pointer paths (e.g. `fields.0.searchable`) so callers can
/// render a human-readable diff.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;

use rigg_core::resources::ResourceKind;

/// Errors that can occur during plan computation.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("rigg: {0}")]
    Rigg(String),
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

/// A reference to a single Azure AI Search resource.
#[derive(Debug, Clone)]
pub struct ResourceRef {
    /// The type of this resource.
    pub kind: ResourceKind,
    /// The resource name (used as the Azure resource identifier).
    pub name: String,
}

/// A field-level change within a resource that differs between local and live.
#[derive(Debug, Clone)]
pub struct FieldChange {
    /// Dotted path to the changed field (e.g. `fields.0.searchable`).
    pub path: String,
    /// Value in the live (Azure) state.
    pub from: serde_json::Value,
    /// Value in the local (disk) state.
    pub to: serde_json::Value,
}

/// All field-level changes for a single resource that needs updating.
#[derive(Debug)]
pub struct ResourceDiff {
    /// Individual field paths that changed, with before/after values.
    pub field_changes: Vec<FieldChange>,
}

/// The output of a plan run: resources to create, update, delete, or leave alone.
#[derive(Debug, Default)]
pub struct PlanReport {
    /// Resources present locally but not in Azure → will be created.
    pub creates: Vec<ResourceRef>,
    /// Resources present in both but with differing content → will be updated.
    pub updates: Vec<(ResourceRef, ResourceDiff)>,
    /// Resources present in Azure but not locally → will be deleted.
    pub deletes: Vec<ResourceRef>,
    /// Resources identical in both places → no action needed.
    pub unchanged: Vec<ResourceRef>,
}

/// Adapter trait abstracting rigg-client operations for testability.
///
/// Production code wires [`RiggClientAdapter`]; tests inject [`MockRiggApi`].
#[trait_variant::make(Send)]
pub trait RiggApiAdapter: Sync {
    /// List all resources of the given kind, returning raw JSON objects.
    async fn list_resources(
        &self,
        kind: ResourceKind,
    ) -> Result<Vec<serde_json::Value>, anyhow::Error>;

    /// Create or replace a resource.
    async fn upsert_resource(
        &self,
        kind: ResourceKind,
        name: &str,
        body: &serde_json::Value,
    ) -> Result<(), anyhow::Error>;

    /// Delete a resource by name.
    async fn delete_resource(&self, kind: ResourceKind, name: &str) -> Result<(), anyhow::Error>;
}

/// Production adapter that wraps `rigg_client::AzureSearchClient`.
pub struct RiggClientAdapter {
    client: rigg_client::AzureSearchClient,
}

impl RiggClientAdapter {
    /// Create an adapter connected to the given endpoint with the default
    /// Azure CLI / environment-variable auth provider.
    pub fn new(base_url: String, preview_api_version: String) -> Result<Self, anyhow::Error> {
        let auth =
            rigg_client::auth::get_auth_provider().map_err(|e| anyhow::anyhow!("auth: {e}"))?;
        let client = rigg_client::AzureSearchClient::with_auth(base_url, preview_api_version, auth)
            .map_err(|e| anyhow::anyhow!("client: {e}"))?;
        Ok(Self { client })
    }
}

impl RiggApiAdapter for RiggClientAdapter {
    async fn list_resources(
        &self,
        kind: ResourceKind,
    ) -> Result<Vec<serde_json::Value>, anyhow::Error> {
        self.client
            .list(kind)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    async fn upsert_resource(
        &self,
        kind: ResourceKind,
        name: &str,
        body: &serde_json::Value,
    ) -> Result<(), anyhow::Error> {
        self.client
            .create_or_update(kind, name, body)
            .await
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    async fn delete_resource(&self, kind: ResourceKind, name: &str) -> Result<(), anyhow::Error> {
        self.client
            .delete(kind, name)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

// ---------------------------------------------------------------------------
// Resource kind → local subdirectory mapping
// ---------------------------------------------------------------------------

/// Maps a [`ResourceKind`] to the flat subdirectory name quelch uses on disk.
///
/// This matches the layout that [`crate::azure::rigg::write`] writes to, which
/// differs from rigg-core's categorised `directory_name()` structure.
/// Exposed for use by the `push` and `pull` modules.
pub fn subdir_for_kind(kind: ResourceKind) -> &'static str {
    subdir_for(kind)
}

fn subdir_for(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Index => "indexes",
        ResourceKind::DataSource => "datasources",
        ResourceKind::Skillset => "skillsets",
        ResourceKind::Indexer => "indexers",
        ResourceKind::KnowledgeSource => "knowledge_sources",
        ResourceKind::KnowledgeBase => "knowledge_bases",
        // Not generated by quelch, but handled gracefully.
        ResourceKind::SynonymMap => "synonym_maps",
        ResourceKind::Alias => "aliases",
        ResourceKind::Agent => "agents",
    }
}

/// The resource kinds quelch manages (matches `write.rs` groups).
///
/// Exposed so `push` and `pull` can iterate over the same set.
pub const MANAGED_KINDS: &[ResourceKind] = &[
    ResourceKind::DataSource,
    ResourceKind::Skillset,
    ResourceKind::Index,
    ResourceKind::Indexer,
    ResourceKind::KnowledgeSource,
    ResourceKind::KnowledgeBase,
];

// ---------------------------------------------------------------------------
// Plan entry point
// ---------------------------------------------------------------------------

/// Run a plan: diff local rigg files against live Azure resources.
///
/// `rigg_dir` must be the root of the quelch rigg output directory (the
/// directory containing `indexes/`, `datasources/`, etc. subdirs).
///
/// `api` is the rigg adapter — use [`RiggClientAdapter`] in production or a
/// mock in tests.
pub async fn run<A: RiggApiAdapter>(rigg_dir: &Path, api: &A) -> Result<PlanReport, PlanError> {
    let local = read_local(rigg_dir)?;
    let live = fetch_live(api).await?;
    Ok(compute_diff(local, live))
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

type ResourceMap = HashMap<(ResourceKind, String), serde_json::Value>;

/// Read all local YAML files under `rigg_dir` into a map keyed by (kind, name).
fn read_local(rigg_dir: &Path) -> Result<ResourceMap, PlanError> {
    let mut map = ResourceMap::new();

    for kind in MANAGED_KINDS {
        let subdir = rigg_dir.join(subdir_for(*kind));
        if !subdir.exists() {
            continue;
        }

        for entry in std::fs::read_dir(&subdir)? {
            let entry = entry?;
            let path: PathBuf = entry.path();

            if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }

            let yaml_text = std::fs::read_to_string(&path)?;
            // Parse YAML → JSON value so we can do uniform comparison.
            let yaml_val: serde_yaml::Value = serde_yaml::from_str(&yaml_text)?;
            let json_val = yaml_to_json(yaml_val);

            // Extract the resource name.
            let name = extract_name(&json_val, &path);
            map.insert((*kind, name), json_val);
        }
    }

    Ok(map)
}

/// Fetch all live resources from Azure into the same map shape.
async fn fetch_live<A: RiggApiAdapter>(api: &A) -> Result<ResourceMap, PlanError> {
    let mut map = ResourceMap::new();

    for kind in MANAGED_KINDS {
        let items = api
            .list_resources(*kind)
            .await
            .map_err(|e| PlanError::Rigg(e.to_string()))?;

        for item in items {
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if !name.is_empty() {
                map.insert((*kind, name), item);
            }
        }
    }

    Ok(map)
}

/// Compute the plan diff between local and live states.
fn compute_diff(local: ResourceMap, live: ResourceMap) -> PlanReport {
    let mut report = PlanReport::default();

    // Everything in local.
    for ((kind, name), local_val) in &local {
        let rref = ResourceRef {
            kind: *kind,
            name: name.clone(),
        };

        match live.get(&(*kind, name.clone())) {
            None => {
                // Local only → create.
                report.creates.push(rref);
            }
            Some(live_val) => {
                // Both → diff.
                let changes = diff_values(local_val, live_val, "");
                if changes.is_empty() {
                    report.unchanged.push(rref);
                } else {
                    report.updates.push((
                        rref,
                        ResourceDiff {
                            field_changes: changes,
                        },
                    ));
                }
            }
        }
    }

    // Everything in live but not in local → delete.
    for (kind, name) in live.keys() {
        if !local.contains_key(&(*kind, name.clone())) {
            report.deletes.push(ResourceRef {
                kind: *kind,
                name: name.clone(),
            });
        }
    }

    report
}

/// Recursively diff two JSON values, producing [`FieldChange`] entries for
/// every leaf that differs.
fn diff_values(local: &JsonValue, live: &JsonValue, path: &str) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    diff_values_inner(local, live, path, &mut changes);
    changes
}

fn diff_values_inner(
    local: &JsonValue,
    live: &JsonValue,
    path: &str,
    changes: &mut Vec<FieldChange>,
) {
    match (local, live) {
        (JsonValue::Object(loc_map), JsonValue::Object(live_map)) => {
            // Keys in local.
            for (k, loc_v) in loc_map {
                let child_path = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                match live_map.get(k) {
                    None => {
                        // Field absent in live — treat as a change.
                        changes.push(FieldChange {
                            path: child_path,
                            from: JsonValue::Null,
                            to: loc_v.clone(),
                        });
                    }
                    Some(live_v) => {
                        diff_values_inner(loc_v, live_v, &child_path, changes);
                    }
                }
            }
            // Keys in live but not in local.
            for (k, live_v) in live_map {
                if !loc_map.contains_key(k) {
                    let child_path = if path.is_empty() {
                        k.clone()
                    } else {
                        format!("{path}.{k}")
                    };
                    changes.push(FieldChange {
                        path: child_path,
                        from: live_v.clone(),
                        to: JsonValue::Null,
                    });
                }
            }
        }
        (JsonValue::Array(loc_arr), JsonValue::Array(live_arr)) => {
            let max_len = loc_arr.len().max(live_arr.len());
            for i in 0..max_len {
                let child_path = if path.is_empty() {
                    i.to_string()
                } else {
                    format!("{path}.{i}")
                };
                match (loc_arr.get(i), live_arr.get(i)) {
                    (Some(l), Some(r)) => diff_values_inner(l, r, &child_path, changes),
                    (Some(l), None) => changes.push(FieldChange {
                        path: child_path,
                        from: JsonValue::Null,
                        to: l.clone(),
                    }),
                    (None, Some(r)) => changes.push(FieldChange {
                        path: child_path,
                        from: r.clone(),
                        to: JsonValue::Null,
                    }),
                    (None, None) => {}
                }
            }
        }
        // Leaf comparison.
        _ => {
            if local != live {
                changes.push(FieldChange {
                    path: path.to_string(),
                    from: live.clone(),
                    to: local.clone(),
                });
            }
        }
    }
}

/// Extract the resource `name` from a JSON object, falling back to the
/// file stem if the `name` key is absent.
fn extract_name(val: &JsonValue, path: &Path) -> String {
    if let Some(s) = val.get("name").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    // Fall back to file stem.
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Convert `serde_yaml::Value` to `serde_json::Value` for uniform comparison.
fn yaml_to_json(v: serde_yaml::Value) -> serde_json::Value {
    // Round-trip through JSON serialisation — straightforward and correct
    // for the YAML subset that Azure resource files use.
    let json_str = serde_json::to_string(&v).unwrap_or_default();
    serde_json::from_str(&json_str).unwrap_or(serde_json::Value::Null)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    // -----------------------------------------------------------------------
    // Mock adapter
    // -----------------------------------------------------------------------

    /// A simple mock rigg API for unit tests.
    #[derive(Default)]
    pub struct MockRiggApi {
        /// Pre-loaded live resources per kind.
        live: HashMap<ResourceKind, Vec<serde_json::Value>>,
        /// Recorded upsert calls: (kind, name).
        pub upserted: Arc<Mutex<Vec<(ResourceKind, String)>>>,
        /// Recorded delete calls: (kind, name).
        pub deleted: Arc<Mutex<Vec<(ResourceKind, String)>>>,
    }

    impl MockRiggApi {
        pub fn with_live(mut self, kind: ResourceKind, items: Vec<serde_json::Value>) -> Self {
            self.live.insert(kind, items);
            self
        }
    }

    impl RiggApiAdapter for MockRiggApi {
        async fn list_resources(
            &self,
            kind: ResourceKind,
        ) -> Result<Vec<serde_json::Value>, anyhow::Error> {
            Ok(self.live.get(&kind).cloned().unwrap_or_default())
        }

        async fn upsert_resource(
            &self,
            kind: ResourceKind,
            name: &str,
            _body: &serde_json::Value,
        ) -> Result<(), anyhow::Error> {
            self.upserted.lock().unwrap().push((kind, name.to_string()));
            Ok(())
        }

        async fn delete_resource(
            &self,
            kind: ResourceKind,
            name: &str,
        ) -> Result<(), anyhow::Error> {
            self.deleted.lock().unwrap().push((kind, name.to_string()));
            Ok(())
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn write_index(dir: &Path, name: &str, content: &str) {
        let sub = dir.join("indexes");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(format!("{name}.yaml")), content).unwrap();
    }

    fn minimal_index_json(name: &str) -> serde_json::Value {
        serde_json::json!({ "name": name, "fields": [] })
    }

    fn minimal_index_yaml(name: &str) -> String {
        format!("name: {name}\nfields: []\n")
    }

    // -----------------------------------------------------------------------
    // plan tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn plan_creates_when_local_only() {
        let dir = tempfile::tempdir().unwrap();
        write_index(
            dir.path(),
            "jira-issues",
            &minimal_index_yaml("jira-issues"),
        );

        let api = MockRiggApi::default(); // live is empty
        let report = run(dir.path(), &api).await.unwrap();

        assert_eq!(report.creates.len(), 1, "expected one create");
        assert_eq!(report.creates[0].name, "jira-issues");
        assert!(matches!(report.creates[0].kind, ResourceKind::Index));
        assert!(report.updates.is_empty());
        assert!(report.deletes.is_empty());
        assert!(report.unchanged.is_empty());
    }

    #[tokio::test]
    async fn plan_deletes_when_live_only() {
        let dir = tempfile::tempdir().unwrap();
        // No local files.

        let api = MockRiggApi::default()
            .with_live(ResourceKind::Index, vec![minimal_index_json("jira-issues")]);
        let report = run(dir.path(), &api).await.unwrap();

        assert_eq!(report.deletes.len(), 1, "expected one delete");
        assert_eq!(report.deletes[0].name, "jira-issues");
        assert!(report.creates.is_empty());
        assert!(report.updates.is_empty());
        assert!(report.unchanged.is_empty());
    }

    #[tokio::test]
    async fn plan_unchanged_when_identical() {
        let dir = tempfile::tempdir().unwrap();
        write_index(
            dir.path(),
            "jira-issues",
            &minimal_index_yaml("jira-issues"),
        );

        let api = MockRiggApi::default()
            .with_live(ResourceKind::Index, vec![minimal_index_json("jira-issues")]);
        let report = run(dir.path(), &api).await.unwrap();

        assert_eq!(report.unchanged.len(), 1, "expected one unchanged");
        assert!(report.creates.is_empty());
        assert!(report.updates.is_empty());
        assert!(report.deletes.is_empty());
    }

    #[tokio::test]
    async fn plan_updates_when_field_changes() {
        let dir = tempfile::tempdir().unwrap();
        // Local has searchable: true on a field; live has false.
        let local_yaml = "name: jira-issues\nfields:\n  - name: title\n    searchable: true\n";
        write_index(dir.path(), "jira-issues", local_yaml);

        let live_json = serde_json::json!({
            "name": "jira-issues",
            "fields": [{ "name": "title", "searchable": false }]
        });
        let api = MockRiggApi::default().with_live(ResourceKind::Index, vec![live_json]);
        let report = run(dir.path(), &api).await.unwrap();

        assert_eq!(report.updates.len(), 1, "expected one update");
        let (rref, diff) = &report.updates[0];
        assert_eq!(rref.name, "jira-issues");
        // At least one field change mentioning "searchable".
        let mentions_searchable = diff
            .field_changes
            .iter()
            .any(|fc| fc.path.contains("searchable"));
        assert!(
            mentions_searchable,
            "expected a FieldChange for 'searchable', got: {:?}",
            diff.field_changes
                .iter()
                .map(|f| &f.path)
                .collect::<Vec<_>>()
        );
        assert!(report.creates.is_empty());
        assert!(report.deletes.is_empty());
        assert!(report.unchanged.is_empty());
    }

    #[test]
    fn diff_values_leaf_change() {
        let a = serde_json::json!({"x": 1});
        let b = serde_json::json!({"x": 2});
        let changes = diff_values(&a, &b, "");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "x");
        assert_eq!(changes[0].from, serde_json::json!(2));
        assert_eq!(changes[0].to, serde_json::json!(1));
    }

    #[test]
    fn diff_values_nested() {
        let a = serde_json::json!({"a": {"b": true}});
        let b = serde_json::json!({"a": {"b": false}});
        let changes = diff_values(&a, &b, "");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "a.b");
    }

    #[test]
    fn diff_values_identical_empty() {
        let a = serde_json::json!({"name": "x"});
        let changes = diff_values(&a, &a, "");
        assert!(changes.is_empty());
    }

    #[test]
    fn subdir_for_round_trips_all_managed_kinds() {
        for kind in MANAGED_KINDS {
            // subdir_for should not panic for any managed kind.
            let s = subdir_for(*kind);
            assert!(!s.is_empty());
        }
    }
}
