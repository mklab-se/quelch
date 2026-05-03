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
/// - `plan` — diffs local files against live Azure resources; produces a
///   [`plan::PlanReport`].
/// - `push` — applies a [`plan::PlanReport`] to Azure in dependency order.
/// - `pull` — fetches live Azure resources back to local files, respecting
///   ownership markers.
pub mod generate;
pub mod ownership;
pub mod plan;
pub mod pull;
pub mod push;
pub mod write;

pub use generate::{GenerateError, GeneratedRiggFiles, all};
pub use plan::{
    PlanError, PlanReport, ResourceDiff, ResourceRef, RiggApiAdapter, RiggClientAdapter,
};
pub use pull::{PullError, PullOptions, PullOutcome};
pub use push::{PushError, PushOutcome};
pub use write::{WriteError, WriteOutcome, write_to_disk};
