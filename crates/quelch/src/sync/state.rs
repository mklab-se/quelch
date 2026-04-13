use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncState {
    pub version: u32,
    pub sources: HashMap<String, SourceState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceState {
    pub last_cursor: Option<DateTime<Utc>>,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub documents_synced: u64,
    pub sync_count: u64,
}

impl Default for SyncState {
    fn default() -> Self {
        Self {
            version: 1,
            sources: HashMap::new(),
        }
    }
}

impl SyncState {
    /// Load state from a JSON file. Returns default state if file doesn't exist.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read_to_string(path).context("failed to read sync state file")?;
        let state: SyncState =
            serde_json::from_str(&data).context("failed to parse sync state file")?;
        Ok(state)
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
        self.sources.get(name).cloned().unwrap_or(SourceState {
            last_cursor: None,
            last_sync_at: None,
            documents_synced: 0,
            sync_count: 0,
        })
    }

    /// Update state for a source after a successful sync batch.
    pub fn update_source(&mut self, name: &str, cursor: DateTime<Utc>, docs_synced: u64) {
        let entry = self.sources.entry(name.to_string()).or_insert(SourceState {
            last_cursor: None,
            last_sync_at: None,
            documents_synced: 0,
            sync_count: 0,
        });
        entry.last_cursor = Some(cursor);
        entry.last_sync_at = Some(Utc::now());
        entry.documents_synced += docs_synced;
        entry.sync_count += 1;
    }

    pub fn reset_source(&mut self, name: &str) {
        self.sources.remove(name);
    }

    pub fn reset_all(&mut self) {
        self.sources.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn default_state_has_no_sources() {
        let state = SyncState::default();
        assert_eq!(state.version, 1);
        assert!(state.sources.is_empty());
    }

    #[test]
    fn load_returns_default_if_file_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let state = SyncState::load(&path).unwrap();
        assert!(state.sources.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");

        let mut state = SyncState::default();
        state.update_source("test", Utc::now(), 42);
        state.save(&path).unwrap();

        let loaded = SyncState::load(&path).unwrap();
        assert_eq!(loaded.version, 1);
        let source = loaded.get_source("test");
        assert!(source.last_cursor.is_some());
        assert_eq!(source.documents_synced, 42);
        assert_eq!(source.sync_count, 1);
    }

    #[test]
    fn update_accumulates_counts() {
        let mut state = SyncState::default();
        state.update_source("s1", Utc::now(), 10);
        state.update_source("s1", Utc::now(), 20);
        let source = state.get_source("s1");
        assert_eq!(source.documents_synced, 30);
        assert_eq!(source.sync_count, 2);
    }

    #[test]
    fn reset_source_removes_it() {
        let mut state = SyncState::default();
        state.update_source("s1", Utc::now(), 10);
        state.reset_source("s1");
        let source = state.get_source("s1");
        assert!(source.last_cursor.is_none());
        assert_eq!(source.documents_synced, 0);
    }

    #[test]
    fn get_source_returns_default_for_unknown() {
        let state = SyncState::default();
        let source = state.get_source("unknown");
        assert!(source.last_cursor.is_none());
        assert_eq!(source.documents_synced, 0);
    }
}
