//! Diff a [`RiggDesiredState`] against the live Azure AI Search service.
//!
//! Phase 4 of the no-deploy pivot: this module operates purely on in-memory
//! state. There are no on-disk YAML files, no `rigg/` directory.
//!
//! [`plan`] takes the desired state plus a [`RiggApiAdapter`] (which fans
//! out to the live service via `rigg-client`) and returns a [`RiggDiff`]
//! classifying each resource as create / update / match. Render the diff
//! with [`RiggDiff::render`] for human-readable output.
//!
//! Quelch never schedules deletes — by spec, Quelch only adds. Users who
//! want a resource gone use the AI Search portal or the standalone `rigg`
//! tool.

use std::collections::HashMap;

use serde_json::Value as JsonValue;

use rigg_core::resources::ResourceKind;

use super::RiggDesiredState;

/// Errors that can occur during plan computation.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    /// The rigg API call to fetch live state failed.
    #[error("rigg api: {0}")]
    Api(String),
    /// Failed to serialise a desired-state resource to JSON.
    #[error("serialise: {0}")]
    Serialise(#[from] serde_json::Error),
}

/// A reference to a single Azure AI Search resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRef {
    /// The type of this resource.
    pub kind: ResourceKind,
    /// The resource name (used as the Azure resource identifier).
    pub name: String,
}

/// A field-level change within a resource that differs between desired and live.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldChange {
    /// Dotted path to the changed field (e.g. `fields.0.searchable`).
    pub path: String,
    /// Value in the live (Azure) state.
    pub from: JsonValue,
    /// Value in the desired (config-derived) state.
    pub to: JsonValue,
}

/// A single resource-level change in the plan.
#[derive(Debug, Clone)]
pub enum ResourceChange {
    /// Resource is in desired state but absent from the live service.
    Create(ResourceRef),
    /// Resource is in both, but their normalised JSON differs.
    Update {
        /// Which resource will be updated.
        rref: ResourceRef,
        /// Field-level differences for human-readable diffs.
        changes: Vec<FieldChange>,
    },
    /// Resource is in both with identical content; no action needed.
    Match(ResourceRef),
}

impl ResourceChange {
    /// The resource this change refers to.
    pub fn resource(&self) -> &ResourceRef {
        match self {
            ResourceChange::Create(r)
            | ResourceChange::Update { rref: r, .. }
            | ResourceChange::Match(r) => r,
        }
    }
}

/// The full plan: per-resource changes for every kind quelch manages.
#[derive(Debug, Default)]
pub struct RiggDiff {
    /// All resource-level changes, in deterministic order (kind-major,
    /// then alphabetical by name).
    pub changes: Vec<ResourceChange>,
}

impl RiggDiff {
    /// Iterate only the changes that would actually mutate the live service
    /// (i.e. excludes [`ResourceChange::Match`]).
    pub fn pending(&self) -> impl Iterator<Item = &ResourceChange> {
        self.changes
            .iter()
            .filter(|c| !matches!(c, ResourceChange::Match(_)))
    }

    /// True if the live state already matches the desired state.
    pub fn is_clean(&self) -> bool {
        self.pending().next().is_none()
    }

    /// Render the diff as a human-readable multi-line string.
    ///
    /// Format:
    /// ```text
    ///   + indexes/jira-issues  (create)
    ///   ~ indexers/jira-issues (update)
    ///       fields.0.searchable: false → true
    ///   = data_sources/jira-issues (no change)
    /// ```
    pub fn render(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        if self.changes.is_empty() {
            writeln!(&mut out, "  (no managed resources)").ok();
            return out;
        }
        for change in &self.changes {
            match change {
                ResourceChange::Create(r) => {
                    writeln!(&mut out, "  + {}/{}  (create)", kind_label(r.kind), r.name).ok();
                }
                ResourceChange::Update { rref, changes } => {
                    writeln!(
                        &mut out,
                        "  ~ {}/{}  (update)",
                        kind_label(rref.kind),
                        rref.name
                    )
                    .ok();
                    for fc in changes {
                        writeln!(
                            &mut out,
                            "      {}: {} → {}",
                            fc.path,
                            short_value(&fc.from),
                            short_value(&fc.to)
                        )
                        .ok();
                    }
                }
                ResourceChange::Match(r) => {
                    writeln!(
                        &mut out,
                        "  = {}/{}  (no change)",
                        kind_label(r.kind),
                        r.name
                    )
                    .ok();
                }
            }
        }
        out
    }
}

