//! Prerequisite validation for `quelch init` and `quelch validate`.
//!
//! Quelch deliberately does not provision the Azure resources it depends on
//! (Cosmos DB, AI Search, the Foundry project / Azure OpenAI account). The
//! user creates them up front; Quelch only configures their internals
//! (Cosmos containers, AI Search indexes/skillsets/KB).
//!
//! This module runs `az`-backed checks against the resources referenced by
//! the parsed [`Config`] and produces a structured [`Report`]. The wizard
//! prints it at exit time; `quelch validate` runs the same check
//! non-interactively.
//!
//! All checks are best-effort: if `az` is not on PATH or the user is not
//! signed in, we emit [`Status::Unknown`] rather than failing — the user
//! can still write the config and re-validate later.

use crate::config::Config;
use crate::config::schema::AiProvider;

use super::discover;

/// Outcome of checking one prerequisite resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The named resource exists and matches its declared type.
    Found,
    /// `az` confirmed the resource is missing in the named resource group.
    Missing,
    /// Could not determine — `az` unavailable, not signed in, or transient
    /// failure. Don't block; surface the uncertainty.
    Unknown,
}

/// One row in the prerequisite report.
#[derive(Debug, Clone)]
pub struct Check {
    /// Human-readable label, e.g. "Cosmos DB account 'quelch-cosmos'".
    pub label: String,
    /// Status outcome.
    pub status: Status,
    /// Optional remediation hint shown when `status == Missing`.
    pub remediation: Option<String>,
}

/// Full prerequisite report.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// All individual checks, in the order they ran.
    pub checks: Vec<Check>,
}

impl Report {
    /// Returns `true` if any check was [`Status::Missing`].
    pub fn has_missing(&self) -> bool {
        self.checks.iter().any(|c| c.status == Status::Missing)
    }

    /// Pretty-print the report to stdout. Suitable for both the wizard's
    /// exit-time summary and `quelch validate`.
    pub fn print(&self) {
        println!("\nPrerequisite check:");
        for c in &self.checks {
            let glyph = match c.status {
                Status::Found => "  [OK]",
                Status::Missing => "  [!!]",
                Status::Unknown => "  [ ?]",
            };
            println!("{glyph} {}", c.label);
            if c.status == Status::Missing
                && let Some(r) = &c.remediation
            {
                for line in r.lines() {
                    println!("       {line}");
                }
            }
        }
        let missing = self
            .checks
            .iter()
            .filter(|c| c.status == Status::Missing)
            .count();
        let unknown = self
            .checks
            .iter()
            .filter(|c| c.status == Status::Unknown)
            .count();
        if missing > 0 || unknown > 0 {
            println!(
                "\n{missing} prerequisite(s) missing, {unknown} could not be checked. \
                 Create them in Azure, then re-run `quelch validate`."
            );
        } else {
            println!("\nAll prerequisites present.");
        }
    }
}

/// Run every prerequisite check applicable to `config` and return the
/// resulting [`Report`]. Best-effort — never fails.
pub async fn check_all(config: &Config) -> Report {
    let mut checks = Vec::new();

    // Cosmos: subscription_id + resource_group + account are all optional in
    // the schema; we only check when the operator filled them in.
    let cosmos = &config.azure.cosmos;
    match (
        cosmos.subscription_id.as_deref(),
        cosmos.resource_group.as_deref(),
    ) {
        (Some(sub), Some(rg)) => {
            checks.push(check_cosmos(sub, rg, cosmos.account.as_deref()).await);

            // AI Search and AI provider live in the same RG by convention.
            if let Some(search) = &config.azure.search {
                checks.push(check_search(sub, rg, &search.endpoint).await);
            }

            if let Some(ai) = &config.azure.ai {
                checks.push(check_ai_provider(sub, rg, ai).await);
            }
        }
        _ => {
            checks.push(Check {
                label: "Cosmos DB account (skipped — subscription_id / resource_group not set)"
                    .to_string(),
                status: Status::Unknown,
                remediation: Some(
                    "Fill in `azure.cosmos.subscription_id` and `azure.cosmos.resource_group` \
                     in quelch.yaml so `quelch validate` can verify the account exists."
                        .to_string(),
                ),
            });
        }
    }

    Report { checks }
}

// ---------------------------------------------------------------------------
// Per-resource checks
// ---------------------------------------------------------------------------

async fn check_cosmos(sub: &str, rg: &str, expected_name: Option<&str>) -> Check {
    let label = match expected_name {
        Some(n) => format!("Cosmos DB account '{n}' in '{rg}'"),
        None => format!("Cosmos DB account in '{rg}'"),
    };
    let remediation = Some(format!(
        "az cosmosdb create -n <name> -g {rg} \
         --kind GlobalDocumentDB --capabilities EnableServerless"
    ));

    let Ok(list) = discover::list_cosmos_accounts(sub, rg).await else {
        return Check {
            label,
            status: Status::Unknown,
            remediation,
        };
    };
    let found = match expected_name {
        Some(n) => list.iter().any(|a| a.name == n),
        None => !list.is_empty(),
    };
    Check {
        label,
        status: if found {
            Status::Found
        } else {
            Status::Missing
        },
        remediation,
    }
}

