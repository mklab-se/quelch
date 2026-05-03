/// rigg integration: generate, plan, push, and pull AI Search resource files.
///
/// This module generates rigg-format YAML files for every AI Search resource
/// implied by a `quelch.yaml` config, then embeds `rigg-client` to plan and
/// push them against a live Azure AI Search service.
///
/// ## Module layout
///
/// - `generate` — converts a [`Config`] into in-memory YAML strings grouped by
///   resource type.
/// - `ownership` — checks whether a file carries the `# rigg:managed-by-user`
///   hand-takeover marker.
/// - `write` — persists [`GeneratedRiggFiles`] to disk, respecting ownership
///   markers and the global `RiggConfig::ownership` setting.
pub mod generate;
pub mod ownership;
pub mod write;

pub use generate::{GenerateError, GeneratedRiggFiles, all};
pub use write::{WriteError, WriteOutcome, write_to_disk};
