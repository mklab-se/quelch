//! Prerequisite validation for `quelch init` and `quelch validate`.
//!
//! Quelch deliberately does not provision the Azure resources it depends on
//! (Cosmos DB, AI Search, the Foundry project / Azure OpenAI account, the
//! Container Apps environment, App Insights, Key Vault). The user creates
//! them up front; Quelch only configures internals (Cosmos containers, AI
//! Search indexes/skillsets/KS/KB, Container App revisions) and writes
//! secrets into the Key Vault.
//!
//! This module runs an `az`-backed check for every required resource and
//! produces a structured report. The wizard prints it at exit time; the
//! `quelch validate` command runs the same check non-interactively.
//!
//! All checks are best-effort: if `az` is not on PATH or the user is not
//! signed in, we emit `Status::Unknown` rather than failing — the user can
//! still write the config and validate later.

use crate::config::{AiProvider, Config, DeploymentRole, DeploymentTarget};

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
    pub status: Status,
    /// Optional remediation hint shown when `status == Missing`.
    pub hint: Option<String>,
}

/// Full prerequisite report.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    /// Returns `true` if any check was [`Status::Missing`].
    pub fn has_missing(&self) -> bool {
        self.checks.iter().any(|c| c.status == Status::Missing)
    }

    /// Pretty-print the report to stdout in a format suitable for
    /// `quelch init` exit-time and `quelch validate`.
    pub fn print(&self) {
        println!("\nPrerequisite check:");
        for c in &self.checks {
            let glyph = match c.status {
                Status::Found => "  ✓",
                Status::Missing => "  ✗",
                Status::Unknown => "  ?",
            };
            println!("{glyph} {}", c.label);
            if c.status == Status::Missing
                && let Some(hint) = &c.hint
            {
                for line in hint.lines() {
                    println!("     {line}");
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
                 See https://github.com/mklab-se/quelch/blob/main/docs/getting-started.md \
                 for the full prerequisites list."
            );
        } else {
            println!("\nAll prerequisites present.");
        }
    }
}

