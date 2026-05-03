//! Where-grammar JSON parser.
//!
//! Converts a JSON object (`serde_json::Value`) into a typed [`Where`] AST.
//!
//! ## Grammar shapes
//!
//! | JSON | AST |
//! |------|-----|
//! | `{"status":"Open"}` | `Field { op: Eq }` |
//! | `{"type":["Story","Bug"]}` | `Field { op: In([…]) }` |
//! | `{"score":{"gte":3}}` | `Field { op: Gte }` |
//! | `{"score":{"lt":8}}` | `Field { op: Lt }` |
//! | `{"name":{"like":"%foo%"}}` | `Field { op: Like }` |
//! | `{"status":{"not":"Done"}}` | `Not(Field { op: Eq })` |
//! | `{"field":{"exists":true}}` | `Exists { present: true }` |
//! | `{"field":{"array_match":{…}}}` | `ArrayMatch { … }` |
//! | `{"and":[…]}` | `And([…])` |
//! | `{"or":[…]}` | `Or([…])` |
//!
//! Multiple top-level keys produce an implicit `And`.

use serde_json::{Map, Value};
use thiserror::Error;

/// Errors produced by the filter parser.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum FilterError {
    /// The filter JSON is structurally invalid (wrong types, missing fields, etc.).
    #[error("invalid filter: {0}")]
    Invalid(String),

    /// The filter uses a grammar shape not supported by this backend.
    #[error("unsupported filter construct: {0}")]
    Unsupported(String),
}

/// A single segment in a dot-separated field path.
///
/// `"fix_versions[].name"` parses to:
/// ```text
/// [FieldSegment { name: "fix_versions", array_projection: true },
///  FieldSegment { name: "name",         array_projection: false }]
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct FieldSegment {
    /// The field name without any `[]` suffix.
    pub name: String,
    /// `true` if the original segment ended with `[]`.
    pub array_projection: bool,
}

impl FieldSegment {
    fn parse(s: &str) -> Self {
        if let Some(base) = s.strip_suffix("[]") {
            FieldSegment {
                name: base.to_owned(),
                array_projection: true,
            }
        } else {
            FieldSegment {
                name: s.to_owned(),
                array_projection: false,
            }
        }
    }
}

/// A dot-separated field path, possibly containing array-projection segments.
///
/// `"assignee.email"` → `[Name("assignee"), Name("email")]`
/// `"fix_versions[].name"` → `[{fix_versions, array:true}, {name, array:false}]`
#[derive(Debug, Clone, PartialEq)]
pub struct FieldPath {
    /// Ordered list of path segments from outermost to innermost.
    pub segments: Vec<FieldSegment>,
}

impl FieldPath {
    /// Parse a dot-separated path string into a [`FieldPath`].
    pub fn parse(s: &str) -> Self {
        let segments = s.split('.').map(FieldSegment::parse).collect();
        FieldPath { segments }
    }

    /// Returns `true` if any segment has `array_projection = true`.
    pub fn has_array_projection(&self) -> bool {
        self.segments.iter().any(|s| s.array_projection)
    }

    /// Returns the first array-projected segment index, if any.
    pub fn array_projection_index(&self) -> Option<usize> {
        self.segments.iter().position(|s| s.array_projection)
    }
}

/// A filter operator.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Equality: `{"field": value}`.
    Eq,
    /// Membership: `{"field": [v1, v2, …]}`.
    In(Vec<Value>),
    /// Greater-than: `{"field": {"gt": n}}`.
    Gt,
    /// Greater-than-or-equal: `{"field": {"gte": n}}`.
    Gte,
    /// Less-than: `{"field": {"lt": n}}`.
    Lt,
    /// Less-than-or-equal: `{"field": {"lte": n}}`.
    Lte,
    /// SQL-style `LIKE` pattern with `%` wildcards: `{"field": {"like": "%foo%"}}`.
    Like,
}

