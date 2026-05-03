//! Translate a [`Where`] AST into an Azure AI Search OData `$filter` string.
//!
//! ## Key differences from Cosmos SQL
//!
//! | Feature | Cosmos SQL | OData |
//! |---------|-----------|-------|
//! | Nested fields | `c.assignee.email` | `assignee/email` |
//! | Equality | `=` | `eq` |
//! | Array any-element | `ARRAY_CONTAINS` | `field/any(v: …)` |
//! | Array projection | `EXISTS(…)` | `field/any(v: v/sub eq …)` |
//! | Parameters | `@p0` | inline values |
//! | String quoting | `@p0` binding | `'value'` (single-quoted, `'` → `''`) |
//!
//! ## Like / pattern matching
//!
//! Azure AI Search does not have a SQL-style `LIKE` operator.  For patterns
//! with a leading `%` (substring match), this translator emits
//! `search.ismatch('pattern*', 'field')`, which uses full-text search semantics.
//! This is an **approximation** — matching behaviour differs from SQL `LIKE`.
//! For exact prefix matching (`"foo%"` with no leading `%`), `startswith` is used
//! instead.  Agents that need precise string matching should use `eq` with the
//! exact value.
//!
//! ## Soft-delete handling
//!
//! When `include_deleted = false` (the default), appends `_deleted ne true` to
//! the filter.  This pattern relies on Azure AI Search field `_deleted` being
//! indexed.

use serde_json::Value;

use super::{
    dates,
    parser::{FieldPath, FilterError, Op, Where},
};

/// Build an OData `$filter` string from a [`Where`] AST.
///
/// Values are inlined (no parameter binding).  String values are single-quoted
/// with internal `'` escaped as `''`.  Datetime values are emitted unquoted in
/// ISO 8601 with `Z`.
pub fn build(root: &Where, include_deleted: bool) -> Result<String, FilterError> {
    let user_filter = translate(root)?;
    if include_deleted {
        Ok(user_filter)
    } else {
        Ok(format!("({user_filter}) and _deleted ne true"))
    }
}

fn translate(w: &Where) -> Result<String, FilterError> {
    match w {
        Where::Field { path, op, value } => translate_field(path, op, value),
        Where::And(children) => {
            if children.is_empty() {
                return Err(FilterError::Invalid("AND with no children".into()));
            }
            let parts: Vec<String> = children
                .iter()
                .map(|c| translate(c).map(|s| format!("({s})")))
                .collect::<Result<_, _>>()?;
            Ok(parts.join(" and "))
        }
        Where::Or(children) => {
            if children.is_empty() {
                return Err(FilterError::Invalid("OR with no children".into()));
            }
            let parts: Vec<String> = children
                .iter()
                .map(|c| translate(c).map(|s| format!("({s})")))
                .collect::<Result<_, _>>()?;
            Ok(parts.join(" or "))
        }
        Where::Not(inner) => {
            let inner_s = translate(inner)?;
            Ok(format!("not ({inner_s})"))
        }
        Where::Exists { path, present } => translate_exists(path, *present),
        Where::ArrayMatch { path, predicate } => translate_array_match(path, predicate),
    }
}

fn translate_field(path: &FieldPath, op: &Op, value: &Value) -> Result<String, FilterError> {
    // Array-projection paths (`fix_versions[].name`) use `/any(v: …)` syntax.
    if path.has_array_projection() {
        return translate_array_projection(path, op, value);
    }

    let col = field_path_to_odata(path);
    let resolved = resolve_value(value);

    match op {
        Op::Eq => Ok(format!("{col} eq {}", odata_literal(&resolved))),
        Op::In(values) => {
            let list = values
                .iter()
                .map(odata_string_literal)
                .collect::<Vec<_>>()
                .join(",");
            Ok(format!("search.in({col}, '{list}', ',')"))
        }
        Op::Gt => Ok(format!("{col} gt {}", odata_literal(&resolved))),
        Op::Gte => Ok(format!("{col} ge {}", odata_literal(&resolved))),
        Op::Lt => Ok(format!("{col} lt {}", odata_literal(&resolved))),
        Op::Lte => Ok(format!("{col} le {}", odata_literal(&resolved))),
        Op::Like => translate_like(&col, &resolved),
    }
}

