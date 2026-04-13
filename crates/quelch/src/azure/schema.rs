use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IndexSchema {
    pub name: String,
    pub fields: Vec<IndexField>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IndexField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub key: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub searchable: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub filterable: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sortable: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub facetable: bool,
}

/// Default index schema for Jira issues.
pub fn jira_index_schema(index_name: &str) -> IndexSchema {
    IndexSchema {
        name: index_name.to_string(),
        fields: vec![
            field("id", "Edm.String", true, false, true, false, false),
            field(
                "source_name",
                "Edm.String",
                false,
                false,
                true,
                false,
                false,
            ),
            field("project", "Edm.String", false, false, true, false, true),
            field("issue_key", "Edm.String", false, false, true, false, false),
            field("issue_type", "Edm.String", false, false, true, false, true),
            field("summary", "Edm.String", false, true, false, false, false),
            field(
                "description",
                "Edm.String",
                false,
                true,
                false,
                false,
                false,
            ),
            field("status", "Edm.String", false, false, true, false, true),
            field("priority", "Edm.String", false, false, true, false, true),
            field("assignee", "Edm.String", false, false, true, false, true),
            field("reporter", "Edm.String", false, false, true, false, false),
            field(
                "labels",
                "Collection(Edm.String)",
                false,
                false,
                true,
                false,
                true,
            ),
            field("comments", "Edm.String", false, true, false, false, false),
            field("content", "Edm.String", false, true, false, false, false),
            field(
                "created_at",
                "Edm.DateTimeOffset",
                false,
                false,
                true,
                true,
                false,
            ),
            field(
                "updated_at",
                "Edm.DateTimeOffset",
                false,
                false,
                true,
                true,
                false,
            ),
        ],
    }
}

fn field(
    name: &str,
    field_type: &str,
    key: bool,
    searchable: bool,
    filterable: bool,
    sortable: bool,
    facetable: bool,
) -> IndexField {
    IndexField {
        name: name.to_string(),
        field_type: field_type.to_string(),
        key,
        searchable,
        filterable,
        sortable,
        facetable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jira_schema_has_correct_key() {
        let schema = jira_index_schema("test-index");
        assert_eq!(schema.name, "test-index");
        let key_field = schema.fields.iter().find(|f| f.key).unwrap();
        assert_eq!(key_field.name, "id");
        assert_eq!(key_field.field_type, "Edm.String");
    }

    #[test]
    fn jira_schema_has_searchable_content() {
        let schema = jira_index_schema("test");
        let content = schema.fields.iter().find(|f| f.name == "content").unwrap();
        assert!(content.searchable);
        assert!(!content.filterable);
    }

    #[test]
    fn jira_schema_serializes_to_json() {
        let schema = jira_index_schema("test");
        let json = serde_json::to_string(&schema).unwrap();
        assert!(json.contains("\"key\":true"));
        assert!(json.contains("\"type\":\"Edm.String\""));
    }
}
