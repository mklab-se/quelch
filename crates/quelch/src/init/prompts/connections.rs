//! Source-connection section of the `quelch init` wizard.
//!
//! Loops adding `Jira`/`Confluence` connections until the user picks "Done".
//! For each connection: name, base URL, hosting flavour (Cloud → email +
//! token; Data Center → PAT), subsource list (projects / spaces).

use std::collections::BTreeSet;

use crate::config::schema::{SourceAuth, SourceConnection, SourceType};

use super::{
    env_var_stem_from_name, git_user_email, parse_csv, prompt_credential_env_var,
    prompt_unique_name,
};

/// Prompt to add one or more source connections (Jira / Confluence). Loops
/// until the user picks "Done".
pub async fn prompt_source_connections() -> anyhow::Result<Vec<SourceConnection>> {
    println!("\n=== Source connections ===");
    println!(
        "A source connection is one (base URL × credential) tuple. Each instance\n\
         (q-ingest, q-mcp) references connections by name. You can add as many as\n\
         you like — typical setups have one Jira connection per PAT, plus a\n\
         Confluence connection.\n"
    );

    let mut connections: Vec<SourceConnection> = Vec::new();
    let mut seen_names: BTreeSet<String> = BTreeSet::new();

    loop {
        let prompt = if connections.is_empty() {
            "Add a source connection?"
        } else {
            "Add another source connection?"
        };
        let idx = inquire::Select::new(
            prompt,
            vec!["Jira", "Confluence", "Done (no more connections)"],
        )
        .with_starting_cursor(if connections.is_empty() { 0 } else { 2 })
        .raw_prompt()?
        .index;

        match idx {
            0 => connections.push(prompt_jira_connection(&seen_names)?),
            1 => connections.push(prompt_confluence_connection(&seen_names)?),
            _ => break,
        }

        if let Some(last) = connections.last() {
            seen_names.insert(last.name.clone());
        }
    }

    Ok(connections)
}

fn prompt_jira_connection(seen: &BTreeSet<String>) -> anyhow::Result<SourceConnection> {
    println!("\n  --- Jira connection ---");
    let name = prompt_unique_name(
        seen,
        "  Connection name (used in `instances:` references):",
        "jira-cloud",
    )?;

    let base_url: String = inquire::Text::new("  Base URL (e.g. https://your-org.atlassian.net):")
        .with_initial_value("https://your-org.atlassian.net")
        .prompt()?;

    let is_cloud = prompt_hosting_kind("Jira")?;

    let projects_str: String =
        inquire::Text::new("  Project keys to ingest (comma-separated, e.g. PROJ,ENG):")
            .prompt()?;
    let projects: Vec<String> = parse_csv(&projects_str);

    let auth = prompt_source_auth(is_cloud, "jira", &name)?;

    Ok(SourceConnection {
        name,
        source_type: SourceType::Jira,
        base_url,
        auth,
        projects,
        spaces: vec![],
    })
}

fn prompt_confluence_connection(seen: &BTreeSet<String>) -> anyhow::Result<SourceConnection> {
    println!("\n  --- Confluence connection ---");
    let name = prompt_unique_name(
        seen,
        "  Connection name (used in `instances:` references):",
        "confluence-cloud",
    )?;

    let base_url: String =
        inquire::Text::new("  Base URL (e.g. https://your-org.atlassian.net/wiki):")
            .with_initial_value("https://your-org.atlassian.net/wiki")
            .prompt()?;

    let is_cloud = prompt_hosting_kind("Confluence")?;

    let spaces_str: String =
        inquire::Text::new("  Space keys to ingest (comma-separated, e.g. ENG,DOCS):").prompt()?;
    let spaces: Vec<String> = parse_csv(&spaces_str);

    let auth = prompt_source_auth(is_cloud, "confluence", &name)?;

    Ok(SourceConnection {
        name,
        source_type: SourceType::Confluence,
        base_url,
        auth,
        projects: vec![],
        spaces,
    })
}

/// Atlassian Cloud (email + token) vs Data Center / Server (PAT only).
fn prompt_hosting_kind(product: &str) -> anyhow::Result<bool> {
    let idx = inquire::Select::new(
        &format!("  Where is your {product} hosted?"),
        vec![
            "Atlassian Cloud (*.atlassian.net)",
            "Data Center / Server (self-hosted)",
        ],
    )
    .with_starting_cursor(0)
    .raw_prompt()?
    .index;
    Ok(idx == 0)
}

fn prompt_source_auth(
    is_cloud: bool,
    product_hint: &str,
    connection_name: &str,
) -> anyhow::Result<SourceAuth> {
    let env_stem = env_var_stem_from_name(connection_name);
    if is_cloud {
        println!(
            "\n  Atlassian Cloud uses email + API token. Create the token at\n\
             https://id.atlassian.com/manage-profile/security/api-tokens — the\n\
             account that owns it must have read access to your projects/spaces."
        );
        let email_default = git_user_email().unwrap_or_default();
        let mut email_prompt = inquire::Text::new("  Atlassian account email:");
        if !email_default.is_empty() {
            email_prompt = email_prompt.with_initial_value(&email_default);
        }
        let email: String = email_prompt.prompt()?;

        let var = prompt_credential_env_var(
            product_hint,
            &format!("{env_stem}_API_TOKEN"),
            "Atlassian Cloud API token",
        )?;
        Ok(SourceAuth::Basic {
            email,
            token: format!("${{{var}}}"),
        })
    } else {
        println!(
            "\n  Data Center / Server uses a Personal Access Token (PAT). Generate one\n\
             from your profile → Personal Access Tokens with read access to the\n\
             projects/spaces above."
        );
        let var = prompt_credential_env_var(
            product_hint,
            &format!("{env_stem}_PAT"),
            "Personal Access Token",
        )?;
        Ok(SourceAuth::Pat {
            token: format!("${{{var}}}"),
        })
    }
}