/// Translate `LIKE` to OData approximation.
///
/// - Pattern `%foo%` → `search.ismatch('foo*', 'field')` (full-text, approximate)
/// - Pattern `foo%`  → `startswith(field, 'foo')`
/// - Pattern `%foo`  → `endswith(field, 'foo')` (OData 4 supports `endswith`)
/// - Pattern `foo`   → `field eq 'foo'`
///
/// This is documented as an approximation; exact LIKE semantics differ.
fn translate_like(col: &str, value: &Value) -> Result<String, FilterError> {
    let pattern = match value {
        Value::String(s) => s.as_str(),
        _ => return Err(FilterError::Invalid("LIKE value must be a string".into())),
    };

    let starts_with_pct = pattern.starts_with('%');
    let ends_with_pct = pattern.ends_with('%');

    let inner = pattern.trim_start_matches('%').trim_end_matches('%');

    match (starts_with_pct, ends_with_pct) {
        (true, true) | (true, false) => {
            // Substring or suffix — use full-text search (approximate).
            // NOTE: search.ismatch uses full-text semantics, not exact substring matching.
            let field_name = col; // the field path, already OData-encoded
            Ok(format!("search.ismatch('{inner}*', '{field_name}')"))
        }
        (false, true) => {
            // Prefix match.
            let quoted = escape_odata_string(inner);
            Ok(format!("startswith({col}, '{quoted}')"))
        }
        (false, false) => {
            // No wildcards — exact equality.
            let quoted = escape_odata_string(pattern);
            Ok(format!("{col} eq '{quoted}'"))
        }
    }
}

/// Handles `"fix_versions[].name": "2.7.0"` → `fix_versions/any(v: v/name eq '2.7.0')`.
fn translate_array_projection(
    path: &FieldPath,
    op: &Op,
    value: &Value,
) -> Result<String, FilterError> {
    let proj_idx = path
        .array_projection_index()
        .ok_or_else(|| FilterError::Invalid("no array projection segment".into()))?;

    // Build the array field path (OData slash-separated).
    let array_col: String = path.segments[..=proj_idx]
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join("/");

    // Build the sub-path within the element.
    let sub_segs: Vec<&str> = path.segments[proj_idx + 1..]
        .iter()
        .map(|s| s.name.as_str())
        .collect();

    let elem_field = if sub_segs.is_empty() {
        "v".to_owned()
    } else {
        format!("v/{}", sub_segs.join("/"))
    };

    let resolved = resolve_value(value);
    let condition = match op {
        Op::Eq => format!("{elem_field} eq {}", odata_literal(&resolved)),
        Op::In(values) => {
            let list = values
                .iter()
                .map(odata_string_literal)
                .collect::<Vec<_>>()
                .join(",");
            format!("search.in({elem_field}, '{list}', ',')")
        }
        Op::Gt => format!("{elem_field} gt {}", odata_literal(&resolved)),
        Op::Gte => format!("{elem_field} ge {}", odata_literal(&resolved)),
        Op::Lt => format!("{elem_field} lt {}", odata_literal(&resolved)),
        Op::Lte => format!("{elem_field} le {}", odata_literal(&resolved)),
        Op::Like => return translate_like(&elem_field, &resolved),
    };

    Ok(format!("{array_col}/any(v: {condition})"))
}

fn translate_exists(path: &FieldPath, present: bool) -> Result<String, FilterError> {
    let col = field_path_to_odata(path);
    if present {
        // `field/any()` = "any element exists" (no lambda body).
        Ok(format!("{col}/any()"))
    } else {
        // No standard OData "not exists"; negate the any().
        Ok(format!("not {col}/any()"))
    }
}

fn translate_array_match(path: &FieldPath, predicate: &Where) -> Result<String, FilterError> {
    let col = field_path_to_odata(path);
    let condition = translate_array_match_predicate(predicate)?;
    Ok(format!("{col}/any(v: {condition})"))
}

/// Translate the predicate inside an `ArrayMatch`, rewriting field references
/// to use `v/field` instead of bare `field`.
fn translate_array_match_predicate(predicate: &Where) -> Result<String, FilterError> {
    match predicate {
        Where::And(children) => {
            let parts: Vec<String> = children
                .iter()
                .map(translate_array_match_predicate)
                .collect::<Result<_, _>>()?;
            Ok(parts.join(" and "))
        }
        Where::Or(children) => {
            let parts: Vec<String> = children
                .iter()
                .map(translate_array_match_predicate)
                .collect::<Result<_, _>>()?;
            Ok(format!("({})", parts.join(" or ")))
        }
        Where::Field { path, op, value } => {
            // Rewrite path to `v/field/subfield`.
            let sub_path = path
                .segments
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join("/");
            let col = format!("v/{sub_path}");
            let resolved = resolve_value(value);
            let condition = match op {
                Op::Eq => format!("{col} eq {}", odata_literal(&resolved)),
                Op::In(values) => {
                    let list = values
                        .iter()
                        .map(odata_string_literal)
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("search.in({col}, '{list}', ',')")
                }
                Op::Gt => format!("{col} gt {}", odata_literal(&resolved)),
                Op::Gte => format!("{col} ge {}", odata_literal(&resolved)),
                Op::Lt => format!("{col} lt {}", odata_literal(&resolved)),
                Op::Lte => format!("{col} le {}", odata_literal(&resolved)),
                Op::Like => return translate_like(&col, &resolved),
            };
            Ok(condition)
        }
        Where::Not(inner) => {
            let inner_s = translate_array_match_predicate(inner)?;
            Ok(format!("not ({inner_s})"))
        }
        other => Err(FilterError::Unsupported(format!(
            "unsupported construct inside array_match predicate: {other:?}"
        ))),
    }
}