async fn check_search(sub: &str, rg: &str, endpoint: &str) -> Check {
    let label = format!("Azure AI Search service at {endpoint} (RG '{rg}')");
    let create_hint = format!(
        "az search service create -n <name> -g {rg} --sku basic\n\
         (then enable the semantic ranker in the portal)"
    );

    let Ok(list) = discover::list_search_services(sub, rg).await else {
        return Check {
            label,
            status: Status::Unknown,
            remediation: Some(create_hint),
        };
    };
    // Endpoint shape: https://<name>.search.windows.net — match by name.
    let expected_name = endpoint
        .strip_prefix("https://")
        .and_then(|rest| rest.split('.').next())
        .unwrap_or("");
    let found = !expected_name.is_empty() && list.iter().any(|s| s.name == expected_name);
    if found {
        return Check {
            label,
            status: Status::Found,
            remediation: None,
        };
    }
    // Not found. Tailor the remediation to what the RG actually contains so
    // the user can either pick an existing service or create a new one.
    let remediation = if list.is_empty() {
        Some(format!(
            "No AI Search services found in resource group '{rg}'. Create one:\n{create_hint}"
        ))
    } else {
        let names: Vec<&str> = list.iter().map(|s| s.name.as_str()).collect();
        Some(format!(
            "Resource group '{rg}' contains {} AI Search service(s): {}.\n\
             Update `azure.search.endpoint` in quelch.yaml to one of those, or create a new one:\n{}",
            names.len(),
            names.join(", "),
            create_hint
        ))
    };
    Check {
        label,
        status: Status::Missing,
        remediation,
    }
}

async fn check_ai_provider(sub: &str, rg: &str, ai: &crate::config::schema::AiConfig) -> Check {
    let endpoint = &ai.endpoint;
    match ai.provider {
        AiProvider::AzureOpenai => {
            let label = format!("Azure OpenAI account at {endpoint}");
            let remediation = Some(format!(
                "Create one in the portal or via:\n\
                 az cognitiveservices account create -n <name> -g {rg} \\\n  \
                   --kind OpenAI --sku S0 -l <region>\n\
                 Then deploy the embedding and chat models you need."
            ));
            let Ok(list) = discover::list_openai_accounts(sub, rg).await else {
                return Check {
                    label,
                    status: Status::Unknown,
                    remediation,
                };
            };
            let found = list.iter().any(|a| a.endpoint == *endpoint);
            Check {
                label,
                status: if found {
                    Status::Found
                } else {
                    Status::Missing
                },
                remediation,
            }
        }
        AiProvider::Foundry => {
            let label = format!("Microsoft Foundry project at {endpoint}");
            let remediation = Some(format!(
                "Create a Foundry project in the portal (https://ai.azure.com)\n\
                 inside resource group '{rg}', then deploy the embedding and\n\
                 chat models you need."
            ));
            let Ok(list) = discover::list_foundry_projects(sub, rg).await else {
                return Check {
                    label,
                    status: Status::Unknown,
                    remediation,
                };
            };
            let found = list.iter().any(|p| p.endpoint == *endpoint);
            Check {
                label,
                status: if found {
                    Status::Found
                } else {
                    Status::Missing
                },
                remediation,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_has_missing_returns_true_when_any_check_missing() {
        let r = Report {
            checks: vec![
                Check {
                    label: "a".into(),
                    status: Status::Found,
                    remediation: None,
                },
                Check {
                    label: "b".into(),
                    status: Status::Missing,
                    remediation: None,
                },
            ],
        };
        assert!(r.has_missing());
    }

    #[test]
    fn report_has_missing_returns_false_when_all_found_or_unknown() {
        let r = Report {
            checks: vec![
                Check {
                    label: "a".into(),
                    status: Status::Found,
                    remediation: None,
                },
                Check {
                    label: "b".into(),
                    status: Status::Unknown,
                    remediation: None,
                },
            ],
        };
        assert!(!r.has_missing());
    }

    #[tokio::test]
    async fn check_all_emits_unknown_when_subscription_missing() {
        // Build a minimal config without subscription_id / resource_group set.
        let yaml = r#"
azure:
  cosmos:
    endpoint: https://example.documents.azure.com
    database: quelch
source_connections: []
instances: []
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        let report = check_all(&cfg).await;
        assert_eq!(report.checks.len(), 1);
        assert_eq!(report.checks[0].status, Status::Unknown);
    }
}
