use std::collections::HashMap;

use super::Config;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedDataSource {
    pub kind: String,
    pub backed_by: Vec<BackedBy>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackedBy {
    pub container: String,
}

pub fn resolve(_config: &Config) -> HashMap<String, ResolvedDataSource> {
    todo!(
        "phase 4: rewire data-source resolution against the new schema (instances + container layout)"
    )
}