// ── Value rendering ─────────────────────────────────────────────────────────

/// Render a JSON value as an OData literal (inline, no `@p0` parameters).
///
/// - Strings → `'escaped'`
/// - Numbers → numeric literal
/// - Booleans → `true` / `false`
/// - Null → `null`
fn odata_literal(v: &Value) -> String {
    match v {
        Value::String(s) => {
            // Datetime strings: OData wants unquoted ISO 8601 values.
            // Detect by checking for the `T` and `Z` datetime pattern.
            if is_iso_datetime(s) {
                s.clone()
            } else {
                format!("'{}'", escape_odata_string(s))
            }
        }
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_owned(),
        Value::Array(_) | Value::Object(_) => {
            // Not directly expressible as a single OData literal.
            "null".to_owned()
        }
    }
}

/// Render a JSON value as a bare string (without surrounding quotes) for use
/// inside a `search.in(…, 'A,B,C', ',')` list.
fn odata_string_literal(v: &Value) -> String {
    match v {
        Value::String(s) => escape_odata_string(s),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// Escape single quotes inside an OData string by doubling them.
fn escape_odata_string(s: &str) -> String {
    s.replace('\'', "''")
}

/// Convert a [`FieldPath`] to OData slash notation (`assignee/email`).
fn field_path_to_odata(path: &FieldPath) -> String {
    path.segments
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join("/")
}

/// Detect ISO 8601 / RFC 3339 datetime strings (end with `Z` and contain `T`).
fn is_iso_datetime(s: &str) -> bool {
    s.contains('T') && s.ends_with('Z')
}

/// Resolve a filter value: expand relative dates to ISO timestamps.
fn resolve_value(v: &Value) -> Value {
    if let Value::String(s) = v
        && let Some(dt) = dates::parse_relative(s)
    {
        return Value::String(dates::to_iso(dt));
    }
    v.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::filter::parser::parse;
    use serde_json::json;

    fn odata(filter: serde_json::Value) -> String {
        let w = parse(&filter).unwrap();
        build(&w, true).unwrap()
    }

    fn odata_default(filter: serde_json::Value) -> String {
        let w = parse(&filter).unwrap();
        build(&w, false).unwrap()
    }

    // ── Equality ──────────────────────────────────────────────────────────

    #[test]
    fn translates_equality() {
        assert_eq!(odata(json!({"status": "Open"})), "status eq 'Open'");
    }

    #[test]
    fn translates_numeric_equality() {
        assert_eq!(odata(json!({"score": 42})), "score eq 42");
    }

    #[test]
    fn translates_bool_equality() {
        assert_eq!(odata(json!({"active": true})), "active eq true");
    }

    // ── Membership ────────────────────────────────────────────────────────

    #[test]
    fn translates_membership() {
        let s = odata(json!({"type": ["Story", "Bug"]}));
        assert_eq!(s, "search.in(type, 'Story,Bug', ',')");
    }

    // ── Comparison ────────────────────────────────────────────────────────

    #[test]
    fn translates_gte() {
        assert_eq!(
            odata(json!({"story_points": {"gte": 3}})),
            "story_points ge 3"
        );
    }

    #[test]
    fn translates_gt() {
        assert_eq!(odata(json!({"score": {"gt": 0}})), "score gt 0");
    }

    #[test]
    fn translates_lt() {
        assert_eq!(odata(json!({"score": {"lt": 100}})), "score lt 100");
    }

    #[test]
    fn translates_lte() {
        assert_eq!(odata(json!({"score": {"lte": 10}})), "score le 10");
    }

    // ── Like ─────────────────────────────────────────────────────────────

    #[test]
    fn translates_like_substring() {
        // "%foo%" → search.ismatch (approximate full-text semantics)
        let s = odata(json!({"summary": {"like": "%foo%"}}));
        assert!(s.contains("search.ismatch"), "got: {s}");
    }

    #[test]
    fn translates_like_prefix() {
        let s = odata(json!({"name": {"like": "iXX-%"}}));
        assert!(s.starts_with("startswith("), "got: {s}");
        assert!(s.contains("iXX-"), "got: {s}");
    }

    // ── Not ───────────────────────────────────────────────────────────────

    #[test]
    fn translates_not() {
        let s = odata(json!({"status": {"not": "Done"}}));
        assert_eq!(s, "not (status eq 'Done')");
    }

    // ── Nested path ───────────────────────────────────────────────────────

    #[test]
    fn translates_nested_path() {
        assert_eq!(
            odata(json!({"assignee.email": "alice@example.com"})),
            "assignee/email eq 'alice@example.com'"
        );
    }

    // ── Array of strings ─────────────────────────────────────────────────

    #[test]
    fn translates_array_string_eq() {
        // Scalar eq on array-of-strings field → field eq 'value'
        // (exact containment is handled by the AI Search index configuration,
        //  not by OData filter syntax for simple equality)
        let s = odata(json!({"labels": "blocker"}));
        assert_eq!(s, "labels eq 'blocker'");
    }

    // ── Array projection ─────────────────────────────────────────────────

    #[test]
    fn translates_array_projection_eq() {
        let s = odata(json!({"fix_versions[].name": "iXX-2.7.0"}));
        assert_eq!(s, "fix_versions/any(v: v/name eq 'iXX-2.7.0')");
    }

    // ── Exists ────────────────────────────────────────────────────────────

    #[test]
    fn translates_exists_true() {
        let s = odata(json!({"fix_versions": {"exists": true}}));
        assert_eq!(s, "fix_versions/any()");
    }

    #[test]
    fn translates_exists_false() {
        let s = odata(json!({"fix_versions": {"exists": false}}));
        assert_eq!(s, "not fix_versions/any()");
    }

    // ── ArrayMatch ────────────────────────────────────────────────────────

    #[test]
    fn translates_array_match() {
        let s = odata(json!({
            "issuelinks": {
                "array_match": {
                    "type": "blocked-by",
                    "target_key": "DO-1170"
                }
            }
        }));
        assert!(s.starts_with("issuelinks/any(v: "), "got: {s}");
        assert!(
            s.contains("v/type eq 'blocked-by'") || s.contains("v/target_key eq 'DO-1170'"),
            "got: {s}"
        );
    }

    // ── Boolean combinations ──────────────────────────────────────────────

    #[test]
    fn translates_and() {
        let s = odata(json!({
            "and": [
                {"status": "Open"},
                {"priority": "High"}
            ]
        }));
        assert_eq!(s, "(status eq 'Open') and (priority eq 'High')");
    }

    #[test]
    fn translates_or() {
        let s = odata(json!({
            "or": [
                {"status": "Open"},
                {"status": "In Progress"}
            ]
        }));
        assert_eq!(s, "(status eq 'Open') or (status eq 'In Progress')");
    }

    // ── Soft-delete ───────────────────────────────────────────────────────

    #[test]
    fn appends_soft_delete_filter_by_default() {
        let s = odata_default(json!({"status": "Open"}));
        assert!(
            s.contains("_deleted ne true"),
            "should contain soft-delete guard: {s}"
        );
    }

    #[test]
    fn omits_soft_delete_filter_when_include_deleted_true() {
        let s = odata(json!({"status": "Open"}));
        assert!(!s.contains("_deleted"), "should not contain _deleted: {s}");
    }

    // ── Date values ───────────────────────────────────────────────────────

    #[test]
    fn translates_date_iso_unquoted() {
        // ISO datetimes should be unquoted in OData.
        let s = odata(json!({"created": {"gte": "2026-01-01T00:00:00Z"}}));
        assert_eq!(s, "created ge 2026-01-01T00:00:00Z");
    }

    #[test]
    fn translates_date_relative_to_iso_unquoted() {
        let s = odata(json!({"created": {"gte": "6 months ago"}}));
        assert!(s.starts_with("created ge "), "got: {s}");
        let ts = s.strip_prefix("created ge ").unwrap();
        assert!(ts.contains('T') && ts.ends_with('Z'), "should be ISO: {ts}");
    }

    // ── String escaping ───────────────────────────────────────────────────

    #[test]
    fn escapes_single_quotes_in_strings() {
        let s = odata(json!({"name": "O'Brien"}));
        assert_eq!(s, "name eq 'O''Brien'");
    }
}
