pub mod auth;
pub mod error;
pub mod expose;
pub mod filter;
pub mod handlers;
pub mod schema;
pub mod server;
pub mod tools;

pub async fn run_server(
    _config: &crate::config::Config,
    _instance_name: &str,
    _bind_addr: &str,
) -> anyhow::Result<()> {
    todo!("phase 7: rewire MCP server against the new instances/source_connections schema")
}

pub async fn run_server_in_memory(
    _config: &crate::config::Config,
    _instance_name: &str,
    _bind_addr: &str,
    _cosmos: std::sync::Arc<dyn crate::cosmos::CosmosBackend>,
) -> anyhow::Result<()> {
    todo!("phase 7: rewire dev-mode MCP server against the new schema")
}
