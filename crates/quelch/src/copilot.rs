// TODO(quelch v2 phase 3+): re-enable when agent generation lands.
//
// The v1 copilot generator is disabled in v2.
// It will be replaced by `quelch agent generate` in Phase 7.

use crate::config::Config;

/// Generated output for a Copilot Studio agent.
pub struct CopilotOutput {
    /// Agent instructions (system prompt) as markdown text.
    pub instructions: String,
    /// Generated topics.
    pub topics: Vec<GeneratedTopic>,
    /// Human-readable guide.
    pub guide: String,
}

/// A generated Copilot Studio topic.
pub struct GeneratedTopic {
    /// Suggested filename.
    pub filename: String,
    /// The YAML content.
    pub yaml: String,
}

/// Stub — will be replaced in Phase 7.
pub fn generate(_config: &Config) -> CopilotOutput {
    CopilotOutput {
        instructions: String::new(),
        topics: vec![],
        guide: "quelch generate-agent is not available in v2; \
                use `quelch agent generate` instead (Phase 7)"
            .to_string(),
    }
}