/// A parsed filter AST node.
#[derive(Debug, Clone, PartialEq)]
pub enum Where {
    /// A leaf field comparison.
    Field {
        /// Dot-separated path to the field.
        path: FieldPath,
        /// The comparison operator (with inline values for `In`).
        op: Op,
        /// The comparison value; `Value::Null` for `In` (values are inside `Op::In`).
        value: Value,
    },
    /// Conjunction: all children must match.
    And(Vec<Where>),
    /// Disjunction: at least one child must match.
    Or(Vec<Where>),
    /// Negation of an inner filter.
    Not(Box<Where>),
    /// Field existence check.
    Exists {
        /// Path to the field being tested.
        path: FieldPath,
        /// `true` = field must exist and be non-empty; `false` = must be absent.
        present: bool,
    },
    /// Same-element predicate over an array field.
    ///
    /// All conditions in `predicate` must hold on the **same** array element.
    /// Use this instead of a top-level `And` when you need correlated multi-field
    /// matching within a single array item.
    ArrayMatch {
        /// Path to the array field.
        path: FieldPath,
        /// Sub-filter evaluated against each element.
        predicate: Box<Where>,
    },
}

/// Parse a JSON filter value into a [`Where`] AST.
///
/// # Errors
///
/// Returns [`FilterError::Invalid`] if the JSON is structurally wrong, or
/// [`FilterError::Unsupported`] if the grammar shape is valid JSON but not
/// implemented.
pub fn parse(value: &Value) -> Result<Where, FilterError> {
    match value {
        Value::Object(map) => parse_object(map),
        _ => Err(FilterError::Invalid(format!(
            "filter must be a JSON object, got {}",
            value_type_name(value)
        ))),
    }
}

fn parse_object(map: &Map<String, Value>) -> Result<Where, FilterError> {
    if map.is_empty() {
        return Err(FilterError::Invalid(
            "filter object must not be empty".into(),
        ));
    }

    // Handle top-level "and" / "or" combinators.
    if let Some(children) = map.get("and") {
        if map.len() > 1 {
            return Err(FilterError::Invalid(
                "'and' must be the only key in its object".into(),
            ));
        }
        let arr = children
            .as_array()
            .ok_or_else(|| FilterError::Invalid("'and' value must be an array".into()))?;
        let children = arr.iter().map(parse).collect::<Result<Vec<_>, _>>()?;
        return Ok(Where::And(children));
    }

    if let Some(children) = map.get("or") {
        if map.len() > 1 {
            return Err(FilterError::Invalid(
                "'or' must be the only key in its object".into(),
            ));
        }
        let arr = children
            .as_array()
            .ok_or_else(|| FilterError::Invalid("'or' value must be an array".into()))?;
        let children = arr.iter().map(parse).collect::<Result<Vec<_>, _>>()?;
        return Ok(Where::Or(children));
    }

    // Each remaining key is a field filter.
    let clauses: Vec<Where> = map
        .iter()
        .map(|(key, val)| parse_field_filter(key, val))
        .collect::<Result<Vec<_>, _>>()?;

    if clauses.len() == 1 {
        Ok(clauses.into_iter().next().unwrap())
    } else {
        Ok(Where::And(clauses))
    }
}

/// Parse a single `"field_path": <op-or-value>` entry.
fn parse_field_filter(key: &str, val: &Value) -> Result<Where, FilterError> {
    let path = FieldPath::parse(key);

    match val {
        // Array → membership (In).
        Value::Array(arr) => Ok(Where::Field {
            path,
            op: Op::In(arr.clone()),
            value: Value::Null,
        }),

        // Object → operator form.
        Value::Object(op_map) => parse_op_object(path, op_map),

        // Scalar → equality.
        scalar => Ok(Where::Field {
            path,
            op: Op::Eq,
            value: scalar.clone(),
        }),
    }
}

