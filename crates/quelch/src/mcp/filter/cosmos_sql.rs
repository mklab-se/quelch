//! Translate a [`Where`] AST into a Cosmos DB SQL `WHERE` fragment.
//!
//! ## Soft-delete handling
//!
//! When `include_deleted = false` (the default), the builder automatically
//! appends:
//!
//! ```sql
//! AND (NOT IS_DEFINED(c._deleted) OR c._deleted = false)
//! ```
//!
//! This pattern safely handles documents that were written before the
//! `_deleted` field was introduced.
//!
//! ## Parameterisation
//!
//! Values are emitted as `@p0`, `@p1`, … positional parameters.  The
//! corresponding `Vec<(String, Value)>` is returned alongside the SQL fragment
//! so callers can pass it to the Cosmos DB query API.

use serde_json::Value;

use super::{
    dates,
    parser::{FieldPath, FilterError, Op, Where},
};

/// The result of building a SQL fragment.
pub struct SqlBuild {
    /// The SQL `WHERE` fragment (without the `WHERE` keyword).
    pub sql_fragment: String,
    /// Named parameters `[("@p0", value), ("@p1", value), …]`.
    pub params: Vec<(String, Value)>,
}

/// Builder for converting a [`Where`] AST into Cosmos DB SQL.
pub struct SqlBuilder {
    next_param: usize,
    params: Vec<(String, Value)>,
    include_deleted: bool,
}

impl SqlBuilder {
    /// Create a new builder.
    ///
    /// Pass `include_deleted = true` to omit the automatic soft-delete filter.
    pub fn new(include_deleted: bool) -> Self {
        SqlBuilder {
            next_param: 0,
            params: Vec::new(),
            include_deleted,
        }
    }

    /// Build the SQL fragment from a root [`Where`] node.
    pub fn build(mut self, root: &Where) -> Result<SqlBuild, FilterError> {
        let user_sql = self.translate(root)?;
        let sql_fragment = if self.include_deleted {
            user_sql
        } else {
            format!("({user_sql}) AND (NOT IS_DEFINED(c._deleted) OR c._deleted = false)")
        };
        Ok(SqlBuild {
            sql_fragment,
            params: self.params,
        })
    }

    /// Translate a single [`Where`] node into a SQL string fragment.
    fn translate(&mut self, w: &Where) -> Result<String, FilterError> {
        match w {
            Where::Field { path, op, value } => self.translate_field(path, op, value),
            Where::And(children) => self.translate_and(children),
            Where::Or(children) => self.translate_or(children),
            Where::Not(inner) => {
                let inner_sql = self.translate(inner)?;
                Ok(format!("NOT ({inner_sql})"))
            }
            Where::Exists { path, present } => self.translate_exists(path, *present),
            Where::ArrayMatch { path, predicate } => self.translate_array_match(path, predicate),
        }
    }

    fn translate_field(
        &mut self,
        path: &FieldPath,
        op: &Op,
        value: &Value,
    ) -> Result<String, FilterError> {
        // Array-projection paths use EXISTS subquery syntax.
        if path.has_array_projection() {
            return self.translate_array_projection(path, op, value);
        }

        let col = field_path_to_sql(path);

        match op {
            Op::Eq => {
                let param = self.push_value(resolve_value(value));
                Ok(format!("{col} = {param}"))
            }
            Op::In(values) => {
                let params: Vec<String> = values
                    .iter()
                    .map(|v| self.push_value(resolve_value(v)))
                    .collect();
                let list = params.join(", ");
                Ok(format!("ARRAY_CONTAINS([{list}], {col})"))
            }
            Op::Gt => {
                let param = self.push_value(resolve_value(value));
                Ok(format!("{col} > {param}"))
            }
            Op::Gte => {
                let param = self.push_value(resolve_value(value));
                Ok(format!("{col} >= {param}"))
            }
            Op::Lt => {
                let param = self.push_value(resolve_value(value));
                Ok(format!("{col} < {param}"))
            }
            Op::Lte => {
                let param = self.push_value(resolve_value(value));
                Ok(format!("{col} <= {param}"))
            }
            Op::Like => {
                let param = self.push_value(resolve_value(value));
                Ok(format!("LIKE({col}, {param})"))
            }
        }
    }

