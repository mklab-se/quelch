//! Compute the desired vs. actual Cosmos container set and diff them.
//!
//! Pure logic, no network — exercised by unit tests.

use crate::config::schema::CosmosConfig;

/// A single Cosmos SQL container plus its partition key path.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerSpec {
    /// Container id (also its name in the ARM resource path).
    pub name: String,
    /// Partition key JSON path, e.g. `/id` or `/source_name`.
    pub partition_key: String,
}

/// Compute the desired container set from the master config.
///
/// Includes only containers whose names are populated in [`ContainerLayout`],
/// plus the meta container partitioned by `/source_name`.
pub fn desired_containers(cosmos: &CosmosConfig) -> Vec<ContainerSpec> {
    let layout = &cosmos.containers;
    let mut out = vec![];
    for (name_opt, partition_key) in [
        (layout.jira_issues.as_deref(), "/id"),
        (layout.jira_sprints.as_deref(), "/id"),
        (layout.jira_fix_versions.as_deref(), "/id"),
        (layout.jira_projects.as_deref(), "/id"),
        (layout.confluence_pages.as_deref(), "/id"),
        (layout.confluence_spaces.as_deref(), "/id"),
    ] {
        if let Some(n) = name_opt {
            out.push(ContainerSpec {
                name: n.to_string(),
                partition_key: partition_key.to_string(),
            });
        }
    }
    out.push(ContainerSpec {
        name: cosmos.meta_container.clone(),
        partition_key: "/source_name".to_string(),
    });
    out
}

/// One element of the plan: what should happen for a single container.
#[derive(Debug, Clone, PartialEq)]
pub enum ContainerDiff {
    /// Container is missing — Quelch will PUT it.
    Create(ContainerSpec),
    /// Container already exists with the right partition key.
    Match {
        /// Container name.
        name: String,
    },
    /// Container exists but its partition key differs from the desired one.
    PartitionKeyMismatch {
        /// Container name.
        name: String,
        /// Partition key Quelch wants.
        want: String,
        /// Partition key actually configured on the live container.
        have: String,
    },
}

/// Compare the desired list against the actual list and emit one
/// [`ContainerDiff`] per desired container.
///
/// Existing containers not in `want` are ignored — Quelch never deletes.
pub fn diff_containers(want: &[ContainerSpec], have: &[ContainerSpec]) -> Vec<ContainerDiff> {
    let mut out = vec![];
    for w in want {
        match have.iter().find(|h| h.name == w.name) {
            Some(h) if h.partition_key == w.partition_key => {
                out.push(ContainerDiff::Match {
                    name: w.name.clone(),
                });
            }
            Some(h) => out.push(ContainerDiff::PartitionKeyMismatch {
                name: w.name.clone(),
                want: w.partition_key.clone(),
                have: h.partition_key.clone(),
            }),
            None => out.push(ContainerDiff::Create(w.clone())),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_creates_missing_and_matches_existing() {
        let want = vec![
            ContainerSpec {
                name: "a".into(),
                partition_key: "/id".into(),
            },
            ContainerSpec {
                name: "b".into(),
                partition_key: "/id".into(),
            },
        ];
        let have = vec![ContainerSpec {
            name: "a".into(),
            partition_key: "/id".into(),
        }];
        assert_eq!(
            diff_containers(&want, &have),
            vec![
                ContainerDiff::Match { name: "a".into() },
                ContainerDiff::Create(ContainerSpec {
                    name: "b".into(),
                    partition_key: "/id".into()
                }),
            ]
        );
    }

    #[test]
    fn diff_flags_partition_key_mismatch() {
        let want = vec![ContainerSpec {
            name: "a".into(),
            partition_key: "/id".into(),
        }];
        let have = vec![ContainerSpec {
            name: "a".into(),
            partition_key: "/wrong".into(),
        }];
        assert_eq!(
            diff_containers(&want, &have),
            vec![ContainerDiff::PartitionKeyMismatch {
                name: "a".into(),
                want: "/id".into(),
                have: "/wrong".into(),
            }]
        );
    }
}
