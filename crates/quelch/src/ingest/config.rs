//! Runtime configuration for the ingest cycle engine.

use std::collections::HashMap;
use std::time::Duration;

/// All configuration values the cycle engine needs at runtime.
///
/// Constructed from [`crate::config::Config`] via [`CycleConfig::from_config`],
/// or built directly in tests.
#[derive(Debug, Clone)]
pub struct CycleConfig {
    /// Deployment name (used as the Cosmos partition key in `quelch-meta`).
    pub deployment_name: String,

    /// Name of the Cosmos container that stores sync cursors (default `"quelch-meta"`).
    pub meta_container: String,

    /// How many minutes behind real-time to lag the incremental window upper bound.
    /// Absorbs Atlassian indexing lag and clock drift. Default `2`.
    pub safety_lag_minutes: u32,

    /// Page size for source-API calls. Default `100`.
    pub batch_size: usize,

    /// How often the worker loop polls for new work.  Not used by `cycle::run`
    /// itself; consumed by the worker loop (Task 3.9).
    pub poll_interval: Duration,

    /// Run reconciliation every N cycles. Default `12`.
    pub reconcile_every: u64,

    /// Warn if a single cycle exceeds this duration. Default `30m`.
    pub max_cycle_duration: Duration,

    /// Companion container names, keyed by category string.
    ///
    /// Recognised keys: `"sprints"`, `"fix_versions"`, `"projects"`, `"spaces"`.
    /// Populated by the worker from `Config`; tests may populate directly.
    pub companion_containers: HashMap<String, String>,
}

impl CycleConfig {
    /// Build a `CycleConfig` from the top-level application config.
    ///
    /// Duration strings (e.g. `"300s"`, `"30m"`) are parsed with
    /// [`humantime::parse_duration`].  Unparseable values fall back to the
    /// documented defaults rather than panicking.
    pub fn from_config(config: &crate::config::Config, deployment_name: impl Into<String>) -> Self {
        let ic = &config.ingest;
        let cc = &config.cosmos.containers;

        let poll_interval =
            humantime::parse_duration(&ic.poll_interval).unwrap_or(Duration::from_secs(300));

        let max_cycle_duration =
            humantime::parse_duration(&ic.max_cycle_duration).unwrap_or(Duration::from_secs(1800));

        let mut companion_containers = HashMap::new();
        companion_containers.insert("sprints".into(), cc.jira_sprints.clone());
        companion_containers.insert("fix_versions".into(), cc.jira_fix_versions.clone());
        companion_containers.insert("projects".into(), cc.jira_projects.clone());
        companion_containers.insert("spaces".into(), cc.confluence_spaces.clone());

        Self {
            deployment_name: deployment_name.into(),
            meta_container: config.cosmos.meta_container.clone(),
            safety_lag_minutes: ic.safety_lag_minutes,
            batch_size: ic.batch_size as usize,
            poll_interval,
            reconcile_every: ic.reconcile_every as u64,
            max_cycle_duration,
            companion_containers,
        }
    }
}

impl Default for CycleConfig {
    /// Sensible defaults for tests that don't need a full `Config`.
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
    fn from_config_parses_durations() {
        let yaml = r#"
azure:
  subscription_id: "sub"
  resource_group: "rg"
  region: "swedencentral"
cosmos:
  database: "quelch"
ai:
  provider: azure_openai
  endpoint: "https://x.openai.azure.com"
  embedding:
    deployment: "te"
    dimensions: 1536
  chat:
    deployment: "gpt-5-mini"
    model_name: "gpt-5-mini"
ingest:
  poll_interval: "120s"
  safety_lag_minutes: 5
  batch_size: 50
  reconcile_every: 6
  max_cycle_duration: "15m"
  max_concurrent_per_source: 1
  max_retries: 3
sources: []
deployments: []
"#;
        let config: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
        let cfg = CycleConfig::from_config(&config, "prod");
        assert_eq!(cfg.deployment_name, "prod");
        assert_eq!(cfg.safety_lag_minutes, 5);
        assert_eq!(cfg.batch_size, 50);
        assert_eq!(cfg.reconcile_every, 6);
        assert_eq!(cfg.poll_interval, Duration::from_secs(120));
        assert_eq!(cfg.max_cycle_duration, Duration::from_secs(900));
    }

    #[test]
    fn from_config_falls_back_on_bad_duration() {
        let yaml = r#"
azure:
  subscription_id: "sub"
  resource_group: "rg"
  region: "swedencentral"
cosmos:
  database: "quelch"
ai:
  provider: azure_openai
  endpoint: "https://x.openai.azure.com"
  embedding:
    deployment: "te"
    dimensions: 1536
  chat:
    deployment: "gpt-5-mini"
    model_name: "gpt-5-mini"
ingest:
  poll_interval: "not-a-duration"
  max_cycle_duration: "also-bad"
sources: []
deployments: []
"#;
        let config: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
        let cfg = CycleConfig::from_config(&config, "dev");
        // Falls back to defaults: 300s and 1800s
        assert_eq!(cfg.poll_interval, Duration::from_secs(300));
        assert_eq!(cfg.max_cycle_duration, Duration::from_secs(1800));
    }
}
