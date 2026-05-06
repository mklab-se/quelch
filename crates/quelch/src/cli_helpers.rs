//! Small helpers shared across CLI dispatch arms.
//!
//! At the moment this is just [`resolve_instance`], which centralises the
//! "pick the right instance from the config" logic used by `quelch ingest`,
//! `quelch mcp`, and the operator commands (`query`, `get`, `search`).

use anyhow::{Result, anyhow};

use crate::config::schema::{Config, InstanceKind};

/// Human-readable, lowercase rendering of an [`InstanceKind`] for error
/// messages. Keeps the rest of the CLI's lowercase convention (`ingest`/`mcp`
/// rather than the `{:?}` form `Ingest`/`Mcp`).
fn kind_label(kind: InstanceKind) -> &'static str {
    match kind {
        InstanceKind::Ingest => "ingest",
        InstanceKind::Mcp => "mcp",
    }
}

/// Resolve the instance name to run as.
///
/// * If `explicit` is `Some(name)`, look up that instance and verify its
///   kind matches `want_kind`.
/// * Otherwise auto-detect: if the config has exactly one instance of
///   `want_kind`, return it; if there are multiple, error with the
///   candidates listed; if there are none, error.
///
/// The returned `&str` borrows from `cfg`.
pub fn resolve_instance<'a>(
    cfg: &'a Config,
    explicit: Option<&str>,
    want_kind: InstanceKind,
) -> Result<&'a str> {
    if let Some(name) = explicit {
        let inst = cfg
            .instances
            .iter()
            .find(|i| i.name == name)
            .ok_or_else(|| anyhow!("instance '{name}' not found"))?;
        if inst.kind() != want_kind {
            return Err(anyhow!(
                "instance '{name}' is {}, expected {}",
                kind_label(inst.kind()),
                kind_label(want_kind)
            ));
        }
        return Ok(&inst.name);
    }
    let candidates: Vec<&str> = cfg
        .instances
        .iter()
        .filter(|i| i.kind() == want_kind)
        .map(|i| i.name.as_str())
        .collect();
    match candidates.as_slice() {
        [single] => Ok(single),
        [] => Err(anyhow!(
            "no {} instances declared in config",
            kind_label(want_kind)
        )),
        many => Err(anyhow!(
            "multiple {} instances ({}); pass --instance to disambiguate",
            kind_label(want_kind),
            many.join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{
        AzureConfig, Config, ContainerLayout, CosmosConfig, IngestInstance, InstanceConfig,
        InstanceKind, InstanceSpec, McpInstance,
    };

    fn cfg_with_instances(specs: &[(&str, InstanceKind)]) -> Config {
        let instances = specs
            .iter()
            .map(|(name, kind)| InstanceConfig {
                name: (*name).to_string(),
                spec: match kind {
                    InstanceKind::Ingest => InstanceSpec::Ingest(IngestInstance {
                        connections: vec![],
                        cycle_interval: std::time::Duration::from_secs(60),
                    }),
                    InstanceKind::Mcp => InstanceSpec::Mcp(McpInstance {
                        expose: vec![],
                        api_key: "k".into(),
                        knowledge_base: "kb".into(),
                        listen: "127.0.0.1:8080".into(),
                    }),
                },
            })
            .collect();
        Config {
            azure: AzureConfig {
                cosmos: CosmosConfig {
                    subscription_id: None,
                    resource_group: None,
                    account: None,
                    endpoint: "https://x".into(),
                    database: "quelch".into(),
                    containers: ContainerLayout::default(),
                    meta_container: "quelch-meta".into(),
                },
                search: None,
                ai: None,
            },
            source_connections: vec![],
            instances,
        }
    }

    #[test]
    fn resolve_picks_single_ingest_when_no_flag() {
        let cfg = cfg_with_instances(&[("only", InstanceKind::Ingest)]);
        assert_eq!(
            resolve_instance(&cfg, None, InstanceKind::Ingest).unwrap(),
            "only"
        );
    }

    #[test]
    fn resolve_requires_flag_when_multiple_match() {
        let cfg = cfg_with_instances(&[("a", InstanceKind::Ingest), ("b", InstanceKind::Ingest)]);
        let err = resolve_instance(&cfg, None, InstanceKind::Ingest).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("a") && msg.contains("b"), "msg: {msg}");
        assert!(msg.contains("--instance"), "msg: {msg}");
    }

    #[test]
    fn resolve_errors_on_kind_mismatch() {
        let cfg = cfg_with_instances(&[("only", InstanceKind::Mcp)]);
        let err = resolve_instance(&cfg, Some("only"), InstanceKind::Ingest).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("expected") || msg.contains("ingest"),
            "msg: {msg}"
        );
    }

    #[test]
    fn resolve_errors_when_no_matching_kind() {
        let cfg = cfg_with_instances(&[("a", InstanceKind::Mcp)]);
        let err = resolve_instance(&cfg, None, InstanceKind::Ingest).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no") && msg.contains("ingest"), "msg: {msg}");
    }
}