fn kind_label(k: ResourceKind) -> &'static str {
    match k {
        ResourceKind::Index => "indexes",
        ResourceKind::DataSource => "data_sources",
        ResourceKind::Skillset => "skillsets",
        ResourceKind::Indexer => "indexers",
        ResourceKind::KnowledgeSource => "knowledge_sources",
        ResourceKind::KnowledgeBase => "knowledge_bases",
        ResourceKind::SynonymMap => "synonym_maps",
        ResourceKind::Alias => "aliases",
        ResourceKind::Agent => "agents",
    }
}

fn short_value(v: &JsonValue) -> String {
    match v {
        JsonValue::Null => "null".to_string(),
        JsonValue::Bool(b) => b.to_string(),
        JsonValue::Number(n) => n.to_string(),
        JsonValue::String(s) => format!("\"{s}\""),
        JsonValue::Array(_) | JsonValue::Object(_) => {
            let s = serde_json::to_string(v).unwrap_or_else(|_| "<?>".to_string());
            if s.len() > 60 {
                format!("{}...", &s[..57])
            } else {
                s
            }
        }
    }
}

/// Resource kinds quelch manages, in dependency order (data sources first,
/// knowledge bases last).
pub const MANAGED_KINDS: &[ResourceKind] = &[
    ResourceKind::DataSource,
    ResourceKind::Skillset,
    ResourceKind::Index,
    ResourceKind::Indexer,
    ResourceKind::KnowledgeSource,
    ResourceKind::KnowledgeBase,
];

/// Adapter trait abstracting rigg-client operations for testability.
///
/// Production code wires [`RiggClientAdapter`]; tests inject a mock.
#[trait_variant::make(Send)]
pub trait RiggApiAdapter: Sync {
    /// List all resources of the given kind, returning raw JSON objects.
    async fn list_resources(&self, kind: ResourceKind) -> Result<Vec<JsonValue>, anyhow::Error>;

    /// Create or replace a resource. The body is the rigg-core resource
    /// serialised to JSON.
    async fn upsert_resource(
        &self,
        kind: ResourceKind,
        name: &str,
        body: &JsonValue,
    ) -> Result<(), anyhow::Error>;
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
    async fn list_resources(&self, kind: ResourceKind) -> Result<Vec<JsonValue>, anyhow::Error> {
        self.client.list(kind).await.map_err(format_rigg_err)
    }

    async fn upsert_resource(
        &self,
        kind: ResourceKind,
        name: &str,
        body: &JsonValue,
    ) -> Result<(), anyhow::Error> {
        self.client
            .create_or_update(kind, name, body)
            .await
            .map(|_| ())
            .map_err(format_rigg_err)
    }
}

/// Wrap a `rigg-client` error into an `anyhow::Error` with the client's
/// suggested remediation appended, plus a quelch-tailored line for the most
/// common failure (403 Forbidden — RBAC misconfigured on the AI Search
/// service).
///
/// Without this, the typed `ClientError` collapses to its `Display` text
/// when crossing the `anyhow::Error` boundary and the user only sees
/// `Access denied (403 Forbidden): foo.search.windows.net` — true, but
/// not enough to act on.
fn format_rigg_err(e: rigg_client::ClientError) -> anyhow::Error {
    let mut msg = format!("{e}\n\nSuggested fix:\n{}", e.suggestion());
    if let rigg_client::ClientError::Forbidden { service, .. } = &e
        && let Some(name) = service.split('.').next()
        && !name.is_empty()
    {
        msg.push_str(&format!(
            "\n\nFor your service '{name}', the concrete commands are:\n  \
             az search service update \\\n    \
             --name {name} --resource-group <RG> \\\n    \
             --auth-options aadOrApiKey\n  \
             az role assignment create \\\n    \
             --assignee \"$(az ad signed-in-user show --query id -o tsv)\" \\\n    \
             --role \"Search Service Contributor\" \\\n    \
             --scope $(az search service show --name {name} --resource-group <RG> --query id -o tsv)\n  \
             az role assignment create \\\n    \
             --assignee \"$(az ad signed-in-user show --query id -o tsv)\" \\\n    \
             --role \"Search Index Data Contributor\" \\\n    \
             --scope $(az search service show --name {name} --resource-group <RG> --query id -o tsv)\n\
             (substitute <RG> for the resource group hosting the search service)"
        ));
    }
    anyhow::anyhow!("{msg}")
}

