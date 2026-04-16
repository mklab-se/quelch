# Copilot Studio: OnKnowledgeRequested Trigger

Custom knowledge sources in Copilot Studio that give you full control over how your agent queries Azure AI Search (or any search endpoint). This bypasses the limitations of the built-in Azure AI Search knowledge source (which is vector-only, no filters, no field control).

## Overview

The `OnKnowledgeRequested` trigger is a special topic trigger that intercepts the moment the orchestrator needs to retrieve knowledge. Instead of the built-in search behavior, **you** write the query logic — including OData filters, hybrid search, field selection, and result formatting.

**Key difference from built-in knowledge:**

| | Built-in Azure AI Search | OnKnowledgeRequested |
|---|---|---|
| Query type | Vector/semantic only | Full control (keyword, semantic, vector, hybrid) |
| OData filters | Not supported | Fully supported |
| Field selection | Not configurable | Fully configurable |
| Result count | Fixed | You decide (`top` parameter) |
| Facets / `$count` | Not available | Available |
| Configuration | UI (Knowledge page) | YAML code editor only |

## Creating an OnKnowledgeRequested Topic

1. Open your agent in Copilot Studio
2. Go to **Topics**
3. **Add** > **Topic** > **From blank**
4. Click the ellipsis (**...**) on the top-right and select **Open code editor**
5. Replace the default YAML with the `OnKnowledgeRequested` structure (see below)
6. Save and publish

> **Note:** This trigger is YAML-only. There is no visual designer support. Your agent must have **generative orchestration** enabled.

## System Variables

Three variables are exclusive to `OnKnowledgeRequested` topics:

| Variable | Type | Direction | Purpose |
|---|---|---|---|
| `System.SearchQuery` | String | Input | Context-aware rewrite of the user's query, optimized for **semantic search**. Resolves pronouns, incorporates multi-turn context. |
| `System.KeywordSearchQuery` | String | Input | Same context-aware rewrite but optimized for **keyword search**. Adds synonyms and related terms. |
| `System.SearchResults` | Table | Output | Where you store formatted results for the generative answers pipeline. |

**Query rewriting example:**
- User: "What's our data retention period?"
- Follow-up: "Does it change for financial data?"
- Follow-up: "And are there exceptions?"
- `System.KeywordSearchQuery` becomes: _"exceptions to data retention policy customer and financial data retention exceptions regulatory exemptions policy exception handling compliance guidelines"_

The rewriting is automatic and opaque — you cannot control it.

## YAML Structure

### Minimal Skeleton

```yaml
kind: AdaptiveDialog
beginDialog:
  kind: OnKnowledgeRequested
  id: main
  intent: {}
  actions:
    # Your actions here
inputType: {}
outputType: {}
```

### Complete Example: Azure AI Search with OData Filters

```yaml
kind: AdaptiveDialog
beginDialog:
  kind: OnKnowledgeRequested
  id: main
  intent: {}
  actions:
    - kind: HttpRequestAction
      id: searchRequest
      method: POST
      url: ="https://YOUR-SERVICE.search.windows.net/indexes('YOUR-INDEX')/docs/search.post.search?api-version=2024-07-01"
      headers:
        Content-Type: application/json
        api-key: YOUR-QUERY-KEY
      body:
        kind: Json
        value:
          search: =System.SearchQuery
          filter: "source_type eq 'jira'"
          queryType: semantic
          semanticConfiguration: my-semantic-config
          select: "summary,content,url,assignee,project,status"
          top: 15
          searchMode: any
          captions: extractive
          answers: extractive
      response: Topic.searchResults
      responseSchema:
        kind: Record
        properties:
          value:
            type:
              kind: Table
              properties:
                summary: String
                content: String
                url: String
                assignee: String
                project: String
                status: String

    - kind: SetVariable
      id: setSearchResults
      variable: System.SearchResults
      value: |-
        =ForAll(Topic.searchResults.value,
        {
          Content: content,
          ContentLocation: url,
          Title: summary
        })

inputType: {}
outputType: {}
```

### Example: Keyword Search with Simple Query

