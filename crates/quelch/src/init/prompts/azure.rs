//! Azure section of the `quelch init` wizard.
//!
//! Drives the user through subscription / resource-group selection and
//! collection of the three Quelch-side dependencies: Cosmos DB, AI Search,
//! and the AI provider (Foundry / Azure OpenAI) with embedding + chat
//! deployments.

use crate::config::schema::{
    AiChat, AiConfig, AiEmbedding, AiProvider, AzureConfig, ContainerLayout, CosmosConfig,
    SearchConfig,
};

use crate::init::discover;

/// Prompt for Azure subscription, the resource group containing Quelch's
/// dependencies, the Cosmos DB account, the AI Search service, and the AI
/// provider (Foundry or Azure OpenAI) with its embedding + chat deployments.
///
/// Returns a fully-populated [`AzureConfig`].
pub async fn prompt_azure() -> anyhow::Result<AzureConfig> {
    println!("\n=== Azure resources Quelch depends on ===");
    println!(
        "Quelch references three pre-existing Azure resources: a Cosmos DB account\n\
         (the system of record), an AI Search service (the index), and an AI\n\
         model provider (Foundry project or Azure OpenAI account) with one\n\
         embedding and one chat deployment. Quelch does NOT provision any of\n\
         these — `quelch azure apply` only configures their internals.\n"
    );
    println!("Discovering Azure subscriptions...");

    let subs = discover::list_subscriptions().await.unwrap_or_default();

    let subscription_id = if !subs.is_empty() {
        let names: Vec<String> = subs
            .iter()
            .map(|s| {
                if s.is_default {
                    format!("{} ({}) [default]", s.name, s.id)
                } else {
                    format!("{} ({})", s.name, s.id)
                }
            })
            .collect();
        let default_idx = subs.iter().position(|s| s.is_default).unwrap_or(0);
        let chosen = inquire::Select::new("Subscription:", names)
            .with_starting_cursor(default_idx)
            .raw_prompt()?
            .index;
        subs[chosen].id.clone()
    } else {
        println!("  (az not available or no subscriptions found — enter manually)");
        inquire::Text::new("Subscription ID:").prompt()?
    };

    let resource_group = pick_resource_group(
        &subscription_id,
        "Resource group containing Cosmos / AI Search / the AI provider:",
    )
    .await?;

    let cosmos = prompt_cosmos(&subscription_id, &resource_group).await?;
    let search = prompt_search(&subscription_id, &resource_group).await?;
    let ai = prompt_ai(&subscription_id, &resource_group).await?;

    Ok(AzureConfig {
        cosmos,
        search: Some(search),
        ai: Some(ai),
    })
}

/// Show a Select listing every resource group in the subscription, plus an
/// "Enter name manually…" escape hatch. Returns the chosen RG name.
async fn pick_resource_group(subscription_id: &str, prompt_text: &str) -> anyhow::Result<String> {
    let groups = discover::list_resource_groups(subscription_id)
        .await
        .unwrap_or_default();

    if groups.is_empty() {
        println!("  (az returned no resource groups — enter manually)");
        return Ok(inquire::Text::new(prompt_text).prompt()?);
    }

    const ENTER_MANUALLY: &str = "Enter name manually…";
    let mut labels: Vec<String> = groups
        .iter()
        .map(|g| format!("{}  ({})", g.name, g.location))
        .collect();
    labels.push(ENTER_MANUALLY.to_string());

    let idx = inquire::Select::new(prompt_text, labels)
        .with_starting_cursor(0)
        .raw_prompt()?
        .index;

    if idx < groups.len() {
        Ok(groups[idx].name.clone())
    } else {
        Ok(inquire::Text::new("Resource group name:").prompt()?)
    }
}