// ---------------------------------------------------------------------------
// plan entry point
// ---------------------------------------------------------------------------

/// Compute a [`RiggDiff`] by diffing `desired` against the live state of
/// the AI Search service reachable through `api`.
pub async fn plan<A: RiggApiAdapter>(
    desired: &RiggDesiredState,
    api: &A,
) -> Result<RiggDiff, PlanError> {
    let live = fetch_live(api).await?;
    let desired_map = serialise_desired(desired)?;

    let mut diff = RiggDiff::default();

    for kind in MANAGED_KINDS {
        // Sort by name for deterministic output.
        let mut entries: Vec<(&String, &JsonValue)> = desired_map
            .iter()
            .filter(|((k, _), _)| k == kind)
            .map(|((_, n), v)| (n, v))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));

        for (name, want) in entries {
            let rref = ResourceRef {
                kind: *kind,
                name: name.clone(),
            };
            match live.get(&(*kind, name.clone())) {
                None => diff.changes.push(ResourceChange::Create(rref)),
                Some(have) => {
                    let changes = diff_values(want, have, "");
                    if changes.is_empty() {
                        diff.changes.push(ResourceChange::Match(rref));
                    } else {
                        diff.changes.push(ResourceChange::Update { rref, changes });
                    }
                }
            }
        }
    }

    Ok(diff)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

type ResourceMap = HashMap<(ResourceKind, String), JsonValue>;

/// Serialise every resource in [`RiggDesiredState`] into the (kind, name) map.
fn serialise_desired(state: &RiggDesiredState) -> Result<ResourceMap, PlanError> {
    let mut map = ResourceMap::new();
    for r in &state.data_sources {
        map.insert(
            (ResourceKind::DataSource, r.name.clone()),
            serde_json::to_value(r)?,
        );
    }
    for r in &state.skillsets {
        map.insert(
            (ResourceKind::Skillset, r.name.clone()),
            serde_json::to_value(r)?,
        );
    }
    for r in &state.indexes {
        map.insert(
            (ResourceKind::Index, r.name.clone()),
            serde_json::to_value(r)?,
        );
    }
    for r in &state.indexers {
        map.insert(
            (ResourceKind::Indexer, r.name.clone()),
            serde_json::to_value(r)?,
        );
    }
    for r in &state.knowledge_sources {
        map.insert(
            (ResourceKind::KnowledgeSource, r.name.clone()),
            serde_json::to_value(r)?,
        );
    }
    for r in &state.knowledge_bases {
        map.insert(
            (ResourceKind::KnowledgeBase, r.name.clone()),
            serde_json::to_value(r)?,
        );
    }
    Ok(map)
}

/// Fetch all live resources from Azure into the same map shape.
///
/// Live AI Search responses include `@odata.context`, `@odata.etag`, and
/// other server-managed metadata that are absent from [`RiggDesiredState`].
/// We strip those before diffing — without this pass, every `quelch azure
/// plan` against a real service shows false-positive `Update` entries on
/// otherwise-quiescent resources.
async fn fetch_live<A: RiggApiAdapter>(api: &A) -> Result<ResourceMap, PlanError> {
    let mut map = ResourceMap::new();
    for kind in MANAGED_KINDS {
        let items = api
            .list_resources(*kind)
            .await
            .map_err(|e| PlanError::Api(e.to_string()))?;
        for mut item in items {
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if !name.is_empty() {
                strip_server_managed_fields(&mut item);
                map.insert((*kind, name), item);
            }
        }
    }
    Ok(map)
}