```yaml
kind: AdaptiveDialog
beginDialog:
  kind: OnKnowledgeRequested
  id: main
  intent: {}
  actions:
    - kind: HttpRequestAction
      id: searchRequest
      url: ="https://YOUR-SERVICE.search.windows.net/indexes('YOUR-INDEX')/docs?api-version=2024-07-01&search=" & System.KeywordSearchQuery & "&$top=15&$select=summary,content,url"
      headers:
        api-key: YOUR-QUERY-KEY
      response: Topic.searchResults
      responseSchema:
        kind: Record
        properties:
          value:
            type:
              kind: Table
              properties:
                summary: String
                content: String
                url: String

    - kind: SetVariable
      id: setSearchResults
      variable: System.SearchResults
      value: |-
        =ForAll(Topic.searchResults.value,
        {
          Content: content,
          ContentLocation: url,
          Title: summary
        })

inputType: {}
outputType: {}
```

## Output Format (System.SearchResults)

You must set `System.SearchResults` to a table with these columns:

| Column | Required | Purpose |
|---|---|---|
| `Content` | **Yes** | The snippet/excerpt text used for grounding the generated answer |
| `ContentLocation` | No | URL — becomes the citation link shown to the user |
| `Title` | No | Citation label |

Use Power Fx `ForAll` to transform your API response:

```
=ForAll(Topic.searchResults.value,
{
  Content: content,
  ContentLocation: url,
  Title: title
})
```

**Snippet limit:** Copilot Studio uses **up to 15 snippets** from `System.SearchResults` to generate a response. This limit applies **across all knowledge topics combined**.

## Azure AI Search Query Parameters

When calling the Azure AI Search POST API, you have access to the full query surface:

| Parameter | Type | Description |
|---|---|---|
| `search` | string | Use `System.SearchQuery` (semantic) or `System.KeywordSearchQuery` (keyword) |
| `filter` | string | OData filter, e.g. `"assignee eq 'John' and project eq 'DO'"` |
| `queryType` | string | `simple`, `full` (Lucene), or `semantic` |
| `semanticConfiguration` | string | Required when queryType=semantic |
| `searchFields` | string | Comma-separated fields to scope full-text search |
| `select` | string | Comma-separated fields to return |
| `top` | integer | Number of results (max recommended: 15) |
| `skip` | integer | Offset for paging |
| `orderby` | string | OData orderby, e.g. `"updated_at desc"` |
| `searchMode` | string | `any` (default) or `all` |
| `count` | boolean | Include total count of matches in response |
| `facets` | array | Facet expressions for aggregation |
| `scoringProfile` | string | Custom scoring profile name |
| `captions` | string | `extractive` for semantic captions |
| `answers` | string | `extractive` for semantic answers |
| `vectorQueries` | array | For vector/hybrid search |

### Dynamic Filters

The real power here: you can construct filters dynamically based on the search query. For example, you could have multiple `OnKnowledgeRequested` topics — one for semantic search, one that applies specific filters.

However, note that **all OnKnowledgeRequested topics fire simultaneously** — you cannot conditionally route to one vs another based on intent. If you need conditional logic, put it inside a single topic using conditional branches.

## Relevance to Quelch's Index

Quelch already indexes Jira data with the right field attributes:

| Field | Filterable | Facetable | Searchable |
|---|---|---|---|
| `assignee` | Yes | Yes | — |
| `project` | Yes | Yes | — |
| `status` | Yes | Yes | — |
| `status_category` | Yes | Yes | — |
| `priority` | Yes | Yes | — |
| `issue_type` | Yes | Yes | — |
| `labels` | Yes | Yes | — |
| `reporter` | Yes | — | — |
| `summary` | — | — | Yes |
| `description` | — | — | Yes |
| `content` | — | — | Yes |
| `comments` | — | — | Yes |
| `created_at` | Yes | — | Sortable |
| `updated_at` | Yes | — | Sortable |

This means queries like these are already possible with the right OData filters:

```
# All issues assigned to a person
filter: "assignee eq 'John Doe'"

# Count issues in a project (use $count=true)
filter: "project eq 'DO'"

# Open bugs assigned to someone
filter: "assignee eq 'Jane' and issue_type eq 'Bug' and status_category ne 'Done'"

# Recent issues in a project
filter: "project eq 'DO' and updated_at ge 2026-01-01T00:00:00Z"
orderby: "updated_at desc"
```

