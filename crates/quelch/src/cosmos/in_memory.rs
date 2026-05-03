//! In-memory Cosmos DB backend for tests and `quelch dev`.
//!
//! Documents are stored in a nested `HashMap`:
//! `container → (id, partition_key) → Value`
//!
//! The SQL parser implements a deliberately minimal subset:
//! - `SELECT * FROM c`  (full scan)
//! - `SELECT * FROM c WHERE c.id = @id`
//! - `SELECT * FROM c WHERE c._partition_key = @pk`
//! - `SELECT * FROM c WHERE c._deleted = false`
//! - `SELECT * FROM c WHERE (NOT IS_DEFINED(c._deleted) OR c._deleted = false)` (soft-delete guard)
//! - AND combinations of any of the above predicates, including parenthesized groups
//! - `SELECT VALUE COUNT(1) FROM c [WHERE ...]` (count with optional filter)
//!
//! Anything outside this list returns `CosmosError::Unsupported`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::cosmos::{
    CosmosBackend, CosmosError, QueryStream, document::CosmosDocument, query_stream::VecQueryStream,
};

type Store = Arc<Mutex<HashMap<String, HashMap<(String, String), Value>>>>;

/// In-memory Cosmos DB backend.
///
/// Thread-safe via `Arc<Mutex<...>>`. Safe to clone — all clones share state.
#[derive(Clone, Default)]
pub struct InMemoryCosmos {
    store: Store,
}

impl InMemoryCosmos {
    /// Create a new empty in-memory backend.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CosmosBackend for InMemoryCosmos {
    async fn upsert(&self, container: &str, doc: Value) -> Result<(), CosmosError> {
        let id = CosmosDocument::extract_id(&doc)?.to_string();
        let pk = CosmosDocument::extract_partition_key(&doc)?.to_string();
        let mut store = self.store.lock().expect("in-memory store poisoned");
        store
            .entry(container.to_string())
            .or_default()
            .insert((id, pk), doc);
        Ok(())
    }

    async fn get(
        &self,
        container: &str,
        id: &str,
        partition_key: &str,
    ) -> Result<Option<Value>, CosmosError> {
        let store = self.store.lock().expect("in-memory store poisoned");
        let result = store
            .get(container)
            .and_then(|c| c.get(&(id.to_string(), partition_key.to_string())))
            .cloned();
        Ok(result)
    }

    async fn query(
        &self,
        container: &str,
        sql: &str,
        params: Vec<(String, Value)>,
    ) -> Result<QueryStream, CosmosError> {
        let store = self.store.lock().expect("in-memory store poisoned");
        let all_docs: Vec<Value> = store
            .get(container)
            .map(|c| c.values().cloned().collect())
            .unwrap_or_default();
        drop(store);

        let params_map: HashMap<String, Value> = params.into_iter().collect();
        let results = execute_sql(sql, all_docs, &params_map)?;
        Ok(QueryStream::new(Box::new(VecQueryStream::new(results))))
    }
}

// ---------------------------------------------------------------------------
// Minimal SQL parser
// ---------------------------------------------------------------------------

/// Parse and execute a minimal SQL-subset query against an in-memory document set.
fn execute_sql(
    sql: &str,
    docs: Vec<Value>,
    params: &HashMap<String, Value>,
) -> Result<Vec<Value>, CosmosError> {
    let normalised = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    let upper = normalised.to_uppercase();

    // `SELECT VALUE COUNT(1) FROM c [WHERE ...]`
    if let Some(rest_upper) = upper.strip_prefix("SELECT VALUE COUNT(1) FROM C") {
        let rest_upper = rest_upper.trim();
        if rest_upper.is_empty() {
            // No WHERE — count everything
            let count = docs.len() as u64;
            return Ok(vec![json!(count)]);
        }
        // Count with WHERE clause — work from normalised (preserves case of params)
        let rest_norm = normalised["SELECT VALUE COUNT(1) FROM c".len()..]
            .trim()
            .to_string();
        let where_clause = strip_prefix_case_insensitive(&rest_norm, "WHERE")
            .ok_or_else(|| CosmosError::Unsupported(format!("query: {sql}")))?
            .trim()
            .to_string();
        let predicates = parse_predicates(&where_clause, sql)?;
        let count = docs
            .into_iter()
            .filter(|doc| predicates.iter().all(|p| p.matches(doc, params)))
            .count() as u64;
        return Ok(vec![json!(count)]);
    }

    // Must start with `SELECT * FROM c` (optionally + WHERE clause)
    let rest = strip_prefix_case_insensitive(&normalised, "SELECT * FROM c")
        .ok_or_else(|| CosmosError::Unsupported(format!("query: {sql}")))?
        .trim()
        .to_string();

    // No WHERE — full scan
    if rest.is_empty() {
        return Ok(docs);
    }

    // Must continue with WHERE
    let where_clause = strip_prefix_case_insensitive(&rest, "WHERE")
        .ok_or_else(|| CosmosError::Unsupported(format!("query: {sql}")))?
        .trim()
        .to_string();

    let predicates = parse_predicates(&where_clause, sql)?;

    let filtered = docs
        .into_iter()
        .filter(|doc| predicates.iter().all(|p| p.matches(doc, params)))
        .collect();

    Ok(filtered)
}

