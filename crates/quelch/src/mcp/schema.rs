//! Static schema catalog for MCP-exposed entity kinds.
//!
//! The catalog is constructed once at server startup and consulted by the
//! `list_sources` tool (to describe each data source to agents) and by the
//! `aggregate` tool (to detect array fields for `GROUP BY` fan-out).
//!
//! # Adding a new kind
//!
//! Add a `KindInfo` entry in [`SchemaCatalog::default`] and list all fields
//! in `fields`.  Mark array fields with `type: "string[]"` (or the appropriate
//! element type + `[]`).  This is what `aggregate` uses to decide whether to
//! emit `JOIN … IN c.<field>` vs. a plain `GROUP BY`.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

/// Schema information for a single entity kind.
#[derive(Debug, Clone)]
pub struct KindInfo {
    /// Human-readable description of this entity kind.
    pub description: String,
    /// Whether this kind supports the `search` tool (i.e., has an AI Search index).
    pub searchable: bool,
    /// All fields available on documents of this kind.
    pub fields: Vec<FieldInfo>,
    /// Example `query` / `aggregate` calls for agents.
    pub examples: Vec<ExampleCall>,
    /// Set of field names that are array-typed (used by `aggregate`).
    pub array_fields: HashSet<String>,
}

/// Schema information for a single field.
#[derive(Debug, Clone, Serialize)]
pub struct FieldInfo {
    /// Dot-path field name (e.g. `assignee.email`, `fix_versions[].name`).
    pub field: String,
    /// Type string: `"string"`, `"integer"`, `"float"`, `"boolean"`, `"datetime"`,
    /// `"string[]"`, `"object[]"`, etc.
    #[serde(rename = "type")]
    pub r#type: String,
    /// Allowed values, if this is an enum-typed field.
    #[serde(rename = "enum", skip_serializing_if = "Option::is_none")]
    pub r#enum: Option<Vec<String>>,
    /// Short description of the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A documented example MCP call for a data source.
#[derive(Debug, Clone, Serialize)]
pub struct ExampleCall {
    /// Plain-English description of what the example shows.
    pub description: String,
    /// The JSON call (pretty-printed or compact).
    pub call: String,
}

/// Registry of all known entity kinds.
pub struct SchemaCatalog {
    kinds: HashMap<String, KindInfo>,
}

impl SchemaCatalog {
    /// Construct the catalog with all built-in kinds.
    pub fn new() -> Self {
        let mut kinds = HashMap::new();
        kinds.insert("jira_issue".to_string(), jira_issue());
        kinds.insert("jira_sprint".to_string(), jira_sprint());
        kinds.insert("jira_fix_version".to_string(), jira_fix_version());
        kinds.insert("jira_project".to_string(), jira_project());
        kinds.insert("confluence_page".to_string(), confluence_page());
        kinds.insert("confluence_space".to_string(), confluence_space());
        Self { kinds }
    }

    /// Look up schema info for a kind.  Returns `None` for unknown kinds.
    pub fn lookup(&self, kind: &str) -> Option<&KindInfo> {
        self.kinds.get(kind)
    }

