use std::collections::BTreeMap;

use crate::config::schema::{Config, InstanceSpec, SourceConnection, SourceType};

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("conflicting subsource claims:\n{0}")]
    Conflicts(String),
    #[error("instance '{instance}' references unknown connection '{connection}'")]
    UnknownConnection {
        instance: String,
        connection: String,
    },
}

pub fn validate(cfg: &Config) -> Result<(), ValidationError> {
    validate_connection_refs(cfg)?;
    validate_no_overlapping_claims(cfg)?;
    Ok(())
}

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

type ClaimKey = (SourceType, String, String);

fn claims_for_connection(c: &SourceConnection) -> impl Iterator<Item = ClaimKey> + '_ {
    let subsources: Vec<&String> = match c.source_type {
        SourceType::Jira => c.projects.iter().collect(),
        SourceType::Confluence => c.spaces.iter().collect(),
    };
    subsources
        .into_iter()
        .map(move |s| (c.source_type, c.base_url.clone(), s.clone()))
}

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