fn strip_prefix_case_insensitive<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() < prefix.len() {
        return None;
    }
    let (head, tail) = s.split_at(prefix.len());
    if head.eq_ignore_ascii_case(prefix) {
        Some(tail)
    } else {
        None
    }
}

#[derive(Debug)]
enum Predicate {
    /// `c.<field> = @param`
    ParamEq { field: String, param: String },
    /// `c._deleted = false` or `(NOT IS_DEFINED(c._deleted) OR c._deleted = false)`
    NotDeleted,
}

impl Predicate {
    fn matches(&self, doc: &Value, params: &HashMap<String, Value>) -> bool {
        match self {
            Predicate::ParamEq { field, param } => {
                let doc_val = doc.get(field.as_str());
                let param_val = params.get(param.as_str());
                match (doc_val, param_val) {
                    (Some(a), Some(b)) => a == b,
                    _ => false,
                }
            }
            Predicate::NotDeleted => {
                // Matches documents where `_deleted` is absent or `false`.
                doc.get("_deleted")
                    .map(|v| v == &Value::Bool(false))
                    .unwrap_or(true)
            }
        }
    }
}

/// Split a WHERE clause into top-level AND operands, respecting parentheses.
///
/// e.g. `(c.status = @p0) AND (NOT IS_DEFINED(c._deleted) OR c._deleted = false)`
/// yields `["(c.status = @p0)", "(NOT IS_DEFINED(c._deleted) OR c._deleted = false)"]`.
fn split_and_top_level(where_clause: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let chars: Vec<char> = where_clause.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ' ' if depth == 0 => {
                // Check if we're at an AND keyword
                let rest: String = chars[i..].iter().collect();
                let rest_upper = rest.to_uppercase();
                if rest_upper.starts_with(" AND ") {
                    parts.push(current.trim().to_string());
                    current = String::new();
                    i += 5; // skip " AND "
                    continue;
                } else {
                    current.push(c);
                }
            }
            _ => {
                current.push(c);
            }
        }
        i += 1;
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        parts.push(trimmed);
    }
    parts
}

/// Parse the WHERE clause (after the `WHERE` keyword) into individual predicates.
/// Supports AND-separated simple expressions, including parenthesized groups.
fn parse_predicates(where_clause: &str, original_sql: &str) -> Result<Vec<Predicate>, CosmosError> {
    let normalised = where_clause
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let parts = split_and_top_level(&normalised);

    let mut predicates = Vec::new();
    for expr in parts {
        let pred = parse_single_predicate(&expr, original_sql)?;
        predicates.push(pred);
    }
    Ok(predicates)
}

