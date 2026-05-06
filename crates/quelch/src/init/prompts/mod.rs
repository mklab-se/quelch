//! Interactive prompt sections for `quelch init`.
//!
//! Each `prompt_*` function drives one section of the wizard and returns a
//! piece of the final [`crate::config::schema::Config`]. The wizard is
//! loosely tree-structured: Azure → source connections → instances. Section
//! helpers live in their own submodules.
//!
//! ## Credentials
//!
//! Wizard-collected secrets (PATs, API tokens) are NEVER written to disk.
//! The wizard always stores `${ENV_VAR_NAME}` placeholders in the generated
//! YAML; [`crate::config::env::substitute_env_vars`] resolves them at load
//! time. See [`prompt_credential_env_var`].

mod azure;
mod connections;
mod instances;

pub use azure::prompt_azure;
pub use connections::prompt_source_connections;
pub use instances::prompt_instances;

use std::collections::BTreeSet;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Shared helpers (used by multiple submodules and re-exported as `pub(super)`)
// ---------------------------------------------------------------------------

/// Prompt for a name that must not collide with anything already in `seen`.
/// Loops until a unique non-empty name is entered.
pub(super) fn prompt_unique_name(
    seen: &BTreeSet<String>,
    prompt: &str,
    initial: &str,
) -> anyhow::Result<String> {
    loop {
        let name: String = inquire::Text::new(prompt)
            .with_initial_value(initial)
            .prompt()?;
        if name.trim().is_empty() {
            println!("  Name cannot be empty.");
            continue;
        }
        if seen.contains(&name) {
            println!(
                "  Name '{name}' is already used by another connection. Pick a different name."
            );
            continue;
        }
        return Ok(name);
    }
}

