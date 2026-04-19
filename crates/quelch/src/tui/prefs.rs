//! Persisted TUI preferences: collapsed sections, focus, log-view toggle.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const CURRENT_PREFS_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    pub version: u32,
    #[serde(default)]
    pub collapsed_sources: HashSet<String>,
    #[serde(default)]
    pub collapsed_subsources: HashMap<String, HashSet<String>>,
    #[serde(default)]
    pub log_view_on: bool,
    #[serde(default = "default_focus")]
    pub focus: String,
}

fn default_focus() -> String {
    "sources".into()
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            version: CURRENT_PREFS_VERSION,
            collapsed_sources: HashSet::new(),
            collapsed_subsources: HashMap::new(),
            log_view_on: false,
            focus: default_focus(),
        }
    }
}

impl Prefs {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path).context("read prefs")?;
        match serde_json::from_str::<Self>(&raw) {
            Ok(p) => Ok(p),
            Err(e) => {
                tracing::warn!(error = %e, "prefs file unreadable — using defaults");
                Ok(Self::default())
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn toggle_source_collapsed(&mut self, src: &str) {
        if !self.collapsed_sources.remove(src) {
            self.collapsed_sources.insert(src.into());
        }
    }

    pub fn toggle_subsource_collapsed(&mut self, src: &str, sub: &str) {
        let set = self
            .collapsed_subsources
            .entry(src.to_string())
            .or_default();
        if !set.remove(sub) {
            set.insert(sub.into());
        }
    }

    pub fn is_source_collapsed(&self, src: &str) -> bool {
        self.collapsed_sources.contains(src)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn roundtrip_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ui.json");
        Prefs::default().save(&path).unwrap();
        let loaded = Prefs::load(&path).unwrap();
        assert_eq!(loaded.version, CURRENT_PREFS_VERSION);
        assert!(loaded.collapsed_sources.is_empty());
        assert!(!loaded.log_view_on);
    }

    #[test]
    fn toggle_source_collapsed() {
        let mut p = Prefs::default();
        p.toggle_source_collapsed("s");
        assert!(p.is_source_collapsed("s"));
        p.toggle_source_collapsed("s");
        assert!(!p.is_source_collapsed("s"));
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ui.json");
        std::fs::write(&path, "not valid json").unwrap();
        let loaded = Prefs::load(&path).unwrap();
        assert_eq!(loaded.version, CURRENT_PREFS_VERSION);
    }
}
