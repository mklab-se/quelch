use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::info;

/// v2 top-level state: per-source container holding per-subsource progress.
#[derive(Debug, Serialize, Deserialize)]
pub struct SyncState {
    pub version: u32,
    pub sources: HashMap<String, SourceState>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceState {
    #[serde(default)]
    pub last_sync_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub sync_count: u64,
    #[serde(default)]
    pub subsources: HashMap<String, SubsourceState>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubsourceState {
    pub last_cursor: Option<DateTime<Utc>>,
    pub last_sync_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub documents_synced: u64,
    #[serde(default)]
    pub last_sample_id: Option<String>,
}

// -----------------------------------------------------------------------
// v1 compatibility — only used during migration.
// -----------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct V1State {
    #[serde(default)]
    #[allow(dead_code)]
    version: u32,
    #[serde(default)]
    sources: HashMap<String, V1SourceState>,
}

#[derive(Debug, Deserialize)]
struct V1SourceState {
    last_cursor: Option<DateTime<Utc>>,
    last_sync_at: Option<DateTime<Utc>>,
    #[serde(default)]
    documents_synced: u64,
    #[serde(default)]
    sync_count: u64,
}

impl Default for SyncState {
    fn default() -> Self {
        Self {
            version: 2,
            sources: HashMap::new(),
        }
    }
}

impl SyncState {
    /// Load state from disk. If the file is v1, migrate to v2 using
    /// `subsources_by_source` to expand the legacy per-source cursor into
    /// per-subsource cursors. Pass `&[]` if you don't need migration
    /// expansion (e.g., in simple unit tests with no v1 file).
    pub fn load(path: &Path, subsources_by_source: &[(String, Vec<String>)]) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read_to_string(path).context("failed to read sync state file")?;
        let peek: serde_json::Value =
            serde_json::from_str(&data).context("failed to parse sync state file")?;
        let version = peek.get("version").and_then(|v| v.as_u64()).unwrap_or(2);
        if version == 1 {
            let v1: V1State = serde_json::from_value(peek)?;
            let migrated = migrate_v1_to_v2(v1, subsources_by_source);
            info!(
                path = %path.display(),
                "Migrated sync state from v1 to v2",
            );
            Ok(migrated)
        } else {
            let v2: SyncState =
                serde_json::from_str(&data).context("failed to parse sync state file (v2)")?;
            Ok(v2)
        }
    }

    /// Save state to a JSON file atomically (write to temp, then rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        let data = serde_json::to_string_pretty(self).context("failed to serialize sync state")?;
        let tmp_path = path.with_extension("tmp");
        std::fs::write(&tmp_path, &data).context("failed to write sync state temp file")?;
        std::fs::rename(&tmp_path, path).context("failed to rename sync state file")?;
        Ok(())
    }

    /// Get state for a source (returns default if not found).
    pub fn get_source(&self, name: &str) -> SourceState {
        self.sources.get(name).cloned().unwrap_or_default()
    }

    /// Update per-subsource state after a successful sync batch.
    pub fn update_subsource(
        &mut self,
        source: &str,
        subsource: &str,
        cursor: DateTime<Utc>,
        docs_synced: u64,
        last_sample_id: Option<String>,
    ) {
        let src = self.sources.entry(source.to_string()).or_default();
        src.last_sync_at = Some(Utc::now());
        let sub = src.subsources.entry(subsource.to_string()).or_default();
        sub.last_cursor = Some(cursor);
        sub.last_sync_at = Some(Utc::now());
        sub.documents_synced += docs_synced;
        if last_sample_id.is_some() {
            sub.last_sample_id = last_sample_id;
        }
    }

    /// Record that a full cycle has finished for `source` — increments sync_count by 1.
    pub fn complete_source_cycle(&mut self, source: &str) {
        let src = self.sources.entry(source.to_string()).or_default();
        src.sync_count += 1;
        src.last_sync_at = Some(Utc::now());
    }

    /// Reset state. If `subsource` is Some, only that subsource is cleared.
    /// Otherwise the entire source entry is removed.
    pub fn reset_source(&mut self, name: &str, subsource: Option<&str>) {
        match subsource {
            Some(key) => {
                if let Some(src) = self.sources.get_mut(name) {
                    src.subsources.remove(key);
                }
            }
            None => {
                self.sources.remove(name);
            }
        }
    }

    pub fn reset_all(&mut self) {
        self.sources.clear();
    }
}