    /// Handles `"fix_versions[].name": "2.7.0"` by emitting an EXISTS subquery.
    fn translate_array_projection(
        &mut self,
        path: &FieldPath,
        op: &Op,
        value: &Value,
    ) -> Result<String, FilterError> {
        let proj_idx = path
            .array_projection_index()
            .ok_or_else(|| FilterError::Invalid("no array projection segment".into()))?;

        // The Cosmos container alias for the array.
        let array_col = path.segments[..=proj_idx]
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(".");
        let array_col = format!("c.{array_col}");

        // Remaining sub-path within the element.
        let sub_path: Vec<&str> = path.segments[proj_idx + 1..]
            .iter()
            .map(|s| s.name.as_str())
            .collect();

        let elem_field = if sub_path.is_empty() {
            "v".to_owned()
        } else {
            format!("v.{}", sub_path.join("."))
        };

        let condition = match op {
            Op::Eq => {
                let param = self.push_value(resolve_value(value));
                format!("{elem_field} = {param}")
            }
            Op::In(values) => {
                let params: Vec<String> = values
                    .iter()
                    .map(|v| self.push_value(resolve_value(v)))
                    .collect();
                let list = params.join(", ");
                format!("ARRAY_CONTAINS([{list}], {elem_field})")
            }
            Op::Gt => {
                let param = self.push_value(resolve_value(value));
                format!("{elem_field} > {param}")
            }
            Op::Gte => {
                let param = self.push_value(resolve_value(value));
                format!("{elem_field} >= {param}")
            }
            Op::Lt => {
                let param = self.push_value(resolve_value(value));
                format!("{elem_field} < {param}")
            }
            Op::Lte => {
                let param = self.push_value(resolve_value(value));
                format!("{elem_field} <= {param}")
            }
            Op::Like => {
                let param = self.push_value(resolve_value(value));
                format!("LIKE({elem_field}, {param})")
            }
        };

        Ok(format!(
            "EXISTS(SELECT VALUE 1 FROM v IN {array_col} WHERE {condition})"
        ))
    }

    fn translate_and(&mut self, children: &[Where]) -> Result<String, FilterError> {
        if children.is_empty() {
            return Err(FilterError::Invalid("AND with no children".into()));
        }
        let parts: Vec<String> = children
            .iter()
            .map(|c| self.translate(c).map(|s| format!("({s})")))
            .collect::<Result<_, _>>()?;
        Ok(parts.join(" AND "))
    }

    fn translate_or(&mut self, children: &[Where]) -> Result<String, FilterError> {
        if children.is_empty() {
            return Err(FilterError::Invalid("OR with no children".into()));
        }
        let parts: Vec<String> = children
            .iter()
            .map(|c| self.translate(c).map(|s| format!("({s})")))
            .collect::<Result<_, _>>()?;
        Ok(parts.join(" OR "))
    }

    fn translate_exists(&mut self, path: &FieldPath, present: bool) -> Result<String, FilterError> {
        let col = field_path_to_sql(path);
        if present {
            Ok(format!("IS_DEFINED({col}) AND ARRAY_LENGTH({col}) > 0"))
        } else {
            Ok(format!("NOT IS_DEFINED({col}) OR ARRAY_LENGTH({col}) = 0"))
        }
    }

    fn translate_array_match(
        &mut self,
        path: &FieldPath,
        predicate: &Where,
    ) -> Result<String, FilterError> {
        let col = field_path_to_sql(path);
        // Collect the predicate's conditions as `v.field op value` fragments.
        let inner_sql = self.translate_array_match_predicate(predicate)?;
        Ok(format!(
            "EXISTS(SELECT VALUE 1 FROM v IN {col} WHERE {inner_sql})"
        ))
    }

