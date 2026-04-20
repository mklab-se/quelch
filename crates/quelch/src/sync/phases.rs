//! String constants used as the `phase` field in structured tracing events.
//! Shared between the sync engine (emitter) and the TUI layer (consumer)
//! so renaming one end forces the other.

pub const CYCLE_STARTED: &str = "cycle_started";
pub const CYCLE_FINISHED: &str = "cycle_finished";
pub const SOURCE_STARTED: &str = "source_started";
pub const SOURCE_FINISHED: &str = "source_finished";
pub const SOURCE_FAILED: &str = "source_failed";
pub const SUBSOURCE_STARTED: &str = "subsource_started";
pub const SUBSOURCE_FINISHED: &str = "subsource_finished";
pub const SUBSOURCE_FAILED: &str = "subsource_failed";
pub const SUBSOURCE_BATCH: &str = "subsource_batch";
pub const SUBSOURCE_EMPTY: &str = "subsource_empty";
pub const DOC_SYNCED: &str = "doc_synced";
pub const AZURE_RESPONSE: &str = "azure_response";
