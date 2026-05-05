/// Interactive wizard and non-interactive scaffolding for `quelch init`.
///
/// Entry point: [`run`].
pub mod discover;
pub mod prereq;
pub mod prompts;
pub mod templates;

use crate::config::{Config, IngestConfig, RiggConfig, StateConfig};
use std::path::Path;

/// Options that control the `init` command.
#[derive(Debug, Default)]
pub struct InitOptions {
    /// Skip all prompts and write a template directly.
    pub non_interactive: bool,
    /// Template name to use in non-interactive mode (default: "minimal").
    pub from_template: Option<String>,
    /// Overwrite an existing `quelch.yaml` without asking.
    pub force: bool,
}

/// Run the `quelch init` command.
///
/// # Non-interactive mode
/// Writes the named template (or "minimal" if none is given) directly to
/// `output_path`.
///
/// # Interactive mode
/// Runs a wizard that prompts for each config section and writes the result.
///
/// # Errors
/// - Returns an error if `output_path` already exists and `--force` is not set.
/// - Returns an error if the template name is unknown.
/// - Returns an error on I/O failure.
pub async fn run(output_path: &Path, options: InitOptions) -> anyhow::Result<()> {
    if output_path.exists() && !options.force {
        anyhow::bail!(
            "{} already exists. Use --force to overwrite.",
            output_path.display()
        );
    }

    if options.non_interactive {
        let cfg = templates::template_for(options.from_template.as_deref().unwrap_or("minimal"))?;
        write_yaml(&cfg, output_path)?;
        println!(
            "Wrote {} (template: {})",
            output_path.display(),
            options.from_template.as_deref().unwrap_or("minimal")
        );
        return Ok(());
    }

    let config = run_interactive().await?;
    write_yaml(&config, output_path)?;
    println!("\nWrote {}", output_path.display());

    // List every `${VAR}` placeholder the wizard wrote, with current set/unset
    // status, so the user knows exactly what env vars to export before running.
    let yaml = serde_yaml::to_string(&config)?;
    let env_refs = prompts::collect_env_var_refs(&yaml);
    if !env_refs.is_empty() {
        println!("\nBefore running Quelch, set these env vars (locally and on Q-Ingest):");
        for name in &env_refs {
            let status = match std::env::var(name) {
                Ok(v) if !v.is_empty() => "✓ set in current shell",
                _ => "✗ NOT set in current shell",
            };
            println!("  - {name}   {status}");
        }
        println!(
            "\nFor a Container App, attach these as secrets / env vars on the\n\
             deployment (e.g. `az containerapp update --set-env-vars …`)."
        );
    }

    // Offer git init + .gitignore. quelch.yaml has connection strings (and
    // sometimes environment-var placeholders that hint at internal naming);
    // keeping the file in a private repo or out of source control is the safe
    // default. This is opt-in via Confirm.
    let project_dir = output_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    offer_git_setup(&project_dir)?;

    println!("\nNext: run `quelch validate` to verify the config and prerequisites.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Git / .gitignore bootstrap
// ---------------------------------------------------------------------------

/// Recommended `.gitignore` block for a Quelch project directory. Keeps the
/// list short and curated — anything more is the user's call.
const GITIGNORE_BLOCK: &str = "\
# Quelch — keep secrets and generated artefacts out of version control.
.env
.env.*
.quelch/
";

/// Offer to `git init` the project directory and write a recommended
/// `.gitignore`. Idempotent: re-running on an already-initialised repo only
/// touches `.gitignore`, and only if the Quelch block isn't already there.
fn offer_git_setup(project_dir: &Path) -> anyhow::Result<()> {
    let is_git_repo = project_dir.join(".git").exists();
    let gitignore_path = project_dir.join(".gitignore");
    let gitignore_has_block = std::fs::read_to_string(&gitignore_path)
        .map(|s| s.contains("# Quelch"))
        .unwrap_or(false);

    // Nothing to do if the repo already exists and has our gitignore block.
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
            .arg(project_dir)
            .status();
        match status {
            Ok(s) if s.success() => println!("✓ Initialised git repo at {}", project_dir.display()),
            Ok(s) => {
                println!("✗ git init exited with status {s}; skipping .gitignore");
                return Ok(());
            }
            Err(e) => {
                println!("✗ Could not run git ({e}); skipping .gitignore");
                return Ok(());
            }
        }
    }

    write_or_append_gitignore(&gitignore_path)?;
    Ok(())
}

/// Write `GITIGNORE_BLOCK` to `path` if the file does not exist, or append it
/// to an existing file that does not already contain the block.
fn write_or_append_gitignore(path: &Path) -> anyhow::Result<()> {
    match std::fs::read_to_string(path) {
        Err(_) => {
            std::fs::write(path, GITIGNORE_BLOCK)?;
            println!("✓ Wrote {}", path.display());
        }
        Ok(existing) if existing.contains("# Quelch") => {
            println!("✓ {} already contains the Quelch block", path.display());
        }
        Ok(existing) => {
            let mut s = existing;
            if !s.ends_with('\n') {
                s.push('\n');
            }
            s.push('\n');
            s.push_str(GITIGNORE_BLOCK);
            std::fs::write(path, s)?;
            println!("✓ Appended Quelch block to {}", path.display());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Interactive flow
// ---------------------------------------------------------------------------

async fn run_interactive() -> anyhow::Result<Config> {
    println!("Welcome to quelch init.");
    println!("This wizard will create a quelch.yaml for your environment.");
    println!();
    println!(
        "What Quelch deploys for you:\n\
         \x20 - Container Apps for Q-MCP and (optionally) Q-Ingest, via Bicep.\n\
         \n\
         What you must create up front (Quelch only references these — it does not\n\
         provision them):\n\
         \x20 - Cosmos DB account\n\
         \x20 - Azure AI Search service\n\
         \x20 - AI model provider (Microsoft Foundry project or Azure OpenAI account)\n\
         \x20   with one embedding deployment and one chat deployment\n\
         \x20 - Container Apps environment\n\
         \x20 - Application Insights component\n\
         \x20 - Key Vault\n\
         \n\
         These can each live in any resource group in your subscription — the\n\
         wizard will let you point at each one individually. See\n\
         docs/getting-started.md for the full prerequisites list and `az`\n\
         commands."
    );
    println!();

    let mut azure = prompts::azure_section().await?;
    let ai = prompts::ai_section(&azure).await?;
    let sources = prompts::sources_section().await?;
    let deployments = prompts::deployments_section(&sources).await?;

    // Region / naming-prefix / environment-tag only matter for Azure-targeted
    // deployments — defer asking until we know the shape.
    if deployments
        .iter()
        .any(|d| matches!(d.target, crate::config::DeploymentTarget::Azure))
    {
        prompts::naming_settings(&mut azure).await?;
    }

    let mcp = prompts::mcp_section(&deployments).await?;

    let config = Config {
        azure,
        cosmos: crate::config::CosmosConfig::default(),
        search: crate::config::SearchConfig::default(),
        ai,
        sources,
        ingest: IngestConfig::default(),
        deployments,
        mcp,
        rigg: RiggConfig::default(),
        state: StateConfig::default(),
    };

    let report = prereq::check_all(&config).await;
    report.print();
    if report.has_missing() {
        println!(
            "\nThe config has been written, but some prerequisites are missing.\n\
             Create them in Azure, then run `quelch validate` to re-check."
        );
    }

    Ok(config)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
    use crate::config::validate;
    use tempfile::NamedTempFile;

    fn temp_yaml_path() -> std::path::PathBuf {
        let f = NamedTempFile::new().unwrap();
        let p = f.path().to_path_buf();
        drop(f); // release the file so `run` can write it
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
        validate::run(&cfg).expect("written config must pass validation");
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
        // Multi-source has 2 sources: Jira + Confluence.
        assert_eq!(cfg.sources.len(), 2);
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
        // Written file must be parseable as Config.
        let cfg: Config = serde_yaml::from_str(&content).unwrap();
        validate::run(&cfg).expect("overwritten config must be valid");
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
            "error must mention the unknown template name: {err}"
        );
    }
}
