//! Where-filter grammar: parser and translators.
//!
//! # Overview
//!
//! This module provides a structured filter AST ([`Where`]) and two translators:
//!
//! - [`cosmos_sql`] — emits a Cosmos DB SQL `WHERE` fragment with `@p0…` parameters.
//! - [`odata`] — emits an AI Search OData `$filter` string with inline values.
//!
//! ## Grammar
//!
//! Filters are expressed as JSON objects.  See `docs/mcp-api.md` "Filter grammar"
//! for the full specification.  Examples:
//!
//! ```json
//! { "status": "Open" }
//! { "type": ["Story", "Bug"] }
//! { "story_points": { "gte": 3, "lt": 8 } }
//! { "and": [ { "status": "Open" }, { "assignee.email": "alice@example.com" } ] }
//! { "fix_versions[].name": "2.7.0" }
//! { "issuelinks": { "array_match": { "type": "blocks", "target_key": "DO-1170" } } }
//! ```

pub mod cosmos_sql;
pub mod dates;
pub mod odata;
pub mod parser;

pub use parser::{FieldPath, FieldSegment, FilterError, Op, Where, parse};