/// Parse a single predicate expression.
///
/// Recognised forms:
/// - `c.<field> = @param`
/// - `c._deleted = false`
/// - `(c.<field> = @param)` — parenthesized
/// - `(NOT IS_DEFINED(c._deleted) OR c._deleted = false)` — soft-delete guard
fn parse_single_predicate(expr: &str, original_sql: &str) -> Result<Predicate, CosmosError> {
    let expr = expr.trim();

    // Strip outer parens if present and recurse
    if expr.starts_with('(') && expr.ends_with(')') {
        let inner = &expr[1..expr.len() - 1];

        // Recognise soft-delete guard pattern generated by SqlBuilder:
        // `NOT IS_DEFINED(c._deleted) OR c._deleted = false`
        let inner_norm = inner.split_whitespace().collect::<Vec<_>>().join(" ");
        let inner_upper = inner_norm.to_uppercase();
        if inner_upper == "NOT IS_DEFINED(C._DELETED) OR C._DELETED = FALSE" {
            return Ok(Predicate::NotDeleted);
        }

        return parse_single_predicate(inner, original_sql);
    }

    // `c._deleted = false`
    {
        let norm = expr.split_whitespace().collect::<Vec<_>>().join(" ");
        let upper = norm.to_uppercase();
        if upper == "C._DELETED = FALSE" {
            return Ok(Predicate::NotDeleted);
        }
    }

    // `c.<field> = @param`
    let parts: Vec<&str> = expr.splitn(2, '=').collect();
    if parts.len() != 2 {
        return Err(CosmosError::Unsupported(format!("query: {original_sql}")));
    }
    let lhs = parts[0].trim();
    let rhs = parts[1].trim();

    // lhs must be `c.<field>`
    let field = lhs
        .strip_prefix("c.")
        .ok_or_else(|| CosmosError::Unsupported(format!("query: {original_sql}")))?;

    if let Some(param_name) = rhs.strip_prefix('@') {
        return Ok(Predicate::ParamEq {
            field: field.to_string(),
            param: format!("@{param_name}"),
        });
    }

    Err(CosmosError::Unsupported(format!("query: {original_sql}")))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_doc(id: &str, pk: &str) -> Value {
        json!({ "id": id, "_partition_key": pk, "data": "hello" })
    }

    fn make_doc_with_deleted(id: &str, pk: &str, deleted: bool) -> Value {
        json!({ "id": id, "_partition_key": pk, "_deleted": deleted })
    }

    // -- Backend trait tests -------------------------------------------------

    #[tokio::test]
    async fn upsert_and_get_round_trip() {
        let backend = InMemoryCosmos::new();
        let doc = make_doc("d1", "pk1");
        backend.upsert("my-container", doc.clone()).await.unwrap();
        let got = backend.get("my-container", "d1", "pk1").await.unwrap();
        assert_eq!(got, Some(doc));
    }

    #[tokio::test]
    async fn point_read_with_wrong_partition_returns_none() {
        let backend = InMemoryCosmos::new();
        backend.upsert("c", make_doc("id1", "pk1")).await.unwrap();
        let got = backend.get("c", "id1", "wrong-pk").await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn upsert_overwrites_existing() {
        let backend = InMemoryCosmos::new();
        let doc1 = json!({ "id": "x", "_partition_key": "pk", "val": 1 });
        let doc2 = json!({ "id": "x", "_partition_key": "pk", "val": 2 });
        backend.upsert("c", doc1).await.unwrap();
        backend.upsert("c", doc2.clone()).await.unwrap();
        let got = backend.get("c", "x", "pk").await.unwrap().unwrap();
        assert_eq!(got["val"], 2);
    }

    #[tokio::test]
    async fn bulk_upsert_round_trips() {
        let backend = InMemoryCosmos::new();
        let docs = vec![
            make_doc("a", "pk"),
            make_doc("b", "pk"),
            make_doc("c", "pk"),
        ];
        backend.bulk_upsert("cont", docs).await.unwrap();
        for id in ["a", "b", "c"] {
            assert!(backend.get("cont", id, "pk").await.unwrap().is_some());
        }
    }

    // -- SQL query tests -----------------------------------------------------

    #[tokio::test]
    async fn query_select_star_returns_all() {
        let backend = InMemoryCosmos::new();
        backend.upsert("c", make_doc("1", "pk")).await.unwrap();
        backend.upsert("c", make_doc("2", "pk")).await.unwrap();
        let mut stream = backend.query("c", "SELECT * FROM c", vec![]).await.unwrap();
        let page = stream.next_page().await.unwrap().unwrap();
        assert_eq!(page.len(), 2);
    }

    #[tokio::test]
    async fn query_by_id_param() {
        let backend = InMemoryCosmos::new();
        backend.upsert("c", make_doc("target", "pk")).await.unwrap();
        backend.upsert("c", make_doc("other", "pk")).await.unwrap();
        let mut stream = backend
            .query(
                "c",
                "SELECT * FROM c WHERE c.id = @id",
                vec![("@id".into(), json!("target"))],
            )
            .await
            .unwrap();
        let page = stream.next_page().await.unwrap().unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0]["id"], "target");
    }

    #[tokio::test]
    async fn query_by_partition_key() {
        let backend = InMemoryCosmos::new();
        backend.upsert("c", make_doc("a", "pk1")).await.unwrap();
        backend.upsert("c", make_doc("b", "pk2")).await.unwrap();
        backend.upsert("c", make_doc("c", "pk1")).await.unwrap();
        let mut stream = backend
            .query(
                "c",
                "SELECT * FROM c WHERE c._partition_key = @pk",
                vec![("@pk".into(), json!("pk1"))],
            )
            .await
            .unwrap();
        let page = stream.next_page().await.unwrap().unwrap();
        assert_eq!(page.len(), 2);
        for doc in &page {
            assert_eq!(doc["_partition_key"], "pk1");
        }
    }

    #[tokio::test]
    async fn query_not_deleted_filter() {
        let backend = InMemoryCosmos::new();
        backend
            .upsert("c", make_doc_with_deleted("live", "pk", false))
            .await
            .unwrap();
        backend
            .upsert("c", make_doc_with_deleted("dead", "pk", true))
            .await
            .unwrap();
        backend
            .upsert("c", make_doc("no-flag", "pk"))
            .await
            .unwrap();
        let mut stream = backend
            .query("c", "SELECT * FROM c WHERE c._deleted = false", vec![])
            .await
            .unwrap();
        let page = stream.next_page().await.unwrap().unwrap();
        // "live" and "no-flag" (absent == not deleted) should pass; "dead" should not
        assert_eq!(page.len(), 2);
        let ids: Vec<&str> = page.iter().map(|d| d["id"].as_str().unwrap()).collect();
        assert!(!ids.contains(&"dead"));
    }

    #[tokio::test]
    async fn query_count_1() {
        let backend = InMemoryCosmos::new();
        backend.upsert("c", make_doc("a", "pk")).await.unwrap();
        backend.upsert("c", make_doc("b", "pk")).await.unwrap();
        let mut stream = backend
            .query("c", "SELECT VALUE COUNT(1) FROM c", vec![])
            .await
            .unwrap();
        let page = stream.next_page().await.unwrap().unwrap();
        assert_eq!(page, vec![json!(2u64)]);
    }

    #[tokio::test]
    async fn query_and_combination() {
        let backend = InMemoryCosmos::new();
        backend
            .upsert(
                "c",
                json!({ "id": "a1", "_partition_key": "pk1", "_deleted": false }),
            )
            .await
            .unwrap();
        backend
            .upsert(
                "c",
                json!({ "id": "b1", "_partition_key": "pk1", "_deleted": true }),
            )
            .await
            .unwrap();
        backend
            .upsert(
                "c",
                json!({ "id": "a2", "_partition_key": "pk2", "_deleted": false }),
            )
            .await
            .unwrap();
        let mut stream = backend
            .query(
                "c",
                "SELECT * FROM c WHERE c._partition_key = @pk AND c._deleted = false",
                vec![("@pk".into(), json!("pk1"))],
            )
            .await
            .unwrap();
        let page = stream.next_page().await.unwrap().unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0]["id"], "a1");
    }

    #[tokio::test]
    async fn query_unsupported_returns_error() {
        let backend = InMemoryCosmos::new();
        let result = backend
            .query("c", "SELECT id FROM c WHERE c.foo = 'bar'", vec![])
            .await;
        assert!(matches!(result, Err(CosmosError::Unsupported(_))));
    }

    #[tokio::test]
    async fn query_second_page_returns_none_in_memory() {
        let backend = InMemoryCosmos::new();
        backend.upsert("c", make_doc("x", "pk")).await.unwrap();
        let mut stream = backend.query("c", "SELECT * FROM c", vec![]).await.unwrap();
        let _ = stream.next_page().await.unwrap(); // first page
        let second = stream.next_page().await.unwrap();
        assert!(second.is_none());
        assert!(stream.continuation_token().is_none());
    }

    #[tokio::test]
    async fn get_from_missing_container_returns_none() {
        let backend = InMemoryCosmos::new();
        let got = backend.get("nonexistent", "id", "pk").await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn upsert_without_id_returns_validation_error() {
        let backend = InMemoryCosmos::new();
        let bad_doc = json!({ "_partition_key": "pk", "data": "no id" });
        let result = backend.upsert("c", bad_doc).await;
        assert!(matches!(result, Err(CosmosError::Validation(_))));
    }
}
