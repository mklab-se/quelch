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
/// One document confirmed landed in Azure AI Search. Fired AFTER
/// `push_documents` returns success. This — not `doc_synced` — is the event
/// the TUI's "recent pushes" and "latest ID" readouts must listen to.
pub const DOC_PUSHED: &str = "doc_pushed";
/// Current stage of a subsource's in-flight batch. Payload carries a `stage`
/// field with one of: "fetching", "embedding", "pushing", "idle".
pub const STAGE: &str = "stage";
pub const AZURE_RESPONSE: &str = "azure_response";
pub const BACKOFF_STARTED: &str = "backoff_started";
pub const BACKOFF_FINISHED: &str = "backoff_finished";
