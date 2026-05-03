/// Write generated rigg files to disk, respecting hand-takeover markers and
/// the global `RiggConfig::ownership` setting.
///
/// The layout under the root directory follows the rigg convention:
///
/// ```text
/// rigg/
/// ├── indexes/
/// ├── datasources/
/// ├── skillsets/
/// ├── indexers/
/// ├── knowledge_sources/
/// └── knowledge_bases/
/// ```
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::azure::rigg::{GeneratedRiggFiles, ownership};
use crate::config::{RiggConfig, RiggOwnership};

/// The outcome of a `write_to_disk` call.
#[derive(Debug, Default)]
pub struct WriteOutcome {
    /// Files that were created or overwritten.
    pub written: Vec<PathBuf>,
    /// Files that were left untouched because they are user-managed.
    pub skipped: Vec<PathBuf>,
}

/// Errors that can occur while writing rigg files to disk.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Write generated rigg files to `root`, respecting:
///
/// * **Per-file `# rigg:managed-by-user` marker** — if the first line of an
///   existing file contains the marker, it is never overwritten regardless of
///   the global ownership setting.
/// * **`RiggConfig::ownership`** — if set to [`RiggOwnership::ManagedByUser`],
///   existing files that *don't* carry the per-file marker are also preserved;
///   only missing files are created.
///
/// Files recorded in [`WriteOutcome::skipped`] were left on disk unchanged.
/// Files recorded in [`WriteOutcome::written`] were created or overwritten.
pub fn write_to_disk(
    files: &GeneratedRiggFiles,
    rigg_config: &RiggConfig,
    root: &Path,
) -> Result<WriteOutcome, WriteError> {
    let mut outcome = WriteOutcome::default();

    let groups: &[(&str, &std::collections::HashMap<String, String>)] = &[
        ("indexes", &files.indexes),
        ("datasources", &files.datasources),
        ("skillsets", &files.skillsets),
        ("indexers", &files.indexers),
        ("knowledge_sources", &files.knowledge_sources),
        ("knowledge_bases", &files.knowledge_bases),
    ];

    for (subdir, map) in groups {
        for (name, yaml) in *map {
            let path = root.join(subdir).join(format!("{name}.yaml"));

            if should_skip(&path, rigg_config) {
                outcome.skipped.push(path);
            } else {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, yaml.as_bytes())?;
                outcome.written.push(path);
            }
        }
    }

    Ok(outcome)
}