/// Prompt for the Cosmos DB account inside the chosen resource group.
async fn prompt_cosmos(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<CosmosConfig> {
    println!("\n--- Cosmos DB account ---");
    let accounts = discover::list_cosmos_accounts(subscription_id, resource_group)
        .await
        .unwrap_or_default();

    let (account_name, endpoint) = if accounts.is_empty() {
        println!("  (no Cosmos accounts found in this RG — enter manually)");
        let name = inquire::Text::new("Cosmos DB account name:").prompt()?;
        let default_endpoint = format!("https://{name}.documents.azure.com");
        let endpoint = inquire::Text::new("Cosmos DB endpoint:")
            .with_initial_value(&default_endpoint)
            .prompt()?;
        (name, endpoint)
    } else {
        const ENTER_MANUALLY: &str = "Enter manually…";
        let mut labels: Vec<String> = accounts
            .iter()
            .map(|a| format!("{}  ({})", a.name, a.endpoint))
            .collect();
        labels.push(ENTER_MANUALLY.to_string());
        let idx = inquire::Select::new("Cosmos DB account:", labels)
            .with_starting_cursor(0)
            .raw_prompt()?
            .index;
        if idx < accounts.len() {
            (accounts[idx].name.clone(), accounts[idx].endpoint.clone())
        } else {
            let name = inquire::Text::new("Cosmos DB account name:").prompt()?;
            let default_endpoint = format!("https://{name}.documents.azure.com");
            let endpoint = inquire::Text::new("Cosmos DB endpoint:")
                .with_initial_value(&default_endpoint)
                .prompt()?;
            (name, endpoint)
        }
    };

    let database = inquire::Text::new("Cosmos database name:")
        .with_initial_value("quelch")
        .prompt()?;

    Ok(CosmosConfig {
        subscription_id: Some(subscription_id.to_string()),
        resource_group: Some(resource_group.to_string()),
        account: Some(account_name),
        endpoint,
        database,
        containers: ContainerLayout::default(),
        meta_container: "quelch-meta".to_string(),
    })
}

/// Prompt for the AI Search service endpoint.
async fn prompt_search(
    subscription_id: &str,
    resource_group: &str,
) -> anyhow::Result<SearchConfig> {
    println!("\n--- Azure AI Search ---");
    let services = discover::list_search_services(subscription_id, resource_group)
        .await
        .unwrap_or_default();

    let endpoint = if services.is_empty() {
        println!("  (no AI Search services found in this RG — enter manually)");
        let placeholder = "https://YOUR-SEARCH.search.windows.net";
        inquire::Text::new("AI Search service endpoint:")
            .with_initial_value(placeholder)
            .prompt()?
    } else {
        const ENTER_MANUALLY: &str = "Enter endpoint manually…";
        let mut labels: Vec<String> = services
            .iter()
            .map(|s| format!("{}.search.windows.net", s.name))
            .collect();
        labels.push(ENTER_MANUALLY.to_string());
        let idx = inquire::Select::new("AI Search service:", labels)
            .with_starting_cursor(0)
            .raw_prompt()?
            .index;
        if idx < services.len() {
            format!("https://{}.search.windows.net", services[idx].name)
        } else {
            let placeholder = "https://YOUR-SEARCH.search.windows.net";
            inquire::Text::new("AI Search service endpoint:")
                .with_initial_value(placeholder)
                .prompt()?
        }
    };

    Ok(SearchConfig { endpoint })
}

/// Prompt for the AI provider (Foundry or Azure OpenAI), endpoint, embedding
/// deployment, and chat deployment.
async fn prompt_ai(subscription_id: &str, resource_group: &str) -> anyhow::Result<AiConfig> {
    println!("\n--- AI model provider ---");
    println!(
        "Quelch wires up two model deployments in your AI Search Knowledge Base:\n  \
         - an embedding model (used by the vectorizer / skillset)\n  \
         - a chat / LLM (used for query planning + answer synthesis)\n\
         Both can live in the same Azure OpenAI account or Foundry project."
    );

    let providers = vec!["Microsoft Foundry  (recommended)", "Azure OpenAI"];
    let provider_idx = inquire::Select::new("Where do your model deployments live?", providers)
        .with_starting_cursor(0)
        .raw_prompt()?
        .index;
    let provider = if provider_idx == 0 {
        AiProvider::Foundry
    } else {
        AiProvider::AzureOpenai
    };

    let (endpoint, account_name) =
        pick_ai_account(subscription_id, resource_group, provider).await?;

    let deployments = match account_name {
        Some(ref name) => discover::list_model_deployments(subscription_id, resource_group, name)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };

    let embedding = pick_embedding_deployment(&deployments)?;
    let chat = pick_chat_deployment(&deployments)?;

    Ok(AiConfig {
        provider,
        endpoint,
        embedding,
        chat,
    })
}

/// Pick a Foundry project or Azure OpenAI account in the chosen RG, falling
/// back to manual endpoint entry. Returns `(endpoint, account_name)`.
async fn pick_ai_account(
    subscription_id: &str,
    resource_group: &str,
    provider: AiProvider,
) -> anyhow::Result<(String, Option<String>)> {
    let kind = match provider {
        AiProvider::Foundry => "Foundry project",
        AiProvider::AzureOpenai => "Azure OpenAI account",
    };

    let candidates: Vec<(String, String)> = match provider {
        AiProvider::Foundry => discover::list_foundry_projects(subscription_id, resource_group)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.name, p.endpoint))
            .collect(),
        AiProvider::AzureOpenai => discover::list_openai_accounts(subscription_id, resource_group)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|a| (a.name, a.endpoint))
            .collect(),
    };

    if candidates.is_empty() {
        println!("  No {kind}s found in resource group '{resource_group}' — enter manually.");
        let placeholder = match provider {
            AiProvider::Foundry => "https://YOUR-FOUNDRY.cognitiveservices.azure.com",
            AiProvider::AzureOpenai => "https://YOUR-OPENAI.openai.azure.com",
        };
        let endpoint = inquire::Text::new(&format!("{kind} endpoint:"))
            .with_initial_value(placeholder)
            .prompt()?;
        return Ok((endpoint, None));
    }

    const ENTER_MANUALLY: &str = "Enter endpoint manually…";
    let mut labels: Vec<String> = candidates
        .iter()
        .map(|(n, e)| format!("{n} — {e}"))
        .collect();
    labels.push(ENTER_MANUALLY.to_string());

    let idx = inquire::Select::new(&format!("{kind}:"), labels)
        .with_starting_cursor(0)
        .raw_prompt()?
        .index;

    if idx < candidates.len() {
        let (name, endpoint) = candidates[idx].clone();
        Ok((endpoint, Some(name)))
    } else {
        let placeholder = match provider {
            AiProvider::Foundry => "https://YOUR-FOUNDRY.cognitiveservices.azure.com",
            AiProvider::AzureOpenai => "https://YOUR-OPENAI.openai.azure.com",
        };
        let endpoint = inquire::Text::new(&format!("{kind} endpoint:"))
            .with_initial_value(placeholder)
            .prompt()?;
        Ok((endpoint, None))
    }
}

