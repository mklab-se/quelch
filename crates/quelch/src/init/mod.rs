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
    println!("Wrote {}", output_path.display());
    println!(
        "Next steps: review the config, fill in any ${{}}-placeholders, then run `quelch validate`."
    );
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
        "Quelch does not provision Azure infrastructure for you — it expects the\n\
         resource group, Cosmos DB account, AI Search service, AI model provider\n\
         (Foundry project or Azure OpenAI account), Container Apps environment,\n\
         Application Insights component, and Key Vault to already exist. See\n\
         docs/getting-started.md for the full prerequisites list and `az`\n\
         commands."
    );
    println!();

    let azure = prompts::azure_section().await?;
    let ai = prompts::ai_section(&azure).await?;
    let sources = prompts::sources_section().await?;
    let deployments = prompts::deployments_section(&sources).await?;
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