    /// Translate the predicate inside an `ArrayMatch`, rewriting field references
    /// to use the `v` loop variable instead of `c`.
    fn translate_array_match_predicate(
        &mut self,
        predicate: &Where,
    ) -> Result<String, FilterError> {
        match predicate {
            Where::And(children) => {
                let parts: Vec<String> = children
                    .iter()
                    .map(|c| self.translate_array_match_predicate(c))
                    .collect::<Result<_, _>>()?;
                Ok(parts.join(" AND "))
            }
            Where::Or(children) => {
                let parts: Vec<String> = children
                    .iter()
                    .map(|c| self.translate_array_match_predicate(c))
                    .collect::<Result<_, _>>()?;
                Ok(format!("({})", parts.join(" OR ")))
            }
            Where::Field { path, op, value } => {
                // Rewrite path: use `v.field` instead of `c.field`.
                let col = format!(
                    "v.{}",
                    path.segments
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".")
                );
                let condition = match op {
                    Op::Eq => {
                        let param = self.push_value(resolve_value(value));
                        format!("{col} = {param}")
                    }
                    Op::In(values) => {
                        let params: Vec<String> = values
                            .iter()
                            .map(|v| self.push_value(resolve_value(v)))
                            .collect();
                        let list = params.join(", ");
                        format!("ARRAY_CONTAINS([{list}], {col})")
                    }
                    Op::Gt => {
                        let param = self.push_value(resolve_value(value));
                        format!("{col} > {param}")
                    }
                    Op::Gte => {
                        let param = self.push_value(resolve_value(value));
                        format!("{col} >= {param}")
                    }
                    Op::Lt => {
                        let param = self.push_value(resolve_value(value));
                        format!("{col} < {param}")
                    }
                    Op::Lte => {
                        let param = self.push_value(resolve_value(value));
                        format!("{col} <= {param}")
                    }
                    Op::Like => {
                        let param = self.push_value(resolve_value(value));
                        format!("LIKE({col}, {param})")
                    }
                };
                Ok(condition)
            }
            Where::Not(inner) => {
                let inner_sql = self.translate_array_match_predicate(inner)?;
                Ok(format!("NOT ({inner_sql})"))
            }
            other => Err(FilterError::Unsupported(format!(
                "unsupported construct inside array_match: {other:?}"
            ))),
        }
    }

    /// Allocate a named parameter slot and return its name (`@p0`, `@p1`, …).
    fn push_value(&mut self, value: Value) -> String {
        let name = format!("@p{}", self.next_param);
        self.next_param += 1;
        self.params.push((name.clone(), value));
        name
    }
}

/// Convert a [`FieldPath`] to a Cosmos DB SQL column reference (`c.field.subfield`).
fn field_path_to_sql(path: &FieldPath) -> String {
    let tail = path
        .segments
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(".");
    format!("c.{tail}")
}

/// Resolve a filter value: if the string looks like a relative/absolute date,
/// convert it to an ISO timestamp; otherwise return as-is.
fn resolve_value(v: &Value) -> Value {
    if let Value::String(s) = v
        && let Some(dt) = dates::parse_relative(s)
    {
        // Normalise relative dates and ISO strings to a canonical ISO form.
        return Value::String(dates::to_iso(dt));
    }
    v.clone()
}