fn pick_embedding_deployment(
    available: &[discover::ModelDeployment],
) -> anyhow::Result<AiEmbedding> {
    let candidates: Vec<&discover::ModelDeployment> = available
        .iter()
        .filter(|d| d.model_name.starts_with("text-embedding"))
        .collect();

    let deployment = if candidates.is_empty() {
        if !available.is_empty() {
            println!("  (no embedding deployments detected — enter manually)");
        }
        inquire::Text::new("Embedding deployment name:")
            .with_initial_value("text-embedding-3-large")
            .prompt()?
    } else {
        let labels: Vec<String> = candidates
            .iter()
            .map(|d| format!("{} ({})", d.name, d.model_name))
            .collect();
        let idx = inquire::Select::new("Embedding deployment:", labels)
            .with_starting_cursor(0)
            .raw_prompt()?
            .index;
        candidates[idx].name.clone()
    };

    let dims_str: String = inquire::Text::new("Embedding dimensions:")
        .with_initial_value("3072")
        .prompt()?;
    let dimensions: u32 = dims_str
        .parse()
        .map_err(|_| anyhow::anyhow!("embedding dimensions must be a positive integer"))?;

    Ok(AiEmbedding {
        deployment,
        dimensions,
    })
}

/// Chat models supported by AI Search agentic retrieval (per Azure AI Search
/// 2025-11-01-preview). Used to filter the deployment Select.
const SUPPORTED_CHAT_MODELS: &[&str] = &[
    "gpt-4o",
    "gpt-4o-mini",
    "gpt-4.1",
    "gpt-4.1-nano",
    "gpt-4.1-mini",
    "gpt-5",
    "gpt-5-nano",
    "gpt-5-mini",
];

fn pick_chat_deployment(available: &[discover::ModelDeployment]) -> anyhow::Result<AiChat> {
    let candidates: Vec<&discover::ModelDeployment> = available
        .iter()
        .filter(|d| SUPPORTED_CHAT_MODELS.iter().any(|m| d.model_name == *m))
        .collect();

    let (deployment, model_name) = if candidates.is_empty() {
        if !available.is_empty() {
            println!(
                "  (no supported chat deployments detected; supported models: {})",
                SUPPORTED_CHAT_MODELS.join(", ")
            );
        }
        let dep: String = inquire::Text::new("Chat deployment name:")
            .with_initial_value("gpt-5-mini")
            .prompt()?;
        let model: String = inquire::Text::new("Chat model name:")
            .with_initial_value(&dep)
            .prompt()?;
        (dep, model)
    } else {
        let labels: Vec<String> = candidates
            .iter()
            .map(|d| format!("{} ({})", d.name, d.model_name))
            .collect();
        let idx = inquire::Select::new("Chat (LLM) deployment:", labels)
            .with_starting_cursor(0)
            .raw_prompt()?
            .index;
        let c = candidates[idx];
        (c.name.clone(), c.model_name.clone())
    };

    Ok(AiChat {
        deployment,
        model_name,
    })
}