    /// Returns `true` if `field` is known to be an array field for `kind`.
    pub fn is_array_field(&self, kind: &str, field: &str) -> bool {
        self.kinds
            .get(kind)
            .map(|k| k.array_fields.contains(field))
            .unwrap_or(false)
    }
}

impl Default for SchemaCatalog {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Per-kind schema definitions
// ---------------------------------------------------------------------------

fn field(name: &str, ty: &str) -> FieldInfo {
    FieldInfo {
        field: name.to_string(),
        r#type: ty.to_string(),
        r#enum: None,
        description: None,
    }
}

fn field_enum(name: &str, values: &[&str]) -> FieldInfo {
    FieldInfo {
        field: name.to_string(),
        r#type: "string".to_string(),
        r#enum: Some(values.iter().map(|s| s.to_string()).collect()),
        description: None,
    }
}

fn field_desc(name: &str, ty: &str, desc: &str) -> FieldInfo {
    FieldInfo {
        field: name.to_string(),
        r#type: ty.to_string(),
        r#enum: None,
        description: Some(desc.to_string()),
    }
}

fn jira_issue() -> KindInfo {
    let array_fields: HashSet<String> = [
        "labels",
        "fix_versions",
        "components",
        "issuelinks",
        "subtasks",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    KindInfo {
        description: "A Jira issue (Story, Task, Bug, Epic, Sub-task, etc.)".to_string(),
        searchable: true,
        array_fields,
        fields: vec![
            field("id", "string"),
            field_desc("key", "string", "Jira issue key, e.g. DO-1234"),
            field_desc("project_key", "string", "Project key, e.g. DO"),
            field_enum("type", &["Story", "Task", "Bug", "Epic", "Sub-task", "Initiative"]),
            field_enum("status", &["To Do", "In Progress", "In Review", "Done", "Blocked", "Closed"]),
            field_enum(
                "status_category",
                &["To Do", "In Progress", "Done"],
            ),
            field_enum("priority", &["Highest", "High", "Medium", "Low", "Lowest"]),
            field_desc("resolution", "string", "Resolution (e.g. Fixed, Won't Fix, Duplicate)"),
            field_desc("summary", "string", "Issue title/summary"),
            field_desc("assignee.email", "string", "Assignee e-mail address"),
            field_desc("assignee.display_name", "string", "Assignee display name"),
            field_desc("reporter.email", "string", "Reporter e-mail address"),
            field_desc("sprint.id", "integer", "Active sprint ID"),
            field_enum("sprint.state", &["active", "future", "closed"]),
            field_desc("sprint.name", "string", "Active sprint name"),
            field_desc("story_points", "float", "Story-point estimate"),
            field_desc("fix_versions[].name", "string", "Fix-version name (array field)"),
            FieldInfo {
                field: "fix_versions".to_string(),
                r#type: "object[]".to_string(),
                r#enum: None,
                description: Some("Fix versions array".to_string()),
            },
            FieldInfo {
                field: "labels".to_string(),
                r#type: "string[]".to_string(),
                r#enum: None,
                description: Some("Labels applied to the issue".to_string()),
            },
            FieldInfo {
                field: "components".to_string(),
                r#type: "object[]".to_string(),
                r#enum: None,
                description: Some("Components the issue belongs to".to_string()),
            },
            FieldInfo {
                field: "issuelinks".to_string(),
                r#type: "object[]".to_string(),
                r#enum: None,
                description: Some("Issue link relationships (blocks, is-blocked-by, etc.)".to_string()),
            },
            field("created", "datetime"),
            field("updated", "datetime"),
            field_desc("parent_key", "string", "Parent issue key (for sub-tasks)"),
            field_desc("epic_key", "string", "Epic link key"),
            FieldInfo {
                field: "_deleted".to_string(),
                r#type: "boolean".to_string(),
                r#enum: None,
                description: Some("Soft-delete tombstone; hidden by default (use include_deleted: true)".to_string()),
            },
        ],
        examples: vec![
            ExampleCall {
                description: "Open Stories assigned to alice in sprint 42".to_string(),
                call: r#"{"tool":"query","data_source":"jira_issues","where":{"and":[{"type":"Story"},{"status":["To Do","In Progress"]},{"assignee.email":"alice@example.com"},{"sprint.id":42}]}}"#.to_string(),
            },
            ExampleCall {
                description: "Count issues by status".to_string(),
                call: r#"{"tool":"aggregate","data_source":"jira_issues","group_by":"status","count":true}"#.to_string(),
            },
            ExampleCall {
                description: "Count issues per label (array fan-out)".to_string(),
                call: r#"{"tool":"aggregate","data_source":"jira_issues","group_by":"labels","count":true,"top_groups":10}"#.to_string(),
            },
        ],
    }
}

fn jira_sprint() -> KindInfo {
    KindInfo {
        description: "A Jira sprint (iteration)".to_string(),
        searchable: false,
        array_fields: HashSet::new(),
        fields: vec![
            field("id", "integer"),
            field("name", "string"),
            field_enum("state", &["active", "future", "closed"]),
            field("start_date", "datetime"),
            field("end_date", "datetime"),
            FieldInfo {
                field: "project_keys".to_string(),
                r#type: "string[]".to_string(),
                r#enum: None,
                description: Some("Projects whose boards include this sprint".to_string()),
            },
            field("created", "datetime"),
            field("updated", "datetime"),
        ],
        examples: vec![ExampleCall {
            description: "Find the active sprint for project DO".to_string(),
            call: r#"{"tool":"query","data_source":"jira_sprints","where":{"state":"active"}}"#
                .to_string(),
        }],
    }
}

fn jira_fix_version() -> KindInfo {
    KindInfo {
        description: "A Jira fix version (release marker)".to_string(),
        searchable: false,
        array_fields: HashSet::new(),
        fields: vec![
            field("id", "string"),
            field("name", "string"),
            field("released", "boolean"),
            field("release_date", "datetime"),
            field_desc("project_key", "string", "Project this version belongs to"),
            field("description", "string"),
        ],
        examples: vec![ExampleCall {
            description: "Unreleased fix versions".to_string(),
            call:
                r#"{"tool":"query","data_source":"jira_fix_versions","where":{"released":false}}"#
                    .to_string(),
        }],
    }
}

fn jira_project() -> KindInfo {
    KindInfo {
        description: "A Jira project".to_string(),
        searchable: false,
        array_fields: HashSet::new(),
        fields: vec![
            field_desc("key", "string", "Project key, e.g. DO"),
            field("name", "string"),
            field_desc("lead", "string", "Project lead display name"),
            field_enum(
                "project_type_key",
                &["software", "business", "service_desk"],
            ),
            field("description", "string"),
        ],
        examples: vec![],
    }
}

fn confluence_page() -> KindInfo {
    let array_fields: HashSet<String> = ["ancestors", "labels"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    KindInfo {
        description: "A Confluence page".to_string(),
        searchable: true,
        array_fields,
        fields: vec![
            field("id", "string"),
            field_desc("page_id", "string", "Confluence page ID"),
            field_desc("space_key", "string", "Space key, e.g. ENG"),
            field("title", "string"),
            field_desc("body", "string", "Page body text (plain text or storage format)"),
            field_desc("version", "integer", "Page version number"),
            FieldInfo {
                field: "ancestors".to_string(),
                r#type: "object[]".to_string(),
                r#enum: None,
                description: Some("Ancestor pages (breadcrumb path)".to_string()),
            },
            FieldInfo {
                field: "labels".to_string(),
                r#type: "string[]".to_string(),
                r#enum: None,
                description: Some("Labels applied to the page".to_string()),
            },
            field_desc("author.email", "string", "Page author e-mail"),
            field("created", "datetime"),
            field("updated", "datetime"),
            FieldInfo {
                field: "_deleted".to_string(),
                r#type: "boolean".to_string(),
                r#enum: None,
                description: Some("Soft-delete tombstone; hidden by default".to_string()),
            },
        ],
        examples: vec![
            ExampleCall {
                description: "Find pages in the ENG space updated in the last 30 days".to_string(),
                call: r#"{"tool":"query","data_source":"confluence_pages","where":{"and":[{"space_key":"ENG"},{"updated":{"gte":"30 days ago"}}]}}"#.to_string(),
            },
        ],
    }
}

fn confluence_space() -> KindInfo {
    KindInfo {
        description: "A Confluence space".to_string(),
        searchable: false,
        array_fields: HashSet::new(),
        fields: vec![
            field_desc("key", "string", "Space key, e.g. ENG"),
            field("name", "string"),
            field_enum("type", &["global", "personal", "archived"]),
            field("description", "string"),
        ],
        examples: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_all_kinds() {
        let catalog = SchemaCatalog::new();
        for kind in &[
            "jira_issue",
            "jira_sprint",
            "jira_fix_version",
            "jira_project",
            "confluence_page",
            "confluence_space",
        ] {
            assert!(catalog.lookup(kind).is_some(), "missing kind: {kind}");
        }
    }

    #[test]
    fn jira_issue_has_array_fields() {
        let catalog = SchemaCatalog::new();
        assert!(catalog.is_array_field("jira_issue", "labels"));
        assert!(catalog.is_array_field("jira_issue", "fix_versions"));
        assert!(!catalog.is_array_field("jira_issue", "status"));
    }

    #[test]
    fn confluence_page_is_searchable() {
        let catalog = SchemaCatalog::new();
        let info = catalog.lookup("confluence_page").unwrap();
        assert!(info.searchable);
    }

    #[test]
    fn jira_sprint_is_not_searchable() {
        let catalog = SchemaCatalog::new();
        let info = catalog.lookup("jira_sprint").unwrap();
        assert!(!info.searchable);
    }
}