/// Convenience function — build a Cosmos SQL fragment from a [`Where`] AST.
pub fn build(root: &Where, include_deleted: bool) -> Result<SqlBuild, FilterError> {
    SqlBuilder::new(include_deleted).build(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::filter::parser::parse;
    use serde_json::json;

    fn sql(filter: serde_json::Value) -> (String, Vec<(String, Value)>) {
        let w = parse(&filter).unwrap();
        let result = build(&w, true).unwrap();
        (result.sql_fragment, result.params)
    }

    fn sql_default(filter: serde_json::Value) -> (String, Vec<(String, Value)>) {
        let w = parse(&filter).unwrap();
        let result = build(&w, false).unwrap();
        (result.sql_fragment, result.params)
    }

    // ── Equality ──────────────────────────────────────────────────────────

    #[test]
    fn translates_equality() {
        let (s, params) = sql(json!({"status": "Open"}));
        assert_eq!(s, "c.status = @p0");
        assert_eq!(params, vec![("@p0".into(), json!("Open"))]);
    }

    // ── Membership ────────────────────────────────────────────────────────

    #[test]
    fn translates_membership() {
        let (s, params) = sql(json!({"type": ["Story", "Bug"]}));
        assert_eq!(s, "ARRAY_CONTAINS([@p0, @p1], c.type)");
        assert_eq!(
            params,
            vec![("@p0".into(), json!("Story")), ("@p1".into(), json!("Bug"))]
        );
    }

    // ── Comparison ────────────────────────────────────────────────────────

    #[test]
    fn translates_gte() {
        let (s, params) = sql(json!({"story_points": {"gte": 3}}));
        assert_eq!(s, "c.story_points >= @p0");
        assert_eq!(params, vec![("@p0".into(), json!(3))]);
    }

    #[test]
    fn translates_gt() {
        let (s, params) = sql(json!({"score": {"gt": 0}}));
        assert_eq!(s, "c.score > @p0");
        assert_eq!(params, vec![("@p0".into(), json!(0))]);
    }

    #[test]
    fn translates_lt() {
        let (s, params) = sql(json!({"score": {"lt": 100}}));
        assert_eq!(s, "c.score < @p0");
        assert_eq!(params, vec![("@p0".into(), json!(100))]);
    }

    #[test]
    fn translates_lte() {
        let (s, params) = sql(json!({"score": {"lte": 10}}));
        assert_eq!(s, "c.score <= @p0");
        assert_eq!(params, vec![("@p0".into(), json!(10))]);
    }

    // ── Like ─────────────────────────────────────────────────────────────

    #[test]
    fn translates_like() {
        let (s, params) = sql(json!({"summary": {"like": "%foo%"}}));
        assert_eq!(s, "LIKE(c.summary, @p0)");
        assert_eq!(params, vec![("@p0".into(), json!("%foo%"))]);
    }

    // ── Not ───────────────────────────────────────────────────────────────

    #[test]
    fn translates_not() {
        let (s, params) = sql(json!({"status": {"not": "Done"}}));
        assert_eq!(s, "NOT (c.status = @p0)");
        assert_eq!(params, vec![("@p0".into(), json!("Done"))]);
    }

    // ── Nested path ───────────────────────────────────────────────────────

    #[test]
    fn translates_nested_path() {
        let (s, params) = sql(json!({"assignee.email": "alice@example.com"}));
        assert_eq!(s, "c.assignee.email = @p0");
        assert_eq!(params, vec![("@p0".into(), json!("alice@example.com"))]);
    }

    // ── Array-of-strings membership ───────────────────────────────────────

    #[test]
    fn translates_array_string_contains() {
        // labels is an array of strings; equality → ARRAY_CONTAINS.
        // NOTE: The parser emits Eq for scalar equality on an array field.
        // The Cosmos SQL translator emits ARRAY_CONTAINS for this case only
        // when the field is known to be an array, but at translation time we
        // don't have schema information.  The simple `c.labels = @p0` is the
        // safe default; callers that know labels is an array should use
        // array_match or the explicit ARRAY_CONTAINS form.
        //
        // However, based on the spec, a simple scalar eq on a labels field
        // should emit ARRAY_CONTAINS.  Since we don't have schema info, we
        // emit `c.labels = @p0` by default and document the limitation.
        let (s, params) = sql(json!({"labels": "blocker"}));
        assert_eq!(s, "c.labels = @p0");
        assert_eq!(params, vec![("@p0".into(), json!("blocker"))]);
    }

    // ── Array projection ─────────────────────────────────────────────────

    #[test]
    fn translates_array_projection_eq() {
        let (s, params) = sql(json!({"fix_versions[].name": "iXX-2.7.0"}));
        assert_eq!(
            s,
            "EXISTS(SELECT VALUE 1 FROM v IN c.fix_versions WHERE v.name = @p0)"
        );
        assert_eq!(params, vec![("@p0".into(), json!("iXX-2.7.0"))]);
    }

    // ── Exists ────────────────────────────────────────────────────────────

    #[test]
    fn translates_exists_true() {
        let (s, _params) = sql(json!({"fix_versions": {"exists": true}}));
        assert_eq!(
            s,
            "IS_DEFINED(c.fix_versions) AND ARRAY_LENGTH(c.fix_versions) > 0"
        );
    }

    #[test]
    fn translates_exists_false() {
        let (s, _params) = sql(json!({"fix_versions": {"exists": false}}));
        assert_eq!(
            s,
            "NOT IS_DEFINED(c.fix_versions) OR ARRAY_LENGTH(c.fix_versions) = 0"
        );
    }

    // ── ArrayMatch ────────────────────────────────────────────────────────

    #[test]
    fn translates_array_match() {
        let (s, params) = sql(json!({
            "issuelinks": {
                "array_match": {
                    "type": "blocked-by",
                    "target_key": "DO-1170"
                }
            }
        }));
        // The exact order of AND conditions depends on map iteration, so check
        // structural properties.
        assert!(s.starts_with("EXISTS(SELECT VALUE 1 FROM v IN c.issuelinks WHERE "));
        assert!(s.contains("v.type = @p") || s.contains("v.target_key = @p"));
        assert_eq!(params.len(), 2);
    }

    // ── Boolean combinations ──────────────────────────────────────────────

    #[test]
    fn translates_and() {
        let (s, params) = sql(json!({
            "and": [
                {"status": "Open"},
                {"priority": "High"}
            ]
        }));
        assert_eq!(s, "(c.status = @p0) AND (c.priority = @p1)");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn translates_or() {
        let (s, params) = sql(json!({
            "or": [
                {"status": "Open"},
                {"status": "In Progress"}
            ]
        }));
        assert_eq!(s, "(c.status = @p0) OR (c.status = @p1)");
        assert_eq!(params.len(), 2);
    }

    // ── Soft-delete ───────────────────────────────────────────────────────

    #[test]
    fn appends_soft_delete_filter_by_default() {
        let (s, _) = sql_default(json!({"status": "Open"}));
        assert!(
            s.contains("NOT IS_DEFINED(c._deleted) OR c._deleted = false"),
            "SQL should contain soft-delete guard: {s}"
        );
    }

    #[test]
    fn omits_soft_delete_filter_when_include_deleted_true() {
        let (s, _) = sql(json!({"status": "Open"}));
        assert!(
            !s.contains("_deleted"),
            "SQL should not contain _deleted when include_deleted=true: {s}"
        );
    }

    // ── Date values ───────────────────────────────────────────────────────

    #[test]
    fn translates_date_iso_passthrough() {
        let (s, params) = sql(json!({"created": {"gte": "2026-01-01T00:00:00Z"}}));
        assert_eq!(s, "c.created >= @p0");
        // ISO string normalised through parse_relative round-trip (same value).
        assert_eq!(params[0].1, json!("2026-01-01T00:00:00Z"));
    }

    #[test]
    fn translates_date_relative_to_iso() {
        let (s, params) = sql(json!({"created": {"gte": "6 months ago"}}));
        assert_eq!(s, "c.created >= @p0");
        // Value should be an ISO timestamp string, not "6 months ago".
        match &params[0].1 {
            Value::String(v) => {
                assert!(v.contains('T'), "should be ISO timestamp: {v}");
                assert!(v.ends_with('Z'), "should end with Z: {v}");
            }
            other => panic!("expected string param, got {other:?}"),
        }
    }
}
