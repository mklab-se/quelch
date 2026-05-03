/// Fetch live Azure AI Search resources and write them back to local rigg files.
///
/// Pull is the inverse of push: it brings the live Azure state to disk so
/// operators can inspect it, hand-edit it, or start tracking it under source
/// control.
///
/// # Ownership markers
///
/// A file that already exists **and** carries the `# rigg:managed-by-user`
/// marker on its first line is left untouched. Its path is recorded in
/// [`PullOutcome::skipped_managed_by_user`].
///
/// # Diff-only mode
///
/// When [`PullOptions::diff_only`] is `true` nothing is written to disk.
/// [`PullOutcome::written`] is still populated with the paths that *would*
/// have been written, so callers can display a preview.
use std::path::{Path, PathBuf};

use rigg_core::resources::ResourceKind;

use crate::azure::rigg::ownership;
use crate::azure::rigg::plan::{MANAGED_KINDS, RiggApiAdapter, subdir_for_kind};

/// Options that control pull behaviour.
#[derive(Debug, Clone, Copy, Default)]
pub struct PullOptions {
    /// If `Some`, only pull resources of this kind.
    pub kind: Option<ResourceKind>,
    /// If `true`, compute what would change but don't write anything.
    pub diff_only: bool,
}

/// The result of a pull run.
#[derive(Debug, Default)]
pub struct PullOutcome {
    /// Paths that were (or would be, if `diff_only`) written.
    pub written: Vec<PathBuf>,
    /// Paths skipped because they carry the `# rigg:managed-by-user` marker.
    pub skipped_managed_by_user: Vec<PathBuf>,
}

/// Errors that can occur during a pull.
#[derive(Debug, thiserror::Error)]
pub enum PullError {
    #[error("rigg: {0}")]
    Rigg(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(String),
}

/// Fetch all live resources and write them to `rigg_dir`.
///
/// `api` is the rigg adapter. `options` controls filtering and dry-run
/// behaviour. See [`PullOptions`] for details.
pub async fn run<A: RiggApiAdapter>(
    rigg_dir: &Path,
    api: &A,
    options: PullOptions,
) -> Result<PullOutcome, PullError> {
    let mut outcome = PullOutcome::default();

    let filtered: Vec<ResourceKind> = match options.kind {
        Some(k) => vec![k],
        None => MANAGED_KINDS.to_vec(),
    };

    for kind in &filtered {
        pull_kind(*kind, rigg_dir, api, options.diff_only, &mut outcome).await?;
    }

    Ok(outcome)
}

/// Pull all resources of a single kind.
async fn pull_kind<A: RiggApiAdapter>(
    kind: ResourceKind,
    rigg_dir: &Path,
    api: &A,
    diff_only: bool,
    outcome: &mut PullOutcome,
) -> Result<(), PullError> {
    let items = api
        .list_resources(kind)
        .await
        .map_err(|e| PullError::Rigg(e.to_string()))?;

    let subdir = rigg_dir.join(subdir_for_kind(kind));

    for item in items {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if name.is_empty() {
            continue;
        }

        let path = subdir.join(format!("{name}.yaml"));

        // Respect managed-by-user marker.
        if ownership::is_managed_by_user(&path) {
            outcome.skipped_managed_by_user.push(path);
            continue;
        }

        // Serialise the JSON body to YAML.
        let yaml_text = json_to_yaml_string(&item).map_err(|e| PullError::Json(e.to_string()))?;

        if diff_only {
            outcome.written.push(path);
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, yaml_text.as_bytes())?;
            outcome.written.push(path);
        }
    }

    Ok(())
}

/// Convert a `serde_json::Value` to a YAML string.
fn json_to_yaml_string(val: &serde_json::Value) -> Result<String, serde_yaml::Error> {
    // Route through serde_yaml's serialiser for clean YAML output.
    let yaml_val: serde_yaml::Value = serde_yaml::to_value(val).unwrap_or(serde_yaml::Value::Null);
    serde_yaml::to_string(&yaml_val)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::azure::rigg::plan::tests::MockRiggApi;
    use std::io::Write as _;

    fn live_index(name: &str) -> serde_json::Value {
        serde_json::json!({ "name": name, "fields": [] })
    }

    #[tokio::test]
    async fn pull_writes_live_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let api =
            MockRiggApi::default().with_live(ResourceKind::Index, vec![live_index("jira-issues")]);

        let outcome = run(dir.path(), &api, PullOptions::default()).await.unwrap();

        assert_eq!(outcome.written.len(), 1);
        assert!(outcome.skipped_managed_by_user.is_empty());

        let path = dir.path().join("indexes/jira-issues.yaml");
        assert!(path.exists(), "file should have been written");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("jira-issues"),
            "YAML should contain the resource name"
        );
    }

    #[tokio::test]
    async fn pull_respects_managed_by_user_marker() {
        let dir = tempfile::tempdir().unwrap();

        // Pre-create the file with the marker.
        let indexes_dir = dir.path().join("indexes");
        std::fs::create_dir_all(&indexes_dir).unwrap();
        let file_path = indexes_dir.join("jira-issues.yaml");
        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(b"# rigg:managed-by-user\nname: jira-issues\n")
            .unwrap();
        drop(f);

        let api =
            MockRiggApi::default().with_live(ResourceKind::Index, vec![live_index("jira-issues")]);

        let outcome = run(dir.path(), &api, PullOptions::default()).await.unwrap();

        assert!(
            outcome.written.is_empty(),
            "marked file must not be overwritten"
        );
        assert_eq!(outcome.skipped_managed_by_user.len(), 1);
        assert_eq!(outcome.skipped_managed_by_user[0], file_path);

        // File contents must be unchanged.
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(content.starts_with("# rigg:managed-by-user"));
    }

    #[tokio::test]
    async fn pull_diff_only_does_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let api =
            MockRiggApi::default().with_live(ResourceKind::Index, vec![live_index("jira-issues")]);

        let outcome = run(
            dir.path(),
            &api,
            PullOptions {
                diff_only: true,
                kind: None,
            },
        )
        .await
        .unwrap();

        // written contains would-write paths but no file was created.
        assert_eq!(
            outcome.written.len(),
            1,
            "diff_only must still populate written"
        );
        let path = dir.path().join("indexes/jira-issues.yaml");
        assert!(!path.exists(), "diff_only must NOT write to disk");
        assert!(outcome.skipped_managed_by_user.is_empty());
    }

    #[tokio::test]
    async fn pull_kind_filter_only_pulls_that_kind() {
        let dir = tempfile::tempdir().unwrap();
        let api = MockRiggApi::default()
            .with_live(ResourceKind::Index, vec![live_index("my-index")])
            .with_live(
                ResourceKind::DataSource,
                vec![serde_json::json!({"name": "my-ds"})],
            );

        let outcome = run(
            dir.path(),
            &api,
            PullOptions {
                kind: Some(ResourceKind::Index),
                diff_only: false,
            },
        )
        .await
        .unwrap();

        // Only the index should have been written.
        assert_eq!(outcome.written.len(), 1);
        assert!(dir.path().join("indexes/my-index.yaml").exists());
        assert!(!dir.path().join("datasources/my-ds.yaml").exists());
    }
}
