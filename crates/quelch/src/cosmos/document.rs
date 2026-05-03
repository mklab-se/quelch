use serde_json::Value;

use crate::cosmos::CosmosError;

/// A thin envelope helper for Cosmos DB documents.
///
/// All documents stored via `CosmosBackend` must carry:
/// - `id`            — unique within the container+partition
/// - `_partition_key` — logical partition; always equal to `deployment_name`
///
/// This module provides utility functions for extracting those fields from a
/// `serde_json::Value` without fully deserialising the document.
pub struct CosmosDocument;

impl CosmosDocument {
    /// Extract the `id` string field from a JSON document value.
    ///
    /// Returns `CosmosError::Validation` if the field is missing or not a string.
    pub fn extract_id(doc: &Value) -> Result<&str, CosmosError> {
        doc.get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CosmosError::Validation("document missing string `id` field".into()))
    }

    /// Extract the `_partition_key` string field from a JSON document value.
    ///
    /// Returns `CosmosError::Validation` if the field is missing or not a string.
    pub fn extract_partition_key(doc: &Value) -> Result<&str, CosmosError> {
        doc.get("_partition_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                CosmosError::Validation("document missing string `_partition_key` field".into())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_id_and_partition_key_happy_path() {
        let doc = json!({
            "id": "abc::def::ghi",
            "_partition_key": "abc",
            "some_field": 42
        });
        assert_eq!(CosmosDocument::extract_id(&doc).unwrap(), "abc::def::ghi");
        assert_eq!(CosmosDocument::extract_partition_key(&doc).unwrap(), "abc");
    }

    #[test]
    fn extract_id_missing_returns_validation_error() {
        let doc = json!({ "_partition_key": "abc" });
        assert!(matches!(
            CosmosDocument::extract_id(&doc),
            Err(CosmosError::Validation(_))
        ));
    }

    #[test]
    fn extract_partition_key_missing_returns_validation_error() {
        let doc = json!({ "id": "abc" });
        assert!(matches!(
            CosmosDocument::extract_partition_key(&doc),
            Err(CosmosError::Validation(_))
        ));
    }
}