The challenge is getting Copilot Studio's LLM to translate natural language into these filters. Approaches:

1. **Single topic with rich instructions** — Include examples of common queries and their corresponding filters in the agent's system message. The LLM may learn to suggest filter patterns.
2. **Multiple topics** — One for semantic search (no filters), one for "assigned to" queries (with assignee filter), one for project-scoped queries, etc. But remember: all fire simultaneously.
3. **Hybrid in one topic** — Use Power Fx conditional logic to detect keywords in the query and add filters dynamically.

## Multiple Knowledge Topics

You can create multiple `OnKnowledgeRequested` topics. They all fire in **parallel** when knowledge retrieval is triggered.

- Results from all topics are combined
- The 15-snippet limit applies across all topics combined
- Results also combine with any built-in knowledge sources you have configured
- You cannot control which topic fires — they all fire every time

## Limitations and Gotchas

1. **YAML-only** — No visual designer. Must be created and edited entirely in code view.
2. **15-snippet limit** — Across ALL knowledge topics combined. Plan your `top` parameter accordingly.
3. **All topics fire simultaneously** — No conditional routing between knowledge topics.
4. **Query rewriting is opaque** — You cannot control how `System.SearchQuery` / `System.KeywordSearchQuery` are generated.
5. **HTTP timeout** — Default 30 seconds. Configurable in the HTTP Request properties.
6. **Error handling** — HTTP failures trigger the `On Error` system topic by default. Configure "Continue on error" for graceful degradation.
7. **Generative orchestration required** — Your agent must have generative orchestration enabled.
8. **`Activity.Text` may be empty** — When knowledge is invoked, use `LastMessage.Text` instead if you need the raw user message.
9. **Response schema** — Must match your API response structure exactly. Use "Get schema from sample JSON" in the HTTP Request node to auto-generate it.

## The Core Challenge for Quelch's Use Case

`OnKnowledgeRequested` gives you full query control, but there's still a gap: **who decides which filter to apply?**

- The user says "find all issues assigned to John in DO"
- The topic fires with `System.SearchQuery` = some rewritten version of that
- But the YAML is static — the OData filter is hardcoded or templated

**Possible approaches:**

1. **No dynamic filters, rely on semantic search quality** — Use `queryType: semantic` with the full content field. The semantic ranker may surface the right results even without filters. Worth trying first as it's the simplest.

2. **Use the agent's LLM via topics + variables** — Before the knowledge request fires, use a regular topic with entity extraction (ask the LLM to extract assignee, project, etc. from the user's message), store in variables, then reference those variables in the filter string. This requires careful topic design.

3. **Power Automate flow as intermediary** — Call a flow that receives the raw query, uses an LLM to parse intent and extract filter parameters, constructs the Azure AI Search query, and returns results. More complex but most flexible.

4. **Multiple purpose-built topics** — Create specific Copilot Studio topics (not knowledge topics) that use entity extraction + slot filling for structured queries like "issues assigned to {person}" and call Azure AI Search with the appropriate filter via HTTP. These would be regular topics, not `OnKnowledgeRequested`, and would return direct answers rather than going through the generative pipeline.

Approach 4 is arguably the most natural for Copilot Studio — it's how the platform is designed to handle structured intents.

## References

- [Custom knowledge sources guide](https://learn.microsoft.com/en-us/microsoft-copilot-studio/guidance/custom-knowledge-sources)
- [Generative orchestration](https://learn.microsoft.com/en-us/microsoft-copilot-studio/guidance/generative-orchestration)
- [Built-in Azure AI Search knowledge](https://learn.microsoft.com/en-us/microsoft-copilot-studio/knowledge-azure-ai-search)
- [Knowledge sources summary](https://learn.microsoft.com/en-us/microsoft-copilot-studio/knowledge-copilot-studio)
- [HTTP Request node](https://learn.microsoft.com/en-us/microsoft-copilot-studio/authoring-http-node)
- [Azure AI Search POST search API](https://learn.microsoft.com/en-us/rest/api/searchservice/documents/search-post)
- [OData filter syntax](https://learn.microsoft.com/en-us/azure/search/search-query-odata-filter)
