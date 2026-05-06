use crate::config::Config;
use crate::cosmos::CosmosBackend;

pub async fn build_cosmos_backend(config: &Config) -> anyhow::Result<Box<dyn CosmosBackend>> {
    let endpoint = config.azure.cosmos.endpoint.clone();
    let endpoint = if endpoint.starts_with("https://") {
        endpoint
    } else {
        format!("https://{endpoint}.documents.azure.com:443/")
    };

    let client = crate::cosmos::CosmosClient::new(&endpoint, &config.azure.cosmos.database).await?;
    Ok(Box::new(client))
}
