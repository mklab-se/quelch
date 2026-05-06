//! Azure AI Search configuration via the embedded `rigg-core` library.
//!
//! Phase 4 of the no-deploy pivot: this module operates purely on in-memory
//! state. There are no on-disk YAML files, no `rigg/` directory.
//!
//! Public surface (after Phase 4 lands fully):
//!   - [`generate`] — build the desired state from a [`crate::config::Config`].
//!   - `plan` — diff the desired state against the live AI Search service.
//!   - `apply` — make the live service match the desired state.
//!
//! All three operate on the in-memory [`RiggDesiredState`] struct.

pub mod generate;
pub mod plan;
pub mod push;

use rigg_core::resources::{DataSource, Index, Indexer, KnowledgeBase, KnowledgeSource, Skillset};

pub use generate::{GenerateError, generate};
pub use plan::{
    PlanError, PlanReport, ResourceDiff, ResourceRef, RiggApiAdapter, RiggClientAdapter,
};
pub use push::{PushError, PushOutcome};

/// In-memory desired state for the Azure AI Search resources Quelch manages.
///
/// Built by [`generate`] from a `quelch.yaml` config; will be consumed by
/// the new `plan` (to compute the diff against the live service) and `apply`
/// (to push the desired state) entry points after Task 4.3.
#[derive(Debug, Default)]
pub struct RiggDesiredState {
    /// AI Search indexes — one per Cosmos container exposed by any MCP instance.
    pub indexes: Vec<Index>,
    /// AI Search indexers — one per index, pulling from the matching data source.
    pub indexers: Vec<Indexer>,
    /// AI Search skillsets — one per index, performing vectorisation.
    pub skillsets: Vec<Skillset>,
    /// AI Search data sources — one per Cosmos container.
    pub data_sources: Vec<DataSource>,
    /// AI Search knowledge sources — one per index, wrapping it for Agentic Retrieval.
    pub knowledge_sources: Vec<KnowledgeSource>,
    /// AI Search knowledge bases — one per MCP instance, listing its KS members.
    pub knowledge_bases: Vec<KnowledgeBase>,
}