/// Split a comma-separated list, trim each entry, drop empties.
pub(super) fn parse_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Convert a name into a sensible env-var name suffix (uppercase, non-alnum
/// → `_`). E.g. `jira-cloud` → `JIRA_CLOUD`.
pub(super) fn env_var_stem_from_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Parse a humantime duration (e.g. `5m`, `30s`, `1h`).
pub(super) fn parse_duration(s: &str) -> anyhow::Result<Duration> {
    humantime::parse_duration(s.trim()).map_err(|e| anyhow::anyhow!("invalid duration '{s}': {e}"))
}

/// Best-effort: read `git config user.email` to suggest a default for the
/// Atlassian Cloud email field.
pub(super) fn git_user_email() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["config", "--get", "user.email"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let email = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if email.is_empty() { None } else { Some(email) }
}

/// Scan the current process env for variable names that look like they hold
/// a credential for the given product. Reads names only — never values.
pub(super) fn find_token_env_vars(product_hint: &str) -> Vec<String> {
    let names = std::env::vars_os().filter_map(|(k, _)| k.into_string().ok());
    match_token_env_var_names(product_hint, names)
}

/// Pure-function core of [`find_token_env_vars`] — separated for testing.
fn match_token_env_var_names<I>(product_hint: &str, names: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let hint = product_hint.to_ascii_uppercase();
    let mut hits: BTreeSet<String> = BTreeSet::new();
    for name in names {
        let upper = name.to_ascii_uppercase();
        if !upper.contains(&hint) {
            continue;
        }
        if upper.contains("PAT")
            || upper.contains("TOKEN")
            || upper.contains("API_KEY")
            || upper.contains("APIKEY")
        {
            hits.insert(name);
        }
    }
    hits.into_iter().collect()
}

/// Prompt the user to pick (or name) the env var that will hold a credential.
/// Returns the env-var name only — never the value.
pub(super) fn prompt_credential_env_var(
    product_hint: &str,
    default_name: &str,
    scope_text: &str,
) -> anyhow::Result<String> {
    let candidates = find_token_env_vars(product_hint);
    const ENTER_NAME: &str = "Use a different env var (enter name)…";

    if candidates.is_empty() {
        println!(
            "  No {product_hint}-related env vars found in your shell. Quelch will\n  \
             store a `${{<NAME>}}` placeholder in quelch.yaml — set the env var\n  \
             before running `quelch …`, both locally and on whatever runs Q-Ingest."
        );
        let name: String =
            inquire::Text::new(&format!("  Env var name that will hold the {scope_text}:"))
                .with_initial_value(default_name)
                .prompt()?;
        return Ok(name);
    }

    println!(
        "  Found {} env var(s) that look like a {product_hint} credential.\n  \
         (Quelch only reads the NAME — the value is not displayed or written\n  \
         to quelch.yaml; only `${{<NAME>}}` is.)",
        candidates.len()
    );

    let mut labels: Vec<String> = candidates
        .iter()
        .map(|name| {
            let set = std::env::var(name).map(|v| !v.is_empty()).unwrap_or(false);
            let marker = if set { "(set)" } else { "(empty!)" };
            format!("{name}  {marker}")
        })
        .collect();
    let enter_name_idx = labels.len();
    labels.push(ENTER_NAME.to_string());

    let idx = inquire::Select::new(&format!("  Which env var holds the {scope_text}?"), labels)
        .with_starting_cursor(0)
        .raw_prompt()?
        .index;

    if idx < candidates.len() {
        Ok(candidates[idx].clone())
    } else {
        debug_assert_eq!(idx, enter_name_idx);
        let name: String =
            inquire::Text::new(&format!("  Env var name that will hold the {scope_text}:"))
                .with_initial_value(default_name)
                .prompt()?;
        Ok(name)
    }
}

// ---------------------------------------------------------------------------
// Env-var summary
// ---------------------------------------------------------------------------

/// Scan a YAML string for `${VAR_NAME}` references and return the unique set
/// of variable names. Hand-rolled (cheaper than pulling in `regex`) and
/// matches what `shellexpand` resolves at config-load time.
pub fn collect_env_var_refs(yaml: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes = yaml.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'$' && bytes[i + 1] == b'{' {
            let start = i + 2;
            if let Some(end_offset) = bytes[start..].iter().position(|&b| b == b'}') {
                let name = &yaml[start..start + end_offset];
                let valid = !name.is_empty()
                    && name
                        .bytes()
                        .next()
                        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
                if valid {
                    out.insert(name.to_string());
                }
                i = start + end_offset + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Tests for the shared helpers (per-section tests live next to their section)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_token_env_var_names_finds_jira_pat_variants() {
        let names = vec![
            "PATH".to_string(),
            "JIRA_PAT".to_string(),
            "JIRA_CLOUD_PAT".to_string(),
            "CONFLUENCE_PAT".to_string(),
            "JIRA_API_TOKEN".to_string(),
        ];
        let hits = match_token_env_var_names("jira", names);
        assert_eq!(
            hits,
            vec![
                "JIRA_API_TOKEN".to_string(),
                "JIRA_CLOUD_PAT".to_string(),
                "JIRA_PAT".to_string(),
            ]
        );
    }

    #[test]
    fn match_token_env_var_names_excludes_path_substring_only_match() {
        let names = vec!["PATH".to_string(), "EDITOR".to_string()];
        let hits = match_token_env_var_names("jira", names);
        assert!(hits.is_empty(), "PATH must not match jira-pat search");
    }

    #[test]
    fn env_var_stem_uppercases_and_replaces_punctuation() {
        assert_eq!(env_var_stem_from_name("jira-cloud"), "JIRA_CLOUD");
        assert_eq!(env_var_stem_from_name("confluence.dc"), "CONFLUENCE_DC");
        assert_eq!(env_var_stem_from_name("MyJira"), "MYJIRA");
    }

    #[test]
    fn collect_env_var_refs_finds_unique_placeholders() {
        let yaml =
            "auth:\n  pat: ${JIRA_PAT}\n  again: ${JIRA_PAT}\n  api: ${CONFLUENCE_API_TOKEN}\n";
        let refs = collect_env_var_refs(yaml);
        assert_eq!(refs.len(), 2);
        assert!(refs.contains("JIRA_PAT"));
        assert!(refs.contains("CONFLUENCE_API_TOKEN"));
    }

    #[test]
    fn collect_env_var_refs_ignores_malformed() {
        let yaml = "${UNTERMINATED\nfoo: ${1BADNAME}\nbar: ${}\nok: ${GOOD}\n";
        let refs = collect_env_var_refs(yaml);
        assert_eq!(refs.iter().collect::<Vec<_>>(), vec!["GOOD"]);
    }

    #[test]
    fn parse_duration_accepts_common_formats() {
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn parse_duration_rejects_garbage() {
        assert!(parse_duration("five minutes").is_err());
    }

    #[test]
    fn parse_csv_trims_and_filters_empty() {
        assert_eq!(
            parse_csv("PROJ, ENG ,, , ANNA"),
            vec!["PROJ".to_string(), "ENG".to_string(), "ANNA".to_string()]
        );
        assert!(parse_csv("").is_empty());
    }
}