/// Run every prerequisite check that applies to `config` and return the
/// resulting [`Report`].
///
/// Skips Container Apps / App Insights checks when no `target: azure`
/// deployment exists in the config (e.g. an on-prem-only setup).
pub async fn check_all(config: &Config) -> Report {
    let mut checks = Vec::new();

    let sub = &config.azure.subscription_id;

    // Cosmos DB account (always required — the system of record). May live in
    // a different RG than the rest of the deployment if `cosmos.account_resource_group`
    // is set.
    checks.push(
        check_cosmos(
            sub,
            config.cosmos_resource_group(),
            config.cosmos.account.as_deref(),
        )
        .await,
    );

    // Azure AI Search service (always required — exposes Q-MCP's search tool).
    checks.push(
        check_search(
            sub,
            config.search_resource_group(),
            config.search.service.as_deref(),
        )
        .await,
    );

    // AI provider (Foundry or Azure OpenAI) — must exist regardless of role.
    // Often lives in a shared / centrally-managed RG.
    checks.push(check_ai_provider(sub, config.ai_resource_group(), config).await);

    // Cloud-only prerequisites: ACA env, App Insights, Key Vault.
    let has_azure_deployment = config
        .deployments
        .iter()
        .any(|d| matches!(d.target, DeploymentTarget::Azure));

    if has_azure_deployment {
        checks
            .push(check_container_apps_env(sub, config.container_apps_env_resource_group()).await);
        checks.push(
            check_application_insights(sub, config.application_insights_resource_group()).await,
        );
        checks.push(check_key_vault(sub, config.key_vault_resource_group()).await);
    }

    // For MCP deployments specifically, note the Container App will be
    // created by `quelch azure deploy` — we don't precheck it.
    if config.deployments.iter().any(|d| {
        matches!(d.role, DeploymentRole::Mcp) && matches!(d.target, DeploymentTarget::Azure)
    }) {
        checks.push(Check {
            label: "Container App for MCP — will be created by `quelch azure deploy`".to_string(),
            status: Status::Found,
            hint: None,
        });
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
    let hint = Some(format!(
        "az cosmosdb create -n <name> -g {rg} \
         --kind GlobalDocumentDB --capabilities EnableServerless"
    ));

    let Ok(list) = discover::list_cosmos_accounts(sub, rg).await else {
        return Check {
            label,
            status: Status::Unknown,
            hint,
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
        hint,
    }
}

async fn check_search(sub: &str, rg: &str, expected_name: Option<&str>) -> Check {
    let label = match expected_name {
        Some(n) => format!("Azure AI Search service '{n}' in '{rg}'"),
        None => format!("Azure AI Search service in '{rg}'"),
    };
    let hint = Some(format!(
        "az search service create -n <name> -g {rg} --sku basic"
    ));

    let Ok(list) = discover::list_search_services(sub, rg).await else {
        return Check {
            label,
            status: Status::Unknown,
            hint,
        };
    };
    let found = match expected_name {
        Some(n) => list.iter().any(|s| s.name == n),
        None => !list.is_empty(),
    };
    Check {
        label,
        status: if found {
            Status::Found
        } else {
            Status::Missing
        },
        hint,
    }
}

async fn check_ai_provider(sub: &str, rg: &str, config: &Config) -> Check {
    let endpoint = &config.ai.endpoint;
    match config.ai.provider {
        AiProvider::AzureOpenai => {
            let label = format!("Azure OpenAI account at {endpoint}");
            let hint = Some(format!(
                "Create one in the portal or via:\n\
                 az cognitiveservices account create -n <name> -g {rg} \\\n  \
                   --kind OpenAI --sku S0 -l <region>\n\
                 Then deploy the embedding and chat models you need."
            ));
            let Ok(list) = discover::list_openai_accounts(sub, rg).await else {
                return Check {
                    label,
                    status: Status::Unknown,
                    hint,
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
                hint,
            }
        }
        AiProvider::Foundry => {
            let label = format!("Microsoft Foundry project at {endpoint}");
            let hint = Some(format!(
                "Create a Foundry project in the portal (https://ai.azure.com)\n\
                 inside resource group '{rg}', then deploy the embedding and\n\
                 chat models you need."
            ));
            let Ok(list) = discover::list_foundry_projects(sub, rg).await else {
                return Check {
                    label,
                    status: Status::Unknown,
                    hint,
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
                hint,
            }
        }
    }
}

async fn check_container_apps_env(sub: &str, rg: &str) -> Check {
    let label = format!("Container Apps environment in '{rg}'");
    let hint = Some(format!(
        "az containerapp env create -n <name> -g {rg} -l <region>"
    ));
    let Ok(list) = discover::list_container_apps_environments(sub, rg).await else {
        return Check {
            label,
            status: Status::Unknown,
            hint,
        };
    };
    Check {
        label,
        status: if list.is_empty() {
            Status::Missing
        } else {
            Status::Found
        },
        hint,
    }
}

async fn check_application_insights(sub: &str, rg: &str) -> Check {
    let label = format!("Application Insights component in '{rg}'");
    let hint = Some(format!(
        "az monitor app-insights component create --app <name> -g {rg} -l <region>"
    ));
    let Ok(list) = discover::list_application_insights(sub, rg).await else {
        return Check {
            label,
            status: Status::Unknown,
            hint,
        };
    };
    Check {
        label,
        status: if list.is_empty() {
            Status::Missing
        } else {
            Status::Found
        },
        hint,
    }
}

async fn check_key_vault(sub: &str, rg: &str) -> Check {
    let label = format!("Key Vault in '{rg}'");
    let hint = Some(format!(
        "az keyvault create -n <globally-unique-name> -g {rg} -l <region>"
    ));
    let Ok(list) = discover::list_key_vaults(sub, rg).await else {
        return Check {
            label,
            status: Status::Unknown,
            hint,
        };
    };
    Check {
        label,
        status: if list.is_empty() {
            Status::Missing
        } else {
            Status::Found
        },
        hint,
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
                    hint: None,
                },
                Check {
                    label: "b".into(),
                    status: Status::Missing,
                    hint: None,
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
                    hint: None,
                },
                Check {
                    label: "b".into(),
                    status: Status::Unknown,
                    hint: None,
                },
            ],
        };
        assert!(!r.has_missing());
    }
}
