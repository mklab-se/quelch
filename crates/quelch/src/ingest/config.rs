use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct CycleConfig {
    pub deployment_name: String,
    pub meta_container: String,
    pub safety_lag_minutes: u32,
    pub batch_size: usize,
    pub poll_interval: Duration,
    pub reconcile_every: u64,
    pub max_cycle_duration: Duration,
    pub companion_containers: HashMap<String, String>,
}

impl CycleConfig {
    pub fn from_config(config: &crate::config::Config, instance_name: impl Into<String>) -> Self {
        let cc = &config.azure.cosmos.containers;

        let mut companion_containers = HashMap::new();
        companion_containers.insert(
            "sprints".into(),
            cc.jira_sprints
                .clone()
                .unwrap_or_else(|| "jira-sprints".into()),
        );
        companion_containers.insert(
            "fix_versions".into(),
            cc.jira_fix_versions
                .clone()
                .unwrap_or_else(|| "jira-fix-versions".into()),
        );
        companion_containers.insert(
            "projects".into(),
            cc.jira_projects
                .clone()
                .unwrap_or_else(|| "jira-projects".into()),
        );
        companion_containers.insert(
            "spaces".into(),
            cc.confluence_spaces
                .clone()
                .unwrap_or_else(|| "confluence-spaces".into()),
        );

        let instance_name: String = instance_name.into();
        let poll_interval = config
            .instances
            .iter()
            .find(|i| i.name == instance_name)
            .and_then(|i| match &i.spec {
                crate::config::InstanceSpec::Ingest(ig) => Some(ig.cycle_interval),
                _ => None,
            })
            .unwrap_or_else(|| Duration::from_secs(300));

        Self {
            deployment_name: instance_name,
            meta_container: config.azure.cosmos.meta_container.clone(),
            safety_lag_minutes: 2,
            batch_size: 100,
            poll_interval,
            reconcile_every: 12,
            max_cycle_duration: Duration::from_secs(1800),
            companion_containers,
        }
    }
}

impl Default for CycleConfig {
    fn default() -> Self {
        let mut companion_containers = HashMap::new();
        companion_containers.insert("sprints".into(), "jira-sprints".into());
        companion_containers.insert("fix_versions".into(), "jira-fix-versions".into());
        companion_containers.insert("projects".into(), "jira-projects".into());
        companion_containers.insert("spaces".into(), "confluence-spaces".into());

        Self {
            deployment_name: "test".into(),
            meta_container: "quelch-meta".into(),
            safety_lag_minutes: 2,
            batch_size: 100,
            poll_interval: Duration::from_secs(300),
            reconcile_every: 12,
            max_cycle_duration: Duration::from_secs(1800),
            companion_containers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_values() {
        let cfg = CycleConfig::default();
        assert_eq!(cfg.safety_lag_minutes, 2);
        assert_eq!(cfg.batch_size, 100);
        assert_eq!(cfg.reconcile_every, 12);
        assert_eq!(cfg.meta_container, "quelch-meta");
        assert_eq!(cfg.companion_containers["sprints"], "jira-sprints");
        assert_eq!(
            cfg.companion_containers["fix_versions"],
            "jira-fix-versions"
        );
        assert_eq!(cfg.companion_containers["projects"], "jira-projects");
        assert_eq!(cfg.companion_containers["spaces"], "confluence-spaces");
    }

    #[test]
    fn from_config_picks_cycle_interval() {
        let yaml = r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
source_connections: []
instances:
  - name: prod
    kind: ingest
    connections: []
    cycle_interval: 2m
"#;
        let config: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
        let cfg = CycleConfig::from_config(&config, "prod");
        assert_eq!(cfg.deployment_name, "prod");
        assert_eq!(cfg.poll_interval, Duration::from_secs(120));
    }
}