fn migrate_v1_to_v2(v1: V1State, subsources_by_source: &[(String, Vec<String>)]) -> SyncState {
    let mut out = SyncState::default();
    for (name, old) in v1.sources {
        let mut src = SourceState {
            last_sync_at: old.last_sync_at,
            sync_count: old.sync_count,
            subsources: HashMap::new(),
        };
        let subs = subsources_by_source
            .iter()
            .find_map(|(n, ss)| if n == &name { Some(ss.clone()) } else { None })
            .unwrap_or_else(|| vec!["_".to_string()]);
        for sub_key in subs {
            src.subsources.insert(
                sub_key,
                SubsourceState {
                    last_cursor: old.last_cursor,
                    last_sync_at: old.last_sync_at,
                    documents_synced: old.documents_synced,
                    last_sample_id: None,
                },
            );
        }
        out.sources.insert(name, src);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn default_state_is_v2() {
        let state = SyncState::default();
        assert_eq!(state.version, 2);
        assert!(state.sources.is_empty());
    }

    #[test]
    fn load_returns_default_if_file_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let state = SyncState::load(&path, &[]).unwrap();
        assert!(state.sources.is_empty());
    }

    #[test]
    fn update_subsource_accumulates() {
        let mut state = SyncState::default();
        let t = Utc::now();
        state.update_subsource("my-jira", "DO", t, 5, Some("DO-1".into()));
        state.update_subsource("my-jira", "DO", t, 7, Some("DO-9".into()));
        let s = state.get_source("my-jira");
        let sub = s.subsources.get("DO").unwrap();
        assert_eq!(sub.documents_synced, 12);
        assert_eq!(sub.last_sample_id.as_deref(), Some("DO-9"));
    }

    #[test]
    fn complete_source_cycle_increments_sync_count_once() {
        let mut state = SyncState::default();
        state.update_subsource("s", "A", Utc::now(), 5, None);
        state.update_subsource("s", "A", Utc::now(), 7, None);
        state.complete_source_cycle("s");
        assert_eq!(state.get_source("s").sync_count, 1);
        state.complete_source_cycle("s");
        assert_eq!(state.get_source("s").sync_count, 2);
    }

    #[test]
    fn migrates_v1_to_v2_copies_cursor_to_all_subsources() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        let v1_json = r#"{
            "version": 1,
            "sources": {
                "my-jira": {
                    "last_cursor": "2026-01-15T10:00:00Z",
                    "last_sync_at": "2026-01-15T10:01:00Z",
                    "documents_synced": 42,
                    "sync_count": 3
                }
            }
        }"#;
        std::fs::write(&path, v1_json).unwrap();
        let expected: Vec<(String, Vec<String>)> = vec![(
            "my-jira".to_string(),
            vec!["DO".to_string(), "HR".to_string()],
        )];
        let state = SyncState::load(&path, &expected).unwrap();
        assert_eq!(state.version, 2);
        let s = state.get_source("my-jira");
        assert_eq!(s.sync_count, 3);
        let do_sub = s.subsources.get("DO").unwrap();
        let hr_sub = s.subsources.get("HR").unwrap();
        assert!(do_sub.last_cursor.is_some());
        assert!(hr_sub.last_cursor.is_some());
        assert_eq!(do_sub.last_cursor, hr_sub.last_cursor);
    }

    #[test]
    fn save_then_load_v2_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        let mut state = SyncState::default();
        let t = Utc::now();
        state.update_subsource("my-jira", "DO", t, 3, Some("DO-5".into()));
        state.save(&path).unwrap();

        let loaded = SyncState::load(&path, &[]).unwrap();
        assert_eq!(loaded.version, 2);
        let sub = loaded
            .get_source("my-jira")
            .subsources
            .get("DO")
            .cloned()
            .unwrap();
        assert_eq!(sub.documents_synced, 3);
        assert_eq!(sub.last_sample_id.as_deref(), Some("DO-5"));
    }

    #[test]
    fn reset_source_clears_all_subsources() {
        let mut state = SyncState::default();
        state.update_subsource("s", "A", Utc::now(), 1, None);
        state.update_subsource("s", "B", Utc::now(), 1, None);
        state.reset_source("s", None);
        assert!(state.get_source("s").subsources.is_empty());
    }

    #[test]
    fn reset_source_single_subsource() {
        let mut state = SyncState::default();
        state.update_subsource("s", "A", Utc::now(), 1, None);
        state.update_subsource("s", "B", Utc::now(), 1, None);
        state.reset_source("s", Some("A"));
        let src = state.get_source("s");
        assert!(!src.subsources.contains_key("A"));
        assert!(src.subsources.contains_key("B"));
    }
}
