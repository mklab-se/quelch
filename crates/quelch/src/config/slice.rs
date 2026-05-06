use crate::config::schema::{Config, InstanceKind, InstanceSpec};

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
