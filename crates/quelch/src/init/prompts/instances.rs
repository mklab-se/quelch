//! Instance section of the `quelch init` wizard.
//!
//! Prompts for one or more `(Q-Ingest | Q-MCP)` instances. At least one
//! instance is required by `quelch validate`, so the loop runs at least once.

use std::collections::BTreeSet;

use crate::config::schema::{
    IngestInstance, InstanceConfig, InstanceSpec, McpInstance, SourceConnection, SourceType,
};

use super::{
    env_var_stem_from_name, parse_duration, prompt_credential_env_var, prompt_unique_name,
};

/// Prompt to add one or more instances (q-ingest / q-mcp). At least one
/// instance is required by `quelch validate`, so the loop runs at least once.
pub async fn prompt_instances(
    connections: &[SourceConnection],
) -> anyhow::Result<Vec<InstanceConfig>> {
    println!("\n=== Instances ===");
    println!(
        "An instance is one process you'll run somewhere. Two kinds:\n  \
         - ingest: pulls from one or more source connections, writes to Cosmos.\n  \
         - mcp: serves the agent-facing MCP API, queries Cosmos + AI Search.\n\
         A typical setup has one of each. Add as many as you need.\n"
    );

    let mut instances: Vec<InstanceConfig> = Vec::new();
    let mut seen_names: BTreeSet<String> = BTreeSet::new();

    loop {
        let must_add = instances.is_empty();
        let prompt = if must_add {
            "Add an instance:"
        } else {
            "Add another instance?"
        };
        let mut options = vec!["Ingest", "MCP"];
        if !must_add {
            options.push("Done (no more instances)");
        }
        let idx = inquire::Select::new(prompt, options.clone())
            .with_starting_cursor(0)
            .raw_prompt()?
            .index;

        // "Done" only present when !must_add, so it sits at index 2.
        if idx == 2 {
            break;
        }
        let inst = if idx == 0 {
            prompt_ingest_instance(&seen_names, connections)?
        } else {
            prompt_mcp_instance(&seen_names, connections)?
        };
        seen_names.insert(inst.name.clone());
        instances.push(inst);
    }

    Ok(instances)
}

fn prompt_ingest_instance(
    seen: &BTreeSet<String>,
    connections: &[SourceConnection],
) -> anyhow::Result<InstanceConfig> {
    println!("\n  --- Ingest instance ---");

    if connections.is_empty() {
        anyhow::bail!(
            "an ingest instance needs at least one source connection — go back \
             and add one first (this should not happen via the wizard)"
        );
    }

    let name = prompt_unique_name(
        seen,
        "  Instance name (used by `quelch ingest --instance ...`):",
        "ingest-main",
    )?;

    let labels: Vec<String> = connections
        .iter()
        .map(|c| format!("{} ({:?}, {})", c.name, c.source_type, c.base_url))
        .collect();
    let chosen_indices =
        inquire::MultiSelect::new("  Connections this instance should ingest from:", labels)
            .with_default(&[0])
            .raw_prompt()?
            .into_iter()
            .map(|s| s.index)
            .collect::<Vec<_>>();

    if chosen_indices.is_empty() {
        anyhow::bail!("an ingest instance must reference at least one connection");
    }

    let chosen_connections: Vec<String> = chosen_indices
        .iter()
        .map(|i| connections[*i].name.clone())
        .collect();

    let interval_str: String = inquire::Text::new("  Cycle interval (e.g. 5m, 30s, 1h):")
        .with_initial_value("5m")
        .prompt()?;
    let cycle_interval = parse_duration(&interval_str)?;

    Ok(InstanceConfig {
        name,
        spec: InstanceSpec::Ingest(IngestInstance {
            connections: chosen_connections,
            cycle_interval,
        }),
    })
}

fn prompt_mcp_instance(
    seen: &BTreeSet<String>,
    connections: &[SourceConnection],
) -> anyhow::Result<InstanceConfig> {
    println!("\n  --- MCP instance ---");

    let name = prompt_unique_name(
        seen,
        "  Instance name (used by `quelch mcp --instance ...`):",
        "mcp-prod",
    )?;

    // Derive available logical data sources from the kinds of connections
    // declared so far. This matches `config::data_sources::resolve`.
    let available = available_data_sources(connections);
    if available.is_empty() {
        println!(
            "  (no source connections declared — the MCP instance needs at least one\n  \
             data source. Add a connection first, then re-run init.)"
        );
        anyhow::bail!("MCP instance needs at least one source connection to expose");
    }

    let chosen_indices = inquire::MultiSelect::new("  Data sources to expose:", available.clone())
        .with_default(&(0..available.len()).collect::<Vec<_>>())
        .raw_prompt()?
        .into_iter()
        .map(|s| s.index)
        .collect::<Vec<_>>();
    if chosen_indices.is_empty() {
        anyhow::bail!("an MCP instance must expose at least one data source");
    }
    let expose: Vec<String> = chosen_indices
        .iter()
        .map(|i| available[*i].clone())
        .collect();

    let env_stem = env_var_stem_from_name(&name);
    let api_key_var =
        prompt_credential_env_var("mcp", &format!("{env_stem}_API_KEY"), "MCP API key")?;
    let api_key = format!("${{{api_key_var}}}");

    let listen: String = inquire::Text::new("  Listen address:")
        .with_initial_value("0.0.0.0:8080")
        .prompt()?;

    let knowledge_base: String = inquire::Text::new("  Knowledge Base name:")
        .with_initial_value("quelch-prod-kb")
        .with_help_message(
            "Used by the MCP `search` tool; created in Azure AI Search by `quelch azure apply`.",
        )
        .prompt()?;

    Ok(InstanceConfig {
        name,
        spec: InstanceSpec::Mcp(McpInstance {
            expose,
            api_key,
            knowledge_base,
            listen,
        }),
    })
}

/// Logical data sources implied by the kinds of source connections present.
/// Mirrors [`crate::config::data_sources::resolve`] but produces just the
/// public names so the MCP `expose:` MultiSelect can offer them.
fn available_data_sources(connections: &[SourceConnection]) -> Vec<String> {
    let mut has_jira = false;
    let mut has_confluence = false;
    for c in connections {
        match c.source_type {
            SourceType::Jira => has_jira = true,
            SourceType::Confluence => has_confluence = true,
        }
    }
    let mut out = Vec::new();
    if has_jira {
        out.extend([
            "jira_issues".to_string(),
            "jira_sprints".to_string(),
            "jira_fix_versions".to_string(),
            "jira_projects".to_string(),
        ]);
    }
    if has_confluence {
        out.extend([
            "confluence_pages".to_string(),
            "confluence_spaces".to_string(),
        ]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::SourceAuth;

    #[test]
    fn available_data_sources_picks_jira_only_when_only_jira_present() {
        let conns = vec![SourceConnection {
            name: "j".to_string(),
            source_type: SourceType::Jira,
            base_url: "https://j".to_string(),
            auth: SourceAuth::Pat {
                token: "T".to_string(),
            },
            projects: vec!["X".to_string()],
            spaces: vec![],
        }];
        let avail = available_data_sources(&conns);
        assert!(avail.contains(&"jira_issues".to_string()));
        assert!(avail.contains(&"jira_projects".to_string()));
        assert!(!avail.iter().any(|d| d.starts_with("confluence")));
    }

    #[test]
    fn available_data_sources_returns_empty_when_no_connections() {
        let avail = available_data_sources(&[]);
        assert!(avail.is_empty());
    }
}
