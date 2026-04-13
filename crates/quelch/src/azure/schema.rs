use serde::{Deserialize, Serialize};

/// Full Azure AI Search index definition with vector search support.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IndexSchema {
    pub name: String,
    pub fields: Vec<IndexField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_search: Option<serde_json::Value>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_search_profile: Option<String>,
}

/// Embedding configuration needed to create vector-enabled indexes.
pub struct EmbeddingConfig {
    pub dimensions: u32,
    pub vectorizer_json: serde_json::Value,
}

/// Build the vector search, semantic, and similarity sections for an index.
fn vector_search_config(
    index_name: &str,
    embedding: &EmbeddingConfig,
) -> (serde_json::Value, serde_json::Value, serde_json::Value) {
    let alg_name = format!("{index_name}-hnsw-algorithm");
    let profile_name = format!("{index_name}-vector-profile");
    let compression_name = format!("{index_name}-scalar-quantization");
    let semantic_name = format!("{index_name}-semantic-config");

    // Inject the profile reference into the vectorizer
    let vectorizer = embedding.vectorizer_json.clone();
    // The vectorizer name comes from the JSON itself

    let vector_search = serde_json::json!({
        "algorithms": [{
            "name": alg_name,
            "kind": "hnsw",
            "hnswParameters": {
                "metric": "cosine",
                "m": 4,
                "efConstruction": 400,
                "efSearch": 500
            }
        }],
        "profiles": [{
            "name": profile_name,
            "algorithm": alg_name,
            "vectorizer": vectorizer.get("name").and_then(|n| n.as_str()).unwrap_or("quelch-vectorizer"),
            "compression": compression_name
        }],
        "vectorizers": [vectorizer],
        "compressions": [{
            "name": compression_name,
            "kind": "scalarQuantization",
            "scalarQuantizationParameters": {
                "quantizedDataType": "int8"
            },
            "rescoringOptions": {
                "enableRescoring": true,
                "defaultOversampling": 4,
                "rescoreStorageMethod": "preserveOriginals"
            }
        }]
    });

    let semantic = serde_json::json!({
        "defaultConfiguration": semantic_name,
        "configurations": [{
            "name": semantic_name,
            "prioritizedFields": {
                "prioritizedContentFields": [{ "fieldName": "content" }],
                "prioritizedKeywordsFields": []
            }
        }]
    });

    let similarity = serde_json::json!({
        "@odata.type": "#Microsoft.Azure.Search.BM25Similarity"
    });

    (vector_search, semantic, similarity)
}

/// Create a vector field definition.
fn vector_field(name: &str, index_name: &str, dimensions: u32) -> IndexField {
    let profile_name = format!("{index_name}-vector-profile");
    IndexField {
        name: name.to_string(),
        field_type: "Collection(Edm.Single)".to_string(),
        key: false,
        searchable: true,
        filterable: false,
        sortable: false,
        facetable: false,
        dimensions: Some(dimensions),
        vector_search_profile: Some(profile_name),
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
        dimensions: None,
        vector_search_profile: None,
    }
}

/// Default index schema for Jira issues with vector search.
pub fn jira_index_schema(index_name: &str, embedding: &EmbeddingConfig) -> IndexSchema {
    let (vector_search, semantic, similarity) = vector_search_config(index_name, embedding);

    let mut fields = vec![
        field("id", "Edm.String", true, false, true, false, false),
        field("url", "Edm.String", false, false, true, false, false),
        field(
            "source_name",
            "Edm.String",
            false,
            false,
            true,
            false,
            false,
        ),
        field("source_type", "Edm.String", false, false, true, false, true),
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
        field(
            "status_category",
            "Edm.String",
            false,
            false,
            true,
            false,
            true,
        ),
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
    ];
    fields.push(vector_field(
        "content_vector",
        index_name,
        embedding.dimensions,
    ));

    IndexSchema {
        name: index_name.to_string(),
        fields,
        similarity: Some(similarity),
        semantic: Some(semantic),
        vector_search: Some(vector_search),
    }
}

