//! `quelch instance list` and `quelch instance config` — emit per-instance
//! config slices.
//!
//! The Quelch user model is "one master `quelch.yaml` checked into git"; the
//! operator runs Q-Ingest / Q-MCP processes themselves on whatever host they
//! prefer. These commands surface the contents of the master config and let
//! the operator emit a slimmed slice (via [`crate::config::slice::slice_for_instance`])
//! ready to copy onto the host that runs each instance.

use std::io::Write;
use std::path::Path;

use anyhow::{Result, anyhow};
use clap::ValueEnum;

use crate::config::schema::{Config, InstanceKind};
use crate::config::slice::slice_for_instance;

/// CLI-facing instance-kind enum used by `quelch instance config --kind`.
///
/// Mirrors [`crate::config::InstanceKind`] but is defined here in the library
/// crate so the implementation can map between the two without depending on
/// the binary-only `cli` module.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstanceKindArg {
    /// Ingest worker instance (Q-Ingest).
    Ingest,
    /// MCP server instance (Q-MCP).
    Mcp,
}

impl From<InstanceKindArg> for InstanceKind {
    fn from(arg: InstanceKindArg) -> Self {
        match arg {
            InstanceKindArg::Ingest => InstanceKind::Ingest,
            InstanceKindArg::Mcp => InstanceKind::Mcp,
        }
    }
}

/// Render the instances declared in the master config as a two-column table:
/// instance name (padded to the longest name) followed by the instance kind.
///
/// If no instances are declared, prints `(no instances declared)`.
pub fn list(cfg: &Config, out: &mut dyn Write) -> Result<()> {
    if cfg.instances.is_empty() {
        writeln!(out, "(no instances declared)")?;
        return Ok(());
    }
    let name_w = cfg
        .instances
        .iter()
        .map(|i| i.name.len())
        .max()
        .unwrap_or(0);
    for inst in &cfg.instances {
        let kind = kind_label(inst.kind());
        writeln!(out, "  {:<name_w$}  {kind}", inst.name)?;
    }
    Ok(())
}

/// Lowercase, user-facing label for an [`InstanceKind`].
fn kind_label(k: InstanceKind) -> &'static str {
    match k {
        InstanceKind::Ingest => "ingest",
        InstanceKind::Mcp => "mcp",
    }
}

/// Slice the master config for the named instance, sanity-check the
/// declared kind, and emit the slice as YAML — to `output` if `Some`,
/// otherwise to `out` (typically stdout).
///
/// Errors if the instance is missing from the config or if its actual kind
/// does not match `declared_kind`.
pub fn config(
    cfg: &Config,
    instance_name: &str,
    declared_kind: InstanceKindArg,
    output: Option<&Path>,
    out: &mut dyn Write,
) -> Result<()> {
    let slice = slice_for_instance(cfg, instance_name)?;
    // `slice_for_instance` always retains exactly the requested instance,
    // so indexing [0] here is safe — but we still avoid `.unwrap()`.
    let actual_kind = slice
        .instances
        .first()
        .ok_or_else(|| anyhow!("internal: sliced config has no instance"))?
        .kind();
    let expected: InstanceKind = declared_kind.into();
    if actual_kind != expected {
        return Err(anyhow!(
            "instance '{}' has kind '{}' in the config but --kind was '{}'",
            instance_name,
            kind_label(actual_kind),
            kind_label(expected)
        ));
    }
    let yaml = serde_yaml::to_string(&slice)?;
    match output {
        Some(p) => {
            std::fs::write(p, yaml).map_err(|e| anyhow!("writing slice to {}: {e}", p.display()))?
        }
        None => write!(out, "{yaml}")?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Config {
        serde_yaml::from_str(include_str!("../config/slice_test_fixture.yaml"))
            .expect("fixture parses")
    }

    #[test]
    fn config_kind_mismatch_errors() {
        let cfg = fixture();
        let mut sink = Vec::new();
        let err = config(
            &cfg,
            "ingest-internal",
            InstanceKindArg::Mcp,
            None,
            &mut sink,
        )
        .expect_err("kind mismatch must error");
        let msg = err.to_string();
        assert!(
            msg.contains("'ingest'"),
            "error should mention actual kind 'ingest' (lowercase), got: {msg}"
        );
        assert!(
            msg.contains("'mcp'"),
            "error should mention requested kind 'mcp' (lowercase), got: {msg}"
        );
    }

    #[test]
    fn config_ingest_emits_slimmed_yaml() {
        let cfg = fixture();
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let mut sink = Vec::new();
        config(
            &cfg,
            "ingest-internal",
            InstanceKindArg::Ingest,
            Some(tmp.path()),
            &mut sink,
        )
        .expect("config write");
        let bytes = std::fs::read(tmp.path()).expect("read tempfile");
        let s = String::from_utf8(bytes).expect("utf8");

        assert!(
            !s.contains("subscription_id"),
            "control-plane subscription_id must be stripped: {s}"
        );
        assert!(
            !s.contains("resource_group"),
            "control-plane resource_group must be stripped: {s}"
        );
        assert!(
            !s.contains("account:"),
            "control-plane account must be stripped: {s}"
        );
        assert!(
            !s.contains("\nai:") && !s.starts_with("ai:"),
            "ai block must be stripped: {s}"
        );
        assert!(
            !s.contains("\nsearch:") && !s.contains("  search:"),
            "search must be stripped for ingest: {s}"
        );
        assert!(
            s.contains("source_connections:"),
            "source_connections must be present for ingest: {s}"
        );
    }

    #[test]
    fn config_mcp_emits_slimmed_yaml_with_search_kept() {
        let cfg = fixture();
        let mut sink = Vec::new();
        config(&cfg, "mcp-prod", InstanceKindArg::Mcp, None, &mut sink).expect("config write");
        let s = String::from_utf8(sink).expect("utf8");

        assert!(
            !s.contains("subscription_id"),
            "control-plane stripped: {s}"
        );
        assert!(
            !s.contains("\nai:") && !s.starts_with("ai:"),
            "ai must be stripped: {s}"
        );
        assert!(s.contains("search:"), "search must be kept for MCP: {s}");
        // For MCP slices, source_connections is emptied; serde_yaml will still
        // emit the (empty) sequence, which is fine.
    }

    #[test]
    fn config_unknown_instance_errors() {
        let cfg = fixture();
        let mut sink = Vec::new();
        let err = config(&cfg, "ghost", InstanceKindArg::Ingest, None, &mut sink)
            .expect_err("unknown instance must error");
        assert!(err.to_string().contains("ghost"));
    }

    #[test]
    fn list_prints_each_instance_with_kind() {
        let cfg = fixture();
        let mut sink = Vec::new();
        list(&cfg, &mut sink).expect("list");
        let s = String::from_utf8(sink).expect("utf8");
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 2, "fixture has two instances; got: {s:?}");
        // ingest-internal is 15 chars; mcp-prod is 8 chars; widest = 15.
        assert_eq!(lines[0], "  ingest-internal  ingest");
        assert_eq!(lines[1], "  mcp-prod         mcp");
    }

    #[test]
    fn list_empty_config_prints_placeholder() {
        let mut cfg = fixture();
        cfg.instances.clear();
        let mut sink = Vec::new();
        list(&cfg, &mut sink).expect("list");
        let s = String::from_utf8(sink).expect("utf8");
        assert_eq!(s.trim_end(), "(no instances declared)");
    }
}