/// Parse `{ "gte": 3, "lt": 8 }` / `{ "not": "Done" }` / `{ "exists": true }` etc.
fn parse_op_object(path: FieldPath, op_map: &Map<String, Value>) -> Result<Where, FilterError> {
    if op_map.is_empty() {
        return Err(FilterError::Invalid(
            "operator object must not be empty".into(),
        ));
    }

    // Special single-key operators: "not", "exists", "array_match"
    if op_map.len() == 1 {
        let (op_key, op_val) = op_map.iter().next().unwrap();
        match op_key.as_str() {
            "not" => {
                let inner = Where::Field {
                    path,
                    op: Op::Eq,
                    value: op_val.clone(),
                };
                return Ok(Where::Not(Box::new(inner)));
            }
            "exists" => {
                let present = op_val.as_bool().ok_or_else(|| {
                    FilterError::Invalid("'exists' value must be a boolean".into())
                })?;
                return Ok(Where::Exists { path, present });
            }
            "array_match" => {
                let predicate = parse(op_val)?;
                return Ok(Where::ArrayMatch {
                    path,
                    predicate: Box::new(predicate),
                });
            }
            _ => {}
        }
    }

    // Multi-key case (e.g., { "gte": 3, "lt": 8 }) → implicit And of each op.
    // Single-key comparison ops also go through here.
    let clauses: Vec<Where> = op_map
        .iter()
        .map(|(op_key, op_val)| parse_single_op(&path, op_key, op_val))
        .collect::<Result<Vec<_>, _>>()?;

    if clauses.len() == 1 {
        Ok(clauses.into_iter().next().unwrap())
    } else {
        Ok(Where::And(clauses))
    }
}

/// Parse a single comparison operator entry like `"gte": 3`.
fn parse_single_op(path: &FieldPath, op_key: &str, op_val: &Value) -> Result<Where, FilterError> {
    let op = match op_key {
        "eq" => Op::Eq,
        "gt" => Op::Gt,
        "gte" => Op::Gte,
        "lt" => Op::Lt,
        "lte" => Op::Lte,
        "like" => Op::Like,
        other => {
            return Err(FilterError::Unsupported(format!(
                "unknown operator '{other}'"
            )));
        }
    };
    Ok(Where::Field {
        path: path.clone(),
        op,
        value: op_val.clone(),
    })
}

fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn path(s: &str) -> FieldPath {
        FieldPath::parse(s)
    }

    // ── Equality ──────────────────────────────────────────────────────────

    #[test]
    fn parses_equality() {
        let w = parse(&json!({"status": "Open"})).unwrap();
        assert_eq!(
            w,
            Where::Field {
                path: path("status"),
                op: Op::Eq,
                value: json!("Open"),
            }
        );
    }

    #[test]
    fn parses_numeric_equality() {
        let w = parse(&json!({"story_points": 5})).unwrap();
        assert_eq!(
            w,
            Where::Field {
                path: path("story_points"),
                op: Op::Eq,
                value: json!(5),
            }
        );
    }

    // ── Membership ────────────────────────────────────────────────────────

    #[test]
    fn parses_membership() {
        let w = parse(&json!({"type": ["Story", "Bug"]})).unwrap();
        assert_eq!(
            w,
            Where::Field {
                path: path("type"),
                op: Op::In(vec![json!("Story"), json!("Bug")]),
                value: Value::Null,
            }
        );
    }

    // ── Comparison ────────────────────────────────────────────────────────

    #[test]
    fn parses_comparison_gte() {
        let w = parse(&json!({"story_points": {"gte": 3}})).unwrap();
        assert_eq!(
            w,
            Where::Field {
                path: path("story_points"),
                op: Op::Gte,
                value: json!(3),
            }
        );
    }

    #[test]
    fn parses_comparison_range() {
        // {"story_points": {"gte": 3, "lt": 8}} → And([Field(gte,3), Field(lt,8)])
        let w = parse(&json!({"story_points": {"gte": 3, "lt": 8}})).unwrap();
        // The op_map iteration order may vary, so normalise by unwrapping the And.
        match w {
            Where::And(mut children) => {
                assert_eq!(children.len(), 2);
                // Sort by op discriminant for a stable assertion.
                children.sort_by_key(|c| match c {
                    Where::Field { op: Op::Gte, .. } => 0,
                    Where::Field { op: Op::Lt, .. } => 1,
                    _ => 99,
                });
                assert_eq!(
                    children[0],
                    Where::Field {
                        path: path("story_points"),
                        op: Op::Gte,
                        value: json!(3),
                    }
                );
                assert_eq!(
                    children[1],
                    Where::Field {
                        path: path("story_points"),
                        op: Op::Lt,
                        value: json!(8),
                    }
                );
            }
            other => panic!("expected And, got {other:?}"),
        }
    }

    #[test]
    fn parses_comparison_lte() {
        let w = parse(&json!({"score": {"lte": 100}})).unwrap();
        assert_eq!(
            w,
            Where::Field {
                path: path("score"),
                op: Op::Lte,
                value: json!(100)
            }
        );
    }

    #[test]
    fn parses_comparison_gt() {
        let w = parse(&json!({"score": {"gt": 0}})).unwrap();
        assert_eq!(
            w,
            Where::Field {
                path: path("score"),
                op: Op::Gt,
                value: json!(0)
            }
        );
    }

    // ── Date values ───────────────────────────────────────────────────────

    #[test]
    fn parses_date_relative() {
        // Raw string is preserved; translator converts later.
        let w = parse(&json!({"created": {"gte": "6 months ago"}})).unwrap();
        assert_eq!(
            w,
            Where::Field {
                path: path("created"),
                op: Op::Gte,
                value: json!("6 months ago"),
            }
        );
    }

    #[test]
    fn parses_date_iso() {
        let w = parse(&json!({"created": {"gte": "2026-01-01T00:00:00Z"}})).unwrap();
        assert_eq!(
            w,
            Where::Field {
                path: path("created"),
                op: Op::Gte,
                value: json!("2026-01-01T00:00:00Z"),
            }
        );
    }

    // ── Like ─────────────────────────────────────────────────────────────

    #[test]
    fn parses_like() {
        let w = parse(&json!({"name": {"like": "iXX-%"}})).unwrap();
        assert_eq!(
            w,
            Where::Field {
                path: path("name"),
                op: Op::Like,
                value: json!("iXX-%"),
            }
        );
    }

    // ── Negation ──────────────────────────────────────────────────────────

    #[test]
    fn parses_negation() {
        let w = parse(&json!({"status": {"not": "Done"}})).unwrap();
        assert_eq!(
            w,
            Where::Not(Box::new(Where::Field {
                path: path("status"),
                op: Op::Eq,
                value: json!("Done"),
            }))
        );
    }

    // ── Nested field path ─────────────────────────────────────────────────

    #[test]
    fn parses_nested_path() {
        let w = parse(&json!({"assignee.email": "alice@example.com"})).unwrap();
        let expected_path = FieldPath {
            segments: vec![
                FieldSegment {
                    name: "assignee".into(),
                    array_projection: false,
                },
                FieldSegment {
                    name: "email".into(),
                    array_projection: false,
                },
            ],
        };
        assert_eq!(
            w,
            Where::Field {
                path: expected_path,
                op: Op::Eq,
                value: json!("alice@example.com"),
            }
        );
    }

    // ── Array projection ─────────────────────────────────────────────────

    #[test]
    fn parses_array_projection() {
        let w = parse(&json!({"fix_versions[].name": "iXX-2.7.0"})).unwrap();
        let expected_path = FieldPath {
            segments: vec![
                FieldSegment {
                    name: "fix_versions".into(),
                    array_projection: true,
                },
                FieldSegment {
                    name: "name".into(),
                    array_projection: false,
                },
            ],
        };
        assert_eq!(
            w,
            Where::Field {
                path: expected_path,
                op: Op::Eq,
                value: json!("iXX-2.7.0"),
            }
        );
    }

    // ── Array of strings membership ───────────────────────────────────────

    #[test]
    fn parses_array_membership_for_string() {
        // Equality on a string-array field: translator generates ARRAY_CONTAINS / any().
        let w = parse(&json!({"labels": "blocker"})).unwrap();
        assert_eq!(
            w,
            Where::Field {
                path: path("labels"),
                op: Op::Eq,
                value: json!("blocker"),
            }
        );
    }

    // ── Boolean combinations ──────────────────────────────────────────────

    #[test]
    fn parses_and_combination() {
        let w = parse(&json!({
            "and": [
                {"status": "Open"},
                {"assignee.email": "alice@example.com"}
            ]
        }))
        .unwrap();

        assert_eq!(
            w,
            Where::And(vec![
                Where::Field {
                    path: path("status"),
                    op: Op::Eq,
                    value: json!("Open"),
                },
                Where::Field {
                    path: path("assignee.email"),
                    op: Op::Eq,
                    value: json!("alice@example.com"),
                },
            ])
        );
    }

    #[test]
    fn parses_or_combination() {
        let w = parse(&json!({
            "or": [
                {"status": "Open"},
                {"status": "In Progress"}
            ]
        }))
        .unwrap();

        assert_eq!(
            w,
            Where::Or(vec![
                Where::Field {
                    path: path("status"),
                    op: Op::Eq,
                    value: json!("Open"),
                },
                Where::Field {
                    path: path("status"),
                    op: Op::Eq,
                    value: json!("In Progress"),
                },
            ])
        );
    }

    // ── Implicit multi-key And ────────────────────────────────────────────

    #[test]
    fn parses_implicit_and_from_multiple_keys() {
        // {"status":"Open","priority":"High"} → And([…])
        let w = parse(&json!({"priority": "High", "status": "Open"})).unwrap();
        match w {
            Where::And(children) => assert_eq!(children.len(), 2),
            other => panic!("expected And, got {other:?}"),
        }
    }

    // ── Exists ────────────────────────────────────────────────────────────

    #[test]
    fn parses_exists_true() {
        let w = parse(&json!({"fix_versions": {"exists": true}})).unwrap();
        assert_eq!(
            w,
            Where::Exists {
                path: path("fix_versions"),
                present: true
            }
        );
    }

    #[test]
    fn parses_exists_false() {
        let w = parse(&json!({"fix_versions": {"exists": false}})).unwrap();
        assert_eq!(
            w,
            Where::Exists {
                path: path("fix_versions"),
                present: false
            }
        );
    }

    // ── ArrayMatch ────────────────────────────────────────────────────────

    #[test]
    fn parses_array_match() {
        let w = parse(&json!({
            "issuelinks": {
                "array_match": {
                    "type": "blocked-by",
                    "target_key": "DO-1170"
                }
            }
        }))
        .unwrap();

        match w {
            Where::ArrayMatch { path, predicate } => {
                assert_eq!(path, FieldPath::parse("issuelinks"));
                match *predicate {
                    Where::And(children) => {
                        assert_eq!(children.len(), 2);
                    }
                    other => panic!("expected And predicate, got {other:?}"),
                }
            }
            other => panic!("expected ArrayMatch, got {other:?}"),
        }
    }

    // ── Error cases ───────────────────────────────────────────────────────

    #[test]
    fn rejects_non_object() {
        let err = parse(&json!("just a string")).unwrap_err();
        assert!(matches!(err, FilterError::Invalid(_)));
    }

    #[test]
    fn rejects_empty_object() {
        let err = parse(&json!({})).unwrap_err();
        assert!(matches!(err, FilterError::Invalid(_)));
    }

    #[test]
    fn rejects_unknown_operator() {
        let err = parse(&json!({"field": {"unknown_op": "value"}})).unwrap_err();
        assert!(matches!(err, FilterError::Unsupported(_)));
    }

    #[test]
    fn rejects_and_with_sibling_keys() {
        let err = parse(&json!({"and": [], "status": "Open"})).unwrap_err();
        assert!(matches!(err, FilterError::Invalid(_)));
    }
}
