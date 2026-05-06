//! Cross-field validation of the master [`Config`].
//!
//! Catches the two failure modes serde's per-field deserialisation cannot:
//!
//! - An ingest instance referencing an undeclared `source_connections[]`
//!   entry.
//! - Two ingest instances claiming the same `(source_type, base_url,
//!   subsource)` triple — the source-of-truth conflict that would let a
//!   second worker steal a cursor at runtime.
//!
//! Run via [`validate`]. The CLI calls this from `quelch validate`.

use std::collections::BTreeMap;

use crate::config::schema::{Config, InstanceSpec, SourceConnection, SourceType};

/// Errors returned by [`validate`].
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    /// One or more `(source_type, base_url, subsource)` triples are claimed
    /// by more than one ingest instance.
    #[error("conflicting subsource claims:\n{0}")]
    Conflicts(String),
    /// An ingest instance references a connection that is not declared in
    /// `source_connections[]`.
    #[error("instance '{instance}' references unknown connection '{connection}'")]
    UnknownConnection {
        /// Name of the offending instance.
        instance: String,
        /// The undefined connection name it tried to reference.
        connection: String,
    },
}

/// Run all cross-field validations against `cfg` and report the first
/// failure.
///
/// # Errors
/// See [`ValidationError`].
pub fn validate(cfg: &Config) -> Result<(), ValidationError> {
    validate_connection_refs(cfg)?;
    validate_no_overlapping_claims(cfg)?;
    Ok(())
}

/// Verify every `instances[].connections[]` name resolves to a declared
/// `source_connections[]` entry.
fn validate_connection_refs(cfg: &Config) -> Result<(), ValidationError> {
    let names: std::collections::HashSet<_> =
        cfg.source_connections.iter().map(|c| &c.name).collect();
    for inst in &cfg.instances {
        if let InstanceSpec::Ingest(spec) = &inst.spec {
            for c in &spec.connections {
                if !names.contains(c) {
                    return Err(ValidationError::UnknownConnection {
                        instance: inst.name.clone(),
                        connection: c.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

/// `(source_type, base_url, subsource)` — uniquely identifies a logical
/// thing being claimed by an ingest instance.
type ClaimKey = (SourceType, String, String);

/// Walk one connection and yield one [`ClaimKey`] per subsource (project for
/// Jira, space for Confluence).
fn claims_for_connection(c: &SourceConnection) -> impl Iterator<Item = ClaimKey> + '_ {
    let subsources: Vec<&String> = match c.source_type {
        SourceType::Jira => c.projects.iter().collect(),
        SourceType::Confluence => c.spaces.iter().collect(),
    };
    subsources
        .into_iter()
        .map(move |s| (c.source_type, c.base_url.clone(), s.clone()))
}

/// Check that no two ingest instances claim the same subsource.
fn validate_no_overlapping_claims(cfg: &Config) -> Result<(), ValidationError> {
    let conn_by_name: BTreeMap<&str, &SourceConnection> = cfg
        .source_connections
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect();

    let mut claimed: BTreeMap<ClaimKey, Vec<&str>> = BTreeMap::new();

    for inst in &cfg.instances {
        let InstanceSpec::Ingest(spec) = &inst.spec else {
            continue;
        };
        for conn_name in &spec.connections {
            let Some(conn) = conn_by_name.get(conn_name.as_str()) else {
                continue;
            };
            for key in claims_for_connection(conn) {
                claimed.entry(key).or_default().push(inst.name.as_str());
            }
        }
    }

    let mut conflicts = String::new();
    for ((kind, base, sub), claimers) in &claimed {
        if claimers.len() > 1 {
            use std::fmt::Write;
            let _ = writeln!(
                conflicts,
                "  - ({:?}, {}, {}) claimed by {}",
                kind,
                base,
                sub,
                claimers.join(", ")
            );
        }
    }
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(ValidationError::Conflicts(conflicts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn rejects_overlapping_subsource_claims_across_ingest_instances() {
        let yaml = r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
source_connections:
  - { name: jira-a, type: jira, base_url: https://jira.example/,
      auth: { kind: pat, token: T },
      projects: [DO, ANNA] }
  - { name: jira-b, type: jira, base_url: https://jira.example/,
      auth: { kind: pat, token: T2 },
      projects: [ANNA, SARA] }
instances:
  - { name: ingest-1, kind: ingest, connections: [jira-a], cycle_interval: 5m }
  - { name: ingest-2, kind: ingest, connections: [jira-b], cycle_interval: 5m }
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        let errs = validate(&cfg).unwrap_err();
        let msg = errs.to_string();
        assert!(msg.contains("ingest-1"), "names ingest-1: {}", msg);
        assert!(msg.contains("ingest-2"), "names ingest-2: {}", msg);
        assert!(
            msg.contains("ANNA"),
            "names the conflicting subsource ANNA: {}",
            msg
        );
        assert!(msg.contains("https://jira.example/"));
    }

    #[test]
    fn accepts_disjoint_ingest_instances() {
        let yaml = r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
source_connections:
  - { name: jira-a, type: jira, base_url: https://jira.example/,
      auth: { kind: pat, token: T }, projects: [DO] }
  - { name: jira-b, type: jira, base_url: https://jira.example/,
      auth: { kind: pat, token: T2 }, projects: [SARA] }
instances:
  - { name: a, kind: ingest, connections: [jira-a], cycle_interval: 5m }
  - { name: b, kind: ingest, connections: [jira-b], cycle_interval: 5m }
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        validate(&cfg).expect("disjoint claims must validate");
    }
}