/// Recursively remove server-managed fields from a JSON value before diffing.
///
/// AI Search responses include several keys that the desired state never
/// produces — they exist purely as response metadata. Comparing them
/// produces noise, never signal.
///
/// Stripped:
/// - any key starting with `@odata.` (e.g. `@odata.context`, `@odata.etag`,
///   `@odata.type`)
/// - any key starting with `@search.` (e.g. `@search.action`,
///   `@search.score`)
/// - common timestamp / etag fields the service stamps onto reads:
///   `etag`, `lastModified`, `createdAt`, `modifiedAt`.
fn strip_server_managed_fields(v: &mut JsonValue) {
    match v {
        JsonValue::Object(map) => {
            map.retain(|k, _| !is_server_managed_key(k));
            for child in map.values_mut() {
                strip_server_managed_fields(child);
            }
        }
        JsonValue::Array(arr) => {
            for child in arr {
                strip_server_managed_fields(child);
            }
        }
        _ => {}
    }
}

fn is_server_managed_key(k: &str) -> bool {
    if k.starts_with("@odata.") || k.starts_with("@search.") {
        return true;
    }
    matches!(k, "etag" | "lastModified" | "createdAt" | "modifiedAt")
}

/// Recursively diff two JSON values, producing [`FieldChange`] entries for
/// every leaf that differs.
fn diff_values(want: &JsonValue, have: &JsonValue, path: &str) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    diff_values_inner(want, have, path, &mut changes);
    changes
}