/// Default index schema for Confluence pages (chunked by heading) with vector search.
pub fn confluence_index_schema(index_name: &str, embedding: &EmbeddingConfig) -> IndexSchema {
    let (vector_search, semantic, similarity) = vector_search_config(index_name, embedding);

    let mut fields = vec![
        field("id", "Edm.String", true, false, true, false, false),
        field("url", "Edm.String", false, false, true, false, false),
        field(
            "source_name",
            "Edm.String",
            false,
            false,
            true,
            false,
            false,
        ),
        field("source_type", "Edm.String", false, false, true, false, true),
        field("space_key", "Edm.String", false, false, true, false, true),
        field("page_id", "Edm.String", false, false, true, false, false),
        field("page_title", "Edm.String", false, true, true, false, false),
        field("chunk_index", "Edm.Int32", false, false, true, true, false),
        field(
            "chunk_heading",
            "Edm.String",
            false,
            true,
            true,
            false,
            false,
        ),
        field("body", "Edm.String", false, true, false, false, false),
        field(
            "labels",
            "Collection(Edm.String)",
            false,
            false,
            true,
            false,
            true,
        ),
        field("author", "Edm.String", false, false, true, false, true),
        field("ancestors", "Edm.String", false, true, false, false, false),
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
    ];
    fields.push(vector_field(
        "content_vector",
        index_name,
        embedding.dimensions,
    ));

    IndexSchema {
        name: index_name.to_string(),
        fields,
        similarity: Some(similarity),
        semantic: Some(semantic),
        vector_search: Some(vector_search),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_embedding() -> EmbeddingConfig {
        EmbeddingConfig {
            dimensions: 3072,
            vectorizer_json: serde_json::json!({
                "name": "test-vectorizer",
                "kind": "azureOpenAI",
                "azureOpenAIParameters": {
                    "resourceUri": "https://test.openai.azure.com",
                    "deploymentId": "text-embedding-3-large",
                    "modelName": "text-embedding-3-large"
                }
            }),
        }
    }

    #[test]
    fn jira_schema_has_correct_key() {
        let schema = jira_index_schema("test-index", &test_embedding());
        assert_eq!(schema.name, "test-index");
        let key_field = schema.fields.iter().find(|f| f.key).unwrap();
        assert_eq!(key_field.name, "id");
        assert_eq!(key_field.field_type, "Edm.String");
    }

    #[test]
    fn jira_schema_has_vector_field() {
        let schema = jira_index_schema("test", &test_embedding());
        let vector = schema
            .fields
            .iter()
            .find(|f| f.name == "content_vector")
            .unwrap();
        assert_eq!(vector.field_type, "Collection(Edm.Single)");
        assert_eq!(vector.dimensions, Some(3072));
        assert!(vector.searchable);
        assert_eq!(
            vector.vector_search_profile,
            Some("test-vector-profile".to_string())
        );
    }

    #[test]
    fn jira_schema_has_vector_search_config() {
        let schema = jira_index_schema("test", &test_embedding());
        let vs = schema.vector_search.unwrap();
        assert!(vs.get("algorithms").is_some());
        assert!(vs.get("profiles").is_some());
        assert!(vs.get("vectorizers").is_some());
        assert!(vs.get("compressions").is_some());

        let profile = &vs["profiles"][0];
        assert_eq!(profile["algorithm"], "test-hnsw-algorithm");
        assert_eq!(profile["vectorizer"], "test-vectorizer");
    }

    #[test]
    fn jira_schema_has_semantic_config() {
        let schema = jira_index_schema("test", &test_embedding());
        let sem = schema.semantic.unwrap();
        assert_eq!(sem["defaultConfiguration"], "test-semantic-config");
        let content_field =
            &sem["configurations"][0]["prioritizedFields"]["prioritizedContentFields"][0];
        assert_eq!(content_field["fieldName"], "content");
    }

    #[test]
    fn jira_schema_has_searchable_content() {
        let schema = jira_index_schema("test", &test_embedding());
        let content = schema.fields.iter().find(|f| f.name == "content").unwrap();
        assert!(content.searchable);
        assert!(!content.filterable);
    }

    #[test]
    fn jira_schema_serializes_to_json() {
        let schema = jira_index_schema("test", &test_embedding());
        let json = serde_json::to_string(&schema).unwrap();
        assert!(json.contains("\"key\":true"));
        assert!(json.contains("content_vector"));
        assert!(json.contains("vectorSearch"));
        assert!(json.contains("semantic"));
        assert!(json.contains("hnsw"));
        assert!(json.contains("scalarQuantization"));
    }

    #[test]
    fn confluence_schema_has_correct_key() {
        let schema = confluence_index_schema("conf-index", &test_embedding());
        assert_eq!(schema.name, "conf-index");
        let key_field = schema.fields.iter().find(|f| f.key).unwrap();
        assert_eq!(key_field.name, "id");
    }

    #[test]
    fn confluence_schema_has_vector_field() {
        let schema = confluence_index_schema("test", &test_embedding());
        let vector = schema
            .fields
            .iter()
            .find(|f| f.name == "content_vector")
            .unwrap();
        assert_eq!(vector.dimensions, Some(3072));
    }

    #[test]
    fn confluence_schema_has_chunk_fields() {
        let schema = confluence_index_schema("test", &test_embedding());
        let chunk_index = schema
            .fields
            .iter()
            .find(|f| f.name == "chunk_index")
            .unwrap();
        assert_eq!(chunk_index.field_type, "Edm.Int32");
        assert!(chunk_index.filterable);
        assert!(chunk_index.sortable);
    }

    #[test]
    fn different_dimensions_reflected() {
        let embed = EmbeddingConfig {
            dimensions: 1536,
            vectorizer_json: serde_json::json!({
                "name": "small-vectorizer",
                "kind": "azureOpenAI",
                "azureOpenAIParameters": {
                    "resourceUri": "https://test.openai.azure.com",
                    "deploymentId": "text-embedding-3-small",
                    "modelName": "text-embedding-3-small"
                }
            }),
        };
        let schema = jira_index_schema("test", &embed);
        let vector = schema
            .fields
            .iter()
            .find(|f| f.name == "content_vector")
            .unwrap();
        assert_eq!(vector.dimensions, Some(1536));
    }
}
