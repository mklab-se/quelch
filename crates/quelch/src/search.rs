use anyhow::{Context, Result};
use colored::Colorize;

use crate::azure::SearchClient;
use crate::config::Config;

/// Run a semantic search across configured indexes.
pub async fn run_search(
    config: &Config,
    query: &str,
    index_filter: Option<&str>,
    top: usize,
    json_output: bool,
) -> Result<()> {
    let azure = SearchClient::new(&config.azure.endpoint, &config.azure.api_key);

    // Collect unique indexes to search
    let mut indexes: Vec<(&str, &str)> = Vec::new(); // (index_name, source_type)
    let mut seen = std::collections::HashSet::new();
    for source in &config.sources {
        let idx = source.index();
        if seen.insert(idx.to_string()) {
            if index_filter.is_some_and(|filter| idx != filter) {
                continue;
            }
            let source_type = match source {
                crate::config::SourceConfig::Jira(_) => "jira",
                crate::config::SourceConfig::Confluence(_) => "confluence",
            };
            indexes.push((idx, source_type));
        }
    }

    if indexes.is_empty() {
        if let Some(filter) = index_filter {
            anyhow::bail!("No configured index matches '{filter}'");
        }
        anyhow::bail!("No indexes configured");
    }

    for (index_name, _source_type) in &indexes {
        let semantic_config = format!("{index_name}-semantic-config");

        let result = azure
            .search(index_name, query, &semantic_config, top)
            .await
            .with_context(|| format!("search failed for index '{index_name}'"))?;

        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&result).unwrap_or_default()
            );
        } else {
            print_results(index_name, query, &result);
        }
    }

    Ok(())
}

fn print_results(index_name: &str, query: &str, result: &serde_json::Value) {
    let total = result
        .get("@odata.count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    println!();
    println!(
        "{}  {}",
        format!("# {index_name}").bold().cyan(),
        format!("({total} total matches)").dimmed()
    );
    println!("{}", format!("  Query: \"{query}\"").dimmed());

    // Print extractive answers if present
    if let Some(answers) = result
        .get("@search.answers")
        .and_then(|a| a.as_array())
        .filter(|a| !a.is_empty())
    {
        println!();
        println!("  {}", "## Direct Answers".bold().green());
        for (i, answer) in answers.iter().enumerate() {
            let key = answer.get("key").and_then(|v| v.as_str()).unwrap_or("?");
            let score = answer.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let text = answer.get("text").and_then(|v| v.as_str()).unwrap_or("");

            // Truncate text for display
            let preview = if text.len() > 200 {
                format!("{}...", &text[..200])
            } else {
                text.to_string()
            };

            println!();
            println!(
                "  {}  {}",
                format!("  {}.", i + 1).bold().yellow(),
                key.bold()
            );
            println!("     {} {:.0}%", "Score:".dimmed(), score * 100.0);
            println!("     {}", preview.dimmed());
        }
    }

    // Print ranked results
    if let Some(values) = result.get("value").and_then(|v| v.as_array()) {
        println!();
        println!("  {}", "## Results".bold().white());

        for (i, doc) in values.iter().enumerate() {
            let reranker = doc
                .get("@search.rerankerScore")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            // Determine display based on source type
            let source_type = doc
                .get("source_type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let (title, subtitle, url) = match source_type {
                "jira" => {
                    let key = doc.get("issue_key").and_then(|v| v.as_str()).unwrap_or("?");
                    let summary = doc.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                    let status = doc.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    let assignee = doc.get("assignee").and_then(|v| v.as_str()).unwrap_or("");
                    let issue_type = doc.get("issue_type").and_then(|v| v.as_str()).unwrap_or("");
                    let url = doc.get("url").and_then(|v| v.as_str()).unwrap_or("");

                    let title = format!("[{key}] {summary}");
                    let subtitle = format!("{issue_type} | {status} | {assignee}");
                    (title, subtitle, url.to_string())
                }
                "confluence" => {
                    let page_title = doc
                        .get("page_title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let heading = doc
                        .get("chunk_heading")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let space = doc.get("space_key").and_then(|v| v.as_str()).unwrap_or("");
                    let url = doc.get("url").and_then(|v| v.as_str()).unwrap_or("");

                    let title = if heading.is_empty() {
                        page_title.to_string()
                    } else {
                        format!("{page_title} > {heading}")
                    };
                    let subtitle = format!("Space: {space}");
                    (title, subtitle, url.to_string())
                }
                _ => {
                    let id = doc.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    (id.to_string(), String::new(), String::new())
                }
            };

            // Caption/snippet from semantic ranker
            let caption = doc
                .get("@search.captions")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|cap| cap.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("");

            let snippet = if caption.len() > 300 {
                format!("{}...", &caption[..300])
            } else {
                caption.to_string()
            };

            println!();
            println!(
                "  {}  {}",
                format!("  {}.", i + 1).bold().yellow(),
                title.bold()
            );
            if !subtitle.is_empty() {
                println!("     {}", subtitle.dimmed());
            }
            println!(
                "     {} {:.2}  {}",
                "Relevance:".dimmed(),
                reranker,
                format_relevance_bar(reranker)
            );
            if !url.is_empty() {
                println!(
                    "     {} {}",
                    "URL:".dimmed(),
                    terminal_hyperlink(&url, &url)
                );
            }
            if !snippet.is_empty() {
                // Strip <em> tags from highlights and display
                let clean = snippet.replace("<em>", "").replace("</em>", "");
                println!("     {}", clean.dimmed());
            }
        }
    }

    println!();
}

/// Create a clickable terminal hyperlink using OSC 8 escape sequence.
/// Supported by iTerm2, Windows Terminal, GNOME Terminal, and most modern terminals.
fn terminal_hyperlink(url: &str, text: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
}

/// Render a simple relevance bar using unicode blocks.
fn format_relevance_bar(score: f64) -> String {
    // Normalize score to 0-10 range (reranker scores typically 0-4)
    let normalized = (score / 4.0 * 10.0).clamp(0.0, 10.0) as usize;
    let filled = "\u{2588}".repeat(normalized);
    let empty = "\u{2591}".repeat(10 - normalized);

    if normalized >= 7 {
        format!("{}{}", filled.green(), empty.dimmed())
    } else if normalized >= 4 {
        format!("{}{}", filled.yellow(), empty.dimmed())
    } else {
        format!("{}{}", filled.red(), empty.dimmed())
    }
}
