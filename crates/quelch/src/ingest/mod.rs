/// Initial backfill and backfill-resume logic.
pub mod backfill;
/// Runtime configuration for the ingest cycle engine.
pub mod config;
/// Enum dispatch for heterogeneous connector types.
pub mod connector_kind;
/// Per-cycle ingest algorithm.
pub mod cycle;
/// Rate-limit-aware HTTP client wrapping `reqwest`.
pub mod rate_limit;
/// Deletion reconciliation.
pub mod reconcile;
/// Test helpers: `MockConnector` and document builders.
#[cfg(test)]
pub(crate) mod test_helpers;
/// Minute-resolution window planning for incremental sync.
pub mod window;
/// Ingest worker: drives the cycle engine for a deployment.
pub mod worker;
