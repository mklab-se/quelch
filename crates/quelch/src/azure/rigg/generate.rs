use std::collections::HashMap;

use thiserror::Error;

use crate::config::Config;

#[derive(Debug, Error)]
pub enum GenerateError {
    #[error("rigg generate is being rewritten in phase 4")]
    Phase4Pending,
}

#[derive(Debug, Default)]
pub struct GeneratedRiggFiles {
    pub indexes: HashMap<String, String>,
    pub skillsets: HashMap<String, String>,
    pub indexers: HashMap<String, String>,
    pub datasources: HashMap<String, String>,
    pub knowledge_sources: HashMap<String, String>,
    pub knowledge_bases: HashMap<String, String>,
}

pub fn all(_config: &Config) -> Result<GeneratedRiggFiles, GenerateError> {
    todo!("phase 4: regenerate rigg artefacts in-memory from the new schema")
}