fn diff_values_inner(
    want: &JsonValue,
    have: &JsonValue,
    path: &str,
    changes: &mut Vec<FieldChange>,
) {
    match (want, have) {
        (JsonValue::Object(w_map), JsonValue::Object(h_map)) => {
            for (k, w_v) in w_map {
                let child = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                match h_map.get(k) {
                    None => changes.push(FieldChange {
                        path: child,
                        from: JsonValue::Null,
                        to: w_v.clone(),
                    }),
                    Some(h_v) => diff_values_inner(w_v, h_v, &child, changes),
                }
            }
            for (k, h_v) in h_map {
                if !w_map.contains_key(k) {
                    let child = if path.is_empty() {
                        k.clone()
                    } else {
                        format!("{path}.{k}")
                    };
                    changes.push(FieldChange {
                        path: child,
                        from: h_v.clone(),
                        to: JsonValue::Null,
                    });
                }
            }
        }
        (JsonValue::Array(w_arr), JsonValue::Array(h_arr)) => {
            let max_len = w_arr.len().max(h_arr.len());
            for i in 0..max_len {
                let child = if path.is_empty() {
                    i.to_string()
                } else {
                    format!("{path}.{i}")
                };
                match (w_arr.get(i), h_arr.get(i)) {
                    (Some(w), Some(h)) => diff_values_inner(w, h, &child, changes),
                    (Some(w), None) => changes.push(FieldChange {
                        path: child,
                        from: JsonValue::Null,
                        to: w.clone(),
                    }),
                    (None, Some(h)) => changes.push(FieldChange {
                        path: child,
                        from: h.clone(),
                        to: JsonValue::Null,
                    }),
                    (None, None) => {}
                }
            }
        }
        _ => {
            if want != have {
                changes.push(FieldChange {
                    path: path.to_string(),
                    from: have.clone(),
                    to: want.clone(),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// In-memory mock of [`RiggApiAdapter`] for unit tests.
    #[derive(Default)]
    pub struct MockRiggApi {
        live: HashMap<ResourceKind, Vec<JsonValue>>,
        /// Recorded upsert calls: (kind, name, body).
        pub upserted: Arc<Mutex<Vec<(ResourceKind, String, JsonValue)>>>,
    }

    impl MockRiggApi {
        /// Pre-load live resources of `kind` for the next `list_resources` call.
        pub fn with_live(mut self, kind: ResourceKind, items: Vec<JsonValue>) -> Self {
            self.live.insert(kind, items);
            self
        }
    }

    impl RiggApiAdapter for MockRiggApi {
        async fn list_resources(
            &self,
            kind: ResourceKind,
        ) -> Result<Vec<JsonValue>, anyhow::Error> {
            Ok(self.live.get(&kind).cloned().unwrap_or_default())
        }

        async fn upsert_resource(
            &self,
            kind: ResourceKind,
            name: &str,
            body: &JsonValue,
        ) -> Result<(), anyhow::Error> {
            self.upserted
                .lock()
                .unwrap()
                .push((kind, name.to_string(), body.clone()));
            Ok(())
        }
    }

    fn desired_with_one_index(name: &str) -> RiggDesiredState {
        let mut state = RiggDesiredState::default();
        state.indexes.push(rigg_core::resources::Index {
            name: name.to_string(),
            fields: vec![],
            scoring_profiles: None,
            default_scoring_profile: None,
            cors_options: None,
            suggesters: None,
            analyzers: None,
            tokenizers: None,
            token_filters: None,
            char_filters: None,
            similarity: None,
            semantic: None,
            vector_search: None,
            extra: Default::default(),
        });
        state
    }

    #[tokio::test]
    async fn plan_creates_when_live_is_empty() {
        let state = desired_with_one_index("jira-issues");
        let api = MockRiggApi::default();
        let diff = plan(&state, &api).await.unwrap();
        assert_eq!(diff.changes.len(), 1);
        assert!(matches!(diff.changes[0], ResourceChange::Create(_)));
        assert!(!diff.is_clean());
    }

    #[tokio::test]
    async fn plan_matches_when_live_has_identical_resource() {
        let state = desired_with_one_index("jira-issues");
        let live = serde_json::to_value(&state.indexes[0]).unwrap();
        let api = MockRiggApi::default().with_live(ResourceKind::Index, vec![live]);
        let diff = plan(&state, &api).await.unwrap();
        assert_eq!(diff.changes.len(), 1);
        assert!(matches!(diff.changes[0], ResourceChange::Match(_)));
        assert!(diff.is_clean());
    }

    #[tokio::test]
    async fn plan_updates_when_live_has_diverging_resource() {
        let state = desired_with_one_index("jira-issues");
        let live = serde_json::json!({
            "name": "jira-issues",
            "fields": [],
            "extraServerField": "drift",
        });
        let api = MockRiggApi::default().with_live(ResourceKind::Index, vec![live]);
        let diff = plan(&state, &api).await.unwrap();
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            ResourceChange::Update { changes, .. } => {
                assert!(
                    changes.iter().any(|c| c.path == "extraServerField"),
                    "expected change on extraServerField, got: {changes:?}"
                );
            }
            other => panic!("expected Update, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn plan_does_not_emit_deletes_for_extra_live_resources() {
        // Quelch only adds; live extras are user-managed and ignored.
        let state = RiggDesiredState::default();
        let live = serde_json::json!({"name": "user-managed", "fields": []});
        let api = MockRiggApi::default().with_live(ResourceKind::Index, vec![live]);
        let diff = plan(&state, &api).await.unwrap();
        assert!(diff.changes.is_empty());
        assert!(diff.is_clean());
    }

    #[test]
    fn render_handles_create_update_match_and_empty() {
        let mut diff = RiggDiff::default();
        assert!(diff.render().contains("(no managed resources)"));

        diff.changes.push(ResourceChange::Create(ResourceRef {
            kind: ResourceKind::Index,
            name: "a".into(),
        }));
        diff.changes.push(ResourceChange::Match(ResourceRef {
            kind: ResourceKind::DataSource,
            name: "b".into(),
        }));
        diff.changes.push(ResourceChange::Update {
            rref: ResourceRef {
                kind: ResourceKind::Skillset,
                name: "c".into(),
            },
            changes: vec![FieldChange {
                path: "fields.0.searchable".into(),
                from: JsonValue::Bool(false),
                to: JsonValue::Bool(true),
            }],
        });
        let s = diff.render();
        assert!(s.contains("+ indexes/a"), "{s}");
        assert!(s.contains("= data_sources/b"), "{s}");
        assert!(s.contains("~ skillsets/c"), "{s}");
        assert!(s.contains("fields.0.searchable: false → true"), "{s}");
    }

    #[test]
    fn format_rigg_err_appends_suggestion_for_forbidden() {
        let e = rigg_client::ClientError::Forbidden {
            service: "flir-ai-search.search.windows.net".to_string(),
            message: "Access denied".to_string(),
            body: String::new(),
        };
        let wrapped = format_rigg_err(e);
        let s = format!("{wrapped:#}");
        // Original rigg-client display text is preserved.
        assert!(
            s.contains("403 Forbidden"),
            "preserves the original error: {s}"
        );
        // Generic rigg suggestion block is appended.
        assert!(s.contains("Suggested fix:"), "appends suggestion: {s}");
        assert!(
            s.contains("aadOrApiKey"),
            "includes RBAC enablement hint: {s}"
        );
        // Quelch-specific tailored block uses the service name extracted
        // from the error.
        assert!(
            s.contains("flir-ai-search"),
            "substitutes the service name: {s}"
        );
        assert!(
            s.contains("Search Service Contributor"),
            "includes role assignment commands: {s}"
        );
        assert!(
            s.contains("Search Index Data Contributor"),
            "includes data-plane role: {s}"
        );
    }

    #[test]
    fn format_rigg_err_appends_suggestion_for_non_forbidden() {
        let e = rigg_client::ClientError::Auth(rigg_client::auth::AuthError::NotLoggedIn);
        let wrapped = format_rigg_err(e);
        let s = format!("{wrapped:#}");
        assert!(s.contains("Suggested fix:"), "{s}");
        assert!(s.contains("az login"), "{s}");
        // No tailored Forbidden block for a non-403 error.
        assert!(!s.contains("Search Service Contributor"), "{s}");
    }

    #[test]
    fn pending_excludes_match_entries() {
        let mut diff = RiggDiff::default();
        diff.changes.push(ResourceChange::Match(ResourceRef {
            kind: ResourceKind::Index,
            name: "a".into(),
        }));
        diff.changes.push(ResourceChange::Create(ResourceRef {
            kind: ResourceKind::Index,
            name: "b".into(),
        }));
        let pending: Vec<_> = diff.pending().collect();
        assert_eq!(pending.len(), 1);
        assert!(matches!(pending[0], ResourceChange::Create(_)));
    }

    #[test]
    fn diff_values_detects_leaf_changes() {
        let want = serde_json::json!({"x": 1});
        let have = serde_json::json!({"x": 2});
        let changes = diff_values(&want, &have, "");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "x");
        assert_eq!(changes[0].from, serde_json::json!(2));
        assert_eq!(changes[0].to, serde_json::json!(1));
    }

    #[tokio::test]
    async fn plan_ignores_server_managed_fields_in_live_state() {
        // The desired state has only a name + fields. The live state mirrors
        // it but adds the noisy server-managed metadata that AI Search returns
        // on every read (@odata.context, @odata.etag, etag). With the filter
        // in place, this must be a Match, not an Update.
        let state = desired_with_one_index("jira-issues");
        let live = serde_json::json!({
            "@odata.context": "https://srv.search.windows.net/$metadata#indexes/$entity",
            "@odata.etag": "\"abc123\"",
            "etag": "abc123",
            "name": "jira-issues",
            "fields": [],
        });
        let api = MockRiggApi::default().with_live(ResourceKind::Index, vec![live]);
        let diff = plan(&state, &api).await.unwrap();
        assert_eq!(diff.changes.len(), 1);
        assert!(
            matches!(diff.changes[0], ResourceChange::Match(_)),
            "expected Match after stripping server-managed fields, got: {:?}",
            diff.changes[0]
        );
        assert!(diff.is_clean());
    }

    #[test]
    fn strip_server_managed_fields_removes_odata_and_etag() {
        let mut v = serde_json::json!({
            "@odata.context": "ctx",
            "@odata.etag": "et",
            "@search.action": "merge",
            "etag": "abc",
            "name": "x",
            "fields": [
                {"name": "a", "@odata.type": "#Edm.String", "type": "Edm.String"},
            ],
        });
        strip_server_managed_fields(&mut v);
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("@odata.context"));
        assert!(!obj.contains_key("@odata.etag"));
        assert!(!obj.contains_key("@search.action"));
        assert!(!obj.contains_key("etag"));
        assert_eq!(obj.get("name").unwrap(), "x");
        // Nested objects also get scrubbed.
        let inner = &v["fields"][0];
        let inner_obj = inner.as_object().unwrap();
        assert!(!inner_obj.contains_key("@odata.type"));
        assert_eq!(inner_obj.get("name").unwrap(), "a");
    }
}