/// Decide whether a file at `path` should be left untouched.
///
/// Returns `true` (skip) when:
/// 1. The file exists and has the per-file `# rigg:managed-by-user` marker, OR
/// 2. The file exists and the global ownership is `ManagedByUser` (no per-file
///    marker needed — the global setting is sufficient to protect the file).
///
/// Returns `false` (write/overwrite) when the file does not exist, regardless
/// of the global ownership — missing files are always created.
fn should_skip(path: &Path, rigg_config: &RiggConfig) -> bool {
    if !path.exists() {
        // Always create missing files, no matter what ownership says.
        return false;
    }

    // Per-file marker wins over everything — always skip.
    if ownership::is_managed_by_user(path) {
        return true;
    }

    // File exists without per-file marker: respect the global ownership flag.
    matches!(rigg_config.ownership, RiggOwnership::ManagedByUser)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn rigg_config(ownership: RiggOwnership) -> RiggConfig {
        RiggConfig {
            dir: "./rigg".to_string(),
            ownership,
        }
    }

    fn single_index_files(name: &str, content: &str) -> GeneratedRiggFiles {
        let mut files = GeneratedRiggFiles::default();
        files.indexes.insert(name.to_string(), content.to_string());
        files
    }

    fn one_of_each_files() -> GeneratedRiggFiles {
        let mut files = GeneratedRiggFiles::default();
        files
            .indexes
            .insert("idx".to_string(), "index: yaml\n".to_string());
        files
            .datasources
            .insert("ds".to_string(), "datasource: yaml\n".to_string());
        files
            .skillsets
            .insert("ss".to_string(), "skillset: yaml\n".to_string());
        files
            .indexers
            .insert("ir".to_string(), "indexer: yaml\n".to_string());
        files
            .knowledge_sources
            .insert("ks".to_string(), "ks: yaml\n".to_string());
        files
            .knowledge_bases
            .insert("kb".to_string(), "kb: yaml\n".to_string());
        files
    }

    // -----------------------------------------------------------------------

    #[test]
    fn write_to_disk_creates_files_in_correct_subdirs() {
        let dir = tempdir().unwrap();
        let files = one_of_each_files();
        let cfg = rigg_config(RiggOwnership::Generated);

        let outcome = write_to_disk(&files, &cfg, dir.path()).unwrap();

        assert_eq!(outcome.skipped.len(), 0);
        assert_eq!(outcome.written.len(), 6);

        assert!(dir.path().join("indexes/idx.yaml").exists());
        assert!(dir.path().join("datasources/ds.yaml").exists());
        assert!(dir.path().join("skillsets/ss.yaml").exists());
        assert!(dir.path().join("indexers/ir.yaml").exists());
        assert!(dir.path().join("knowledge_sources/ks.yaml").exists());
        assert!(dir.path().join("knowledge_bases/kb.yaml").exists());
    }

    #[test]
    fn write_to_disk_overwrites_when_ownership_is_generated() {
        let dir = tempdir().unwrap();
        let indexes_dir = dir.path().join("indexes");
        fs::create_dir_all(&indexes_dir).unwrap();
        let file_path = indexes_dir.join("jira-issues.yaml");
        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(b"old content\n").unwrap();
        drop(f);

        let files = single_index_files("jira-issues", "new content\n");
        let cfg = rigg_config(RiggOwnership::Generated);

        let outcome = write_to_disk(&files, &cfg, dir.path()).unwrap();

        assert_eq!(outcome.skipped.len(), 0);
        assert_eq!(outcome.written.len(), 1);

        let on_disk = fs::read_to_string(&file_path).unwrap();
        assert_eq!(on_disk, "new content\n");
    }

    #[test]
    fn write_to_disk_preserves_managed_by_user_files() {
        let dir = tempdir().unwrap();
        let indexes_dir = dir.path().join("indexes");
        fs::create_dir_all(&indexes_dir).unwrap();
        let file_path = indexes_dir.join("jira-issues.yaml");
        let original = "# rigg:managed-by-user\nname: custom\n";
        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(original.as_bytes()).unwrap();
        drop(f);

        // Even with Generated ownership the per-file marker must protect it.
        let files = single_index_files("jira-issues", "should not be written\n");
        let cfg = rigg_config(RiggOwnership::Generated);

        let outcome = write_to_disk(&files, &cfg, dir.path()).unwrap();

        assert_eq!(outcome.written.len(), 0);
        assert_eq!(outcome.skipped.len(), 1);
        assert_eq!(outcome.skipped[0], file_path);

        let on_disk = fs::read_to_string(&file_path).unwrap();
        assert_eq!(on_disk, original);
    }

    #[test]
    fn write_to_disk_preserves_existing_files_when_ownership_is_managed_by_user() {
        let dir = tempdir().unwrap();
        let indexes_dir = dir.path().join("indexes");
        fs::create_dir_all(&indexes_dir).unwrap();
        let file_path = indexes_dir.join("jira-issues.yaml");
        let original = "name: existing\n"; // No per-file marker.
        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(original.as_bytes()).unwrap();
        drop(f);

        let files = single_index_files("jira-issues", "should not be written\n");
        let cfg = rigg_config(RiggOwnership::ManagedByUser);

        let outcome = write_to_disk(&files, &cfg, dir.path()).unwrap();

        assert_eq!(outcome.written.len(), 0);
        assert_eq!(outcome.skipped.len(), 1);

        let on_disk = fs::read_to_string(&file_path).unwrap();
        assert_eq!(on_disk, original);
    }

    #[test]
    fn write_to_disk_creates_missing_files_when_ownership_is_managed_by_user() {
        let dir = tempdir().unwrap();
        // Do NOT pre-create the file.
        let files = single_index_files("jira-issues", "generated content\n");
        let cfg = rigg_config(RiggOwnership::ManagedByUser);

        let outcome = write_to_disk(&files, &cfg, dir.path()).unwrap();

        assert_eq!(outcome.written.len(), 1);
        assert_eq!(outcome.skipped.len(), 0);

        let on_disk = fs::read_to_string(dir.path().join("indexes/jira-issues.yaml")).unwrap();
        assert_eq!(on_disk, "generated content\n");
    }
}
