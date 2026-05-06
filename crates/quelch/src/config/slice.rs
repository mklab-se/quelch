//! Per-instance slicing of the master [`Config`].
//!
//! The Quelch user model is "one master `quelch.yaml` checked into git".
//! Each instance only needs the parts relevant to it (source credentials for
//! ingest; AI Search endpoint for MCP). [`slice_for_instance`] takes a master
//! config and emits a slimmed copy ready to ship to the host that runs the
//! named instance.

use crate::config::schema::{Config, InstanceKind, InstanceSpec};

/// Slice the master `cfg` down to what `instance_name` needs to run.
///
/// - Strips Azure control-plane fields (`subscription_id`, `resource_group`,
///   `account`) and the `ai` block — neither is needed at runtime.
/// - For ingest instances: keeps only the `source_connections` referenced by
///   the instance, and removes `search`.
/// - For MCP instances: removes `source_connections` entirely and keeps
///   `search`.
/// - Replaces `instances[]` with a single-element list containing only the
///   requested instance.
///
/// # Errors
/// Returns an error if `instance_name` is not declared in `cfg`.
pub fn slice_for_instance(cfg: &Config, instance_name: &str) -> anyhow::Result<Config> {
    let instance = cfg
        .instances
        .iter()
        .find(|i| i.name == instance_name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "instance '{}' not found in config (have: {})",
                instance_name,
                cfg.instances
                    .iter()
                    .map(|i| i.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?
        .clone();

    let mut sliced = cfg.clone();

    sliced.azure.cosmos.subscription_id = None;
    sliced.azure.cosmos.resource_group = None;
    sliced.azure.cosmos.account = None;
    sliced.azure.ai = None;

    match instance.kind() {
        InstanceKind::Ingest => {
            sliced.azure.search = None;

            let connections = match &instance.spec {
                InstanceSpec::Ingest(i) => i.connections.clone(),
                _ => unreachable!(),
            };
            sliced
                .source_connections
                .retain(|c| connections.contains(&c.name));
        }
        InstanceKind::Mcp => {
            sliced.source_connections.clear();
        }
    }

    sliced.instances = vec![instance];
    Ok(sliced)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Config {
        serde_yaml::from_str(include_str!("slice_test_fixture.yaml")).expect("fixture parses")
    }

    #[test]
    fn slices_ingest_instance_strips_search_and_ai() {
        let cfg = fixture();
        let slice = slice_for_instance(&cfg, "ingest-internal").expect("slice");
        assert!(slice.azure.search.is_none(), "search must be stripped");
        assert!(slice.azure.ai.is_none(), "ai must be stripped");
        assert!(slice.azure.cosmos.subscription_id.is_none());
        assert!(slice.azure.cosmos.resource_group.is_none());
        assert!(slice.azure.cosmos.account.is_none());
        assert_eq!(slice.instances.len(), 1);
        assert_eq!(slice.instances[0].name, "ingest-internal");
    }

    #[test]
    fn slices_ingest_instance_keeps_only_referenced_connections() {
        let cfg = fixture();
        let slice = slice_for_instance(&cfg, "ingest-internal").expect("slice");
        let names: Vec<_> = slice.source_connections.iter().map(|c| &c.name).collect();
        assert!(names.contains(&&"jira-x".to_string()));
        assert!(names.contains(&&"jira-y".to_string()));
        assert!(!names.iter().any(|n| n.as_str() == "confluence-internal"));
    }

    #[test]
    fn slices_mcp_instance_strips_source_connections_and_ai() {
        let cfg = fixture();
        let slice = slice_for_instance(&cfg, "mcp-prod").expect("slice");
        assert!(slice.source_connections.is_empty());
        assert!(slice.azure.ai.is_none());
        assert!(slice.azure.search.is_some(), "search kept for MCP");
    }

    #[test]
    fn unknown_instance_returns_error() {
        let cfg = fixture();
        let err = slice_for_instance(&cfg, "ghost").unwrap_err();
        assert!(err.to_string().contains("ghost"));
    }
}
