//! Interactive wizard and non-interactive scaffolding for `quelch init`.
//!
//! The wizard collects answers through a sequence of section helpers in
//! [`prompts`], assembles a [`crate::config::schema::Config`], runs a
//! best-effort prerequisite check ([`prereq::check_all`]), and writes a
//! YAML file the user can pass to subsequent commands.
//!
//! Non-interactive mode picks one of the built-in [`templates`] and writes
//! it directly.
//!
//! Entry point: [`run`].

pub mod discover;
pub mod prereq;
pub mod prompts;
pub mod templates;

use std::path::Path;

use crate::config::schema::Config;

/// Options controlling the `init` command.
#[derive(Debug, Default)]
pub struct InitOptions {
    /// Skip all prompts and write a built-in template directly.
    pub non_interactive: bool,
    /// Template name to use in non-interactive mode (default: "minimal").
    pub from_template: Option<String>,
    /// Overwrite an existing `quelch.yaml` without asking.
    pub force: bool,
}

/// Run `quelch init`, writing a `quelch.yaml` to `output_path`.
///
/// In non-interactive mode the named template (or `minimal` if none) is
/// written directly. In interactive mode a sequence of prompts is run, the
/// resulting [`Config`] is serialised to YAML, and a prerequisite check is
/// printed.
///
/// If the user cancels with Ctrl-C / Esc, the function prints a friendly
/// "Wizard cancelled" line and returns `Ok(())` without writing anything.
///
/// # Errors
/// - `output_path` already exists and `force` is not set.
/// - The named template does not exist.
/// - I/O failures writing the YAML file.
pub async fn run(output_path: &Path, options: InitOptions) -> anyhow::Result<()> {
    if output_path.exists() && !options.force {
        anyhow::bail!(
            "{} already exists. Use --force to overwrite.",
            output_path.display()
        );
    }

    if options.non_interactive {
        let name = options.from_template.as_deref().unwrap_or("minimal");
        let cfg = templates::template_for(name)?;
        write_yaml(&cfg, output_path)?;
        println!("Wrote {} (template: {name})", output_path.display());
        return Ok(());
    }

    let config = match run_interactive().await {
        Ok(c) => c,
        Err(e) => {
            // Translate inquire's "Esc / Ctrl-C" errors into a friendlier
            // exit. These are normal user actions, not failures.
            if let Some(inq) = e.downcast_ref::<inquire::InquireError>()
                && matches!(
                    inq,
                    inquire::InquireError::OperationCanceled
                        | inquire::InquireError::OperationInterrupted
                )
            {
                println!("\nWizard cancelled — no quelch.yaml was written.");
                return Ok(());
            }
            return Err(e);
        }
    };
    write_yaml(&config, output_path)?;
    println!("\nWrote {}", output_path.display());

    print_env_var_summary(&config)?;
    offer_git_setup(output_path)?;

    println!(
        "\nNext: run `quelch validate --config {}` to verify the config and prerequisites.",
        output_path.display()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Interactive flow
// ---------------------------------------------------------------------------

async fn run_interactive() -> anyhow::Result<Config> {
    println!("Welcome to quelch init.");
    println!("This wizard will create a quelch.yaml for your environment.\n");
    println!(
        "Quelch references existing Azure resources but does NOT provision them.\n\
         Before running this wizard you should already have:\n  \
         - a Cosmos DB account\n  \
         - an Azure AI Search service (Basic+ with the semantic ranker enabled)\n  \
         - a Foundry project or Azure OpenAI account with an embedding + chat\n  \
         \x20 deployment\n\
         If you don't, you can still complete the wizard — `quelch validate` will\n\
         flag what's missing.\n"
    );

    let azure = prompts::prompt_azure().await?;
    let connections = prompts::prompt_source_connections().await?;
    if connections.is_empty() {
        anyhow::bail!(
            "no source connections were configured — at least one is required \
             for an ingest or MCP instance to do anything useful"
        );
    }
    let instances = prompts::prompt_instances(&connections).await?;

    let config = Config {
        azure,
        source_connections: connections,
        instances,
    };

    let report = prereq::check_all(&config).await;
    report.print();
    if report.has_missing() {
        println!(
            "\nThe config will be written, but some prerequisites are missing.\n\
             Create them in Azure, then re-run `quelch validate` to re-check."
        );
    }

    Ok(config)
}

// ---------------------------------------------------------------------------
// Post-init helpers
// ---------------------------------------------------------------------------

fn print_env_var_summary(config: &Config) -> anyhow::Result<()> {
    let yaml = serde_yaml::to_string(config)?;
    let env_refs = prompts::collect_env_var_refs(&yaml);
    if env_refs.is_empty() {
        return Ok(());
    }
    println!("\nBefore running Quelch, set these env vars:");
    for name in &env_refs {
        let status = match std::env::var(name) {
            Ok(v) if !v.is_empty() => "[OK] set in current shell",
            _ => "[!!] NOT set in current shell",
        };
        println!("  - {name}   {status}");
    }
    Ok(())
}

/// Offer to `git init` the project directory and write a recommended
/// `.gitignore`. Idempotent.
fn offer_git_setup(output_path: &Path) -> anyhow::Result<()> {
    let project_dir = output_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let is_git_repo = project_dir.join(".git").exists();
    let gitignore_path = project_dir.join(".gitignore");
    let gitignore_has_block = std::fs::read_to_string(&gitignore_path)
        .map(|s| s.contains("# Quelch"))
        .unwrap_or(false);

    if is_git_repo && gitignore_has_block {
        return Ok(());
    }

    println!();
    let prompt = if is_git_repo {
        "Add a recommended .gitignore block to this repo? (keeps .env files and .quelch/ out of git)"
    } else {
        "Initialise this folder as a git repo and write a recommended .gitignore?"
    };
    let yes = inquire::Confirm::new(prompt).with_default(true).prompt()?;
    if !yes {
        return Ok(());
    }

    if !is_git_repo {
        let status = std::process::Command::new("git")
            .arg("init")
            .arg("--initial-branch=main")
            .arg(&project_dir)
            .status();
        match status {
            Ok(s) if s.success() => {
                println!("Initialised git repo at {}", project_dir.display())
            }
            Ok(s) => {
                println!("git init exited with status {s}; skipping .gitignore");
                return Ok(());
            }
            Err(e) => {
                println!("Could not run git ({e}); skipping .gitignore");
                return Ok(());
            }
        }
    }

    write_or_append_gitignore(&gitignore_path)?;
    Ok(())
}

const GITIGNORE_BLOCK: &str = "\
# Quelch — keep secrets and generated artefacts out of version control.
.env
.env.*
.quelch/
";

fn write_or_append_gitignore(path: &Path) -> anyhow::Result<()> {
    match std::fs::read_to_string(path) {
        Err(_) => {
            std::fs::write(path, GITIGNORE_BLOCK)?;
            println!("Wrote {}", path.display());
        }
        Ok(existing) if existing.contains("# Quelch") => {
            println!("{} already contains the Quelch block", path.display());
        }
        Ok(existing) => {
            let mut s = existing;
            if !s.ends_with('\n') {
                s.push('\n');
            }
            s.push('\n');
            s.push_str(GITIGNORE_BLOCK);
            std::fs::write(path, s)?;
            println!("Appended Quelch block to {}", path.display());
        }
    }
    Ok(())
}

fn write_yaml(config: &Config, path: &Path) -> anyhow::Result<()> {
    let yaml = serde_yaml::to_string(config)?;
    std::fs::write(path, yaml)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::validate::validate;
    use tempfile::NamedTempFile;

    fn temp_yaml_path() -> std::path::PathBuf {
        let f = NamedTempFile::new().unwrap();
        let p = f.path().to_path_buf();
        drop(f);
        p
    }

    #[tokio::test]
    async fn non_interactive_writes_minimal_template() {
        let path = temp_yaml_path();
        run(
            &path,
            InitOptions {
                non_interactive: true,
                from_template: None,
                force: false,
            },
        )
        .await
        .unwrap();

        assert!(path.exists(), "quelch.yaml must be written");
        let written = std::fs::read_to_string(&path).unwrap();
        let cfg: Config = serde_yaml::from_str(&written).unwrap();
        validate(&cfg).expect("written config must validate");
        assert_eq!(cfg.source_connections.len(), 1);
    }

    #[tokio::test]
    async fn non_interactive_respects_from_template() {
        let path = temp_yaml_path();
        run(
            &path,
            InitOptions {
                non_interactive: true,
                from_template: Some("multi-source".to_string()),
                force: false,
            },
        )
        .await
        .unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        let cfg: Config = serde_yaml::from_str(&written).unwrap();
        assert_eq!(cfg.source_connections.len(), 2);
    }

    #[tokio::test]
    async fn refuses_overwrite_without_force() {
        let path = temp_yaml_path();
        std::fs::write(&path, "# existing").unwrap();

        let err = run(
            &path,
            InitOptions {
                non_interactive: true,
                from_template: None,
                force: false,
            },
        )
        .await
        .unwrap_err();

        assert!(
            err.to_string().contains("already exists"),
            "error must mention 'already exists': {err}"
        );
    }

    #[tokio::test]
    async fn force_overwrites_existing_file() {
        let path = temp_yaml_path();
        std::fs::write(&path, "# old content").unwrap();

        run(
            &path,
            InitOptions {
                non_interactive: true,
                from_template: None,
                force: true,
            },
        )
        .await
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            !content.contains("# old content"),
            "file should have been overwritten"
        );
        let cfg: Config = serde_yaml::from_str(&content).unwrap();
        validate(&cfg).expect("overwritten config must validate");
    }

    #[tokio::test]
    async fn unknown_template_returns_error() {
        let path = temp_yaml_path();
        let err = run(
            &path,
            InitOptions {
                non_interactive: true,
                from_template: Some("does-not-exist".to_string()),
                force: false,
            },
        )
        .await
        .unwrap_err();

        assert!(
            err.to_string().contains("does-not-exist"),
            "error must mention the unknown name: {err}"
        );
    }

    #[tokio::test]
    async fn distributed_template_round_trips_through_run() {
        let path = temp_yaml_path();
        run(
            &path,
            InitOptions {
                non_interactive: true,
                from_template: Some("distributed".to_string()),
                force: true,
            },
        )
        .await
        .unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        let cfg: Config = serde_yaml::from_str(&written).unwrap();
        validate(&cfg).expect("distributed template validates");
        // Distributed has two ingest instances + one MCP.
        let ingests = cfg
            .instances
            .iter()
            .filter(|i| matches!(i.spec, crate::config::schema::InstanceSpec::Ingest(_)))
            .count();
        assert_eq!(ingests, 2);
    }
}
