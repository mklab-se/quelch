//! Per-target bundle writers.
//!
//! Each module exposes a `write(bundle: &Bundle, output_dir: &Path) -> Result<(), TargetError>`
//! function that materialises the bundle for that specific agent platform.

pub mod claude_code;
pub mod codex;
pub mod copilot_cli;
pub mod copilot_studio;
pub mod markdown;
pub mod vscode_copilot;
