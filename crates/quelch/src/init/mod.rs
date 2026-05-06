pub mod discover;
pub mod prereq;
pub mod prompts;
pub mod templates;

use std::path::Path;

#[derive(Debug, Default)]
pub struct InitOptions {
    pub non_interactive: bool,
    pub from_template: Option<String>,
    pub force: bool,
}

pub async fn run(_output_path: &Path, _options: InitOptions) -> anyhow::Result<()> {
    todo!("phase 9: rewrite the init wizard for the new instances/source_connections schema")
}
