use serde_json::{Value, json};

/// Return all 17 Jira issues in Jira DC REST API v2 format.
pub fn jira_issues() -> Vec<Value> {
    let issues = vec![
        issue(
            "QUELCH-1",
            "10001",
            "As a user, I want to configure data sources in YAML so I can easily set up sync targets",
            "The configuration system should support YAML files with environment variable substitution. \
             Users define Azure AI Search connection details and one or more source blocks (Jira, Confluence). \
             Each source specifies URL, auth credentials, project/space filters, and the target index name. \
             The config should be validated on load with clear error messages for missing or invalid fields.",
            "Story",
            "Done",
            "Done",
            "High",
            "Kristofer Liljeblad",
            "Kristofer Liljeblad",
            &["backend", "config"],
            "2026-01-15T09:00:00.000+0000",
            "2026-01-22T16:30:00.000+0000",
            &[
                (
                    "Emma Andersson",
                    "2026-01-17T10:15:00.000+0000",
                    "Should we use serde_yaml or a custom parser? serde_yaml covers most use cases and is well maintained.",
                ),
                (
                    "Kristofer Liljeblad",
                    "2026-01-18T14:00:00.000+0000",
                    "Agreed, serde_yaml with shellexpand for env var substitution. Merged in first PR.",
                ),
            ],
        ),
        issue(
            "QUELCH-2",
            "10002",
            "As a user, I want to sync Jira issues to Azure AI Search so they become searchable",
            "Implement the Jira DC connector that fetches issues via REST API v2 /rest/api/2/search endpoint. \
             Use JQL with project filter and updated >= cursor for incremental fetching. \
             Map Jira fields (summary, description, status, priority, assignee, labels, comments) to the \
             Azure AI Search index schema. Support offset-based pagination for DC.",
            "Story",
            "Done",
            "Done",
            "High",
            "Kristofer Liljeblad",
            "Emma Andersson",
            &["backend", "jira", "connector"],
            "2026-01-18T10:00:00.000+0000",
            "2026-02-05T11:45:00.000+0000",
            &[
                (
                    "Lars Svensson",
                    "2026-01-25T09:30:00.000+0000",
                    "The description field in DC v2 is plain text, but in Cloud v3 it is ADF JSON. We need to handle both formats.",
                ),
                (
                    "Kristofer Liljeblad",
                    "2026-02-01T13:20:00.000+0000",
                    "Added extract_text() that detects string vs ADF object and handles both. Tests passing.",
                ),
            ],
        ),
        issue(
            "QUELCH-3",
            "10003",
            "As a user, I want incremental sync so only changed issues are fetched",
            "Implement cursor-based incremental sync using the updated timestamp of the last synced document. \
             On each sync cycle, build JQL with updated >= cursor to only fetch issues modified since last run. \
             Persist the cursor in a JSON state file after each batch for crash safety. \
             First sync should do a full fetch (no cursor).",
            "Story",
            "Done",
            "Done",
            "Medium",
            "Emma Andersson",
            "Kristofer Liljeblad",
            &["backend", "sync"],
            "2026-01-25T08:30:00.000+0000",
            "2026-02-10T15:00:00.000+0000",
            &[
                (
                    "Sofia Eriksson",
                    "2026-02-03T11:00:00.000+0000",
                    "What happens if the state file gets corrupted? Should we add validation on load?",
                ),
                (
                    "Emma Andersson",
                    "2026-02-05T09:15:00.000+0000",
                    "Good point. Added serde validation and fallback to full sync if state is unreadable.",
                ),
                (
                    "Kristofer Liljeblad",
                    "2026-02-08T16:45:00.000+0000",
                    "Tested with a 10k issue project on DC. Incremental sync picks up changes in under 5 seconds.",
                ),
            ],
        ),
        issue(
            "QUELCH-4",
            "10004",
            "As a user, I want to sync Confluence pages to Azure AI Search",
            "Implement the Confluence connector supporting both Cloud and Data Center. \
             Use the content search API with CQL filters for space and lastmodified date. \
             Expand body.storage, version, history, ancestors, metadata.labels, and space. \
             Convert XHTML storage format to plain text for indexing. Handle pagination via _links.next.",
            "Story",
            "Done",
            "Done",
            "High",
            "Lars Svensson",
            "Kristofer Liljeblad",
            &["backend", "confluence", "connector"],
            "2026-02-01T10:00:00.000+0000",
            "2026-02-20T14:30:00.000+0000",
            &[
                (
                    "Lars Svensson",
                    "2026-02-10T11:30:00.000+0000",
                    "Confluence XHTML includes macros like ac:structured-macro that we need to strip cleanly.",
                ),
                (
                    "Kristofer Liljeblad",
                    "2026-02-15T09:00:00.000+0000",
                    "HTML stripping handles CDATA sections, entities, and Confluence macros. All tests green.",
                ),
            ],
        ),
        issue(
            "QUELCH-5",
            "10005",
            "Long Confluence pages should be chunked by heading for better search relevance",
            "Pages over 4000 characters should be split into smaller chunks for better search relevance. \
             Primary strategy: split on h1/h2/h3 headings, keeping each section as a separate document. \
             Fallback: fixed-size chunking with 200 char overlap when no headings exist. \
             Each chunk gets a unique ID based on page ID and chunk index. The heading becomes part of the title.",
            "Story",
            "Done",
            "Done",
            "Medium",
            "Lars Svensson",
            "Emma Andersson",
            &["backend", "confluence", "search-quality"],
            "2026-02-12T09:00:00.000+0000",
            "2026-02-28T17:00:00.000+0000",
            &[
                (
                    "Emma Andersson",
                    "2026-02-18T14:00:00.000+0000",
                    "The overlap ensures we do not lose context at chunk boundaries. 200 chars seems like a good default.",
                ),
                (
                    "Sofia Eriksson",
                    "2026-02-25T10:30:00.000+0000",
                    "Tested with some very long architecture docs. Heading-based splitting produces much better results than fixed-size.",
                ),
            ],
        ),
        issue(
            "QUELCH-6",
            "10006",
            "As an admin, I want a setup command that creates Azure AI Search indexes",
            "Add a `quelch setup` command that reads the config, determines required indexes, \
             and creates them in Azure AI Search if they do not exist. Support --yes flag for \
             non-interactive mode. Define separate schemas for Jira and Confluence indexes \
             with appropriate field types (searchable, filterable, facetable).",
            "Story",
            "Done",
            "Done",
            "Medium",
            "Sofia Eriksson",
            "Kristofer Liljeblad",
            &["backend", "azure", "cli"],
            "2026-02-15T08:00:00.000+0000",
            "2026-03-01T12:00:00.000+0000",
            &[
                (
                    "Kristofer Liljeblad",
                    "2026-02-22T11:00:00.000+0000",
                    "The index schema includes: id, url, source_name, source_type, content (searchable), plus type-specific fields.",
                ),
                (
                    "Sofia Eriksson",
                    "2026-02-27T15:30:00.000+0000",
                    "Setup command now creates indexes with proper analyzers. Added interactive confirmation prompt.",
                ),
            ],
        ),
        issue(
            "QUELCH-7",
            "10007",
            "Support both Jira Cloud and Data Center authentication methods",
            "Cloud uses email + API token with Basic auth. Data Center uses Personal Access Token with Bearer auth. \
             The auth config should auto-detect based on which fields are provided. \
             Cloud: base64(email:token) in Authorization header. DC: Bearer {pat} header. \
             Both methods must work for Jira and Confluence connectors.",
            "Story",
            "Done",
            "Done",
            "High",
            "Kristofer Liljeblad",
            "Lars Svensson",
            &["backend", "auth"],
            "2026-02-20T10:00:00.000+0000",
            "2026-03-05T16:00:00.000+0000",
            &[(
                "Lars Svensson",
                "2026-02-25T09:45:00.000+0000",
                "The AuthConfig enum with Cloud and DataCenter variants works cleanly with serde tagging.",
            )],
        ),
        issue(
            "QUELCH-8",
            "10008",
            "As a user, I want continuous sync with configurable poll interval",
            "Implement `quelch watch` command that runs sync in a loop with configurable poll_interval \
             (default 300 seconds). First cycle should auto-create indexes if --create-indexes is set. \
             Subsequent cycles should use RequireExisting mode. Log next sync time after each cycle. \
             Handle errors gracefully without stopping the loop.",
            "Story",
            "In Progress",
            "In Progress",
            "Medium",
            "Emma Andersson",
            "Sofia Eriksson",
            &["backend", "cli", "sync"],
            "2026-03-01T09:00:00.000+0000",
            "2026-04-10T14:20:00.000+0000",
            &[
                (
                    "Sofia Eriksson",
                    "2026-03-15T10:00:00.000+0000",
                    "Basic watch loop is working. Still need to add the TUI dashboard for visual feedback.",
                ),
                (
                    "Emma Andersson",
                    "2026-04-05T11:30:00.000+0000",
                    "The watch command now catches sync errors and logs them without crashing. Poll interval is configurable.",
                ),
            ],
        ),
        issue(
            "QUELCH-9",
            "10009",
            "Build a TUI dashboard for watch mode showing sync status",
            "Create a terminal UI using ratatui that displays: current sync source, progress bar, \
             documents synced count, last sync timestamp, errors. The TUI should update in real time \
             during watch mode. Support --quiet flag to disable TUI and only log.",
            "Story",
            "To Do",
            "To Do",
            "Low",
            "Sofia Eriksson",
            "Emma Andersson",
            &["frontend", "tui", "ux"],
            "2026-03-10T10:00:00.000+0000",
            "2026-03-10T10:00:00.000+0000",
            &[(
                "Emma Andersson",
                "2026-03-10T14:00:00.000+0000",
                "Ratatui is already in our workspace deps. We can start with a simple table layout showing source status.",
            )],
        ),
        issue(
            "QUELCH-10",
            "10010",
            "Add parallel sync with credential-based concurrency control",
            "When multiple sources share the same credentials, limit concurrency to avoid rate limiting. \
             The config should have max_concurrent_per_credential setting. Sources with different \
             credentials can sync in parallel without restriction. Use tokio semaphores for control.",
            "Story",
            "To Do",
            "To Do",
            "Medium",
            "Kristofer Liljeblad",
            "Lars Svensson",
            &["backend", "performance"],
            "2026-03-15T08:00:00.000+0000",
            "2026-03-15T08:00:00.000+0000",
            &[(
                "Lars Svensson",
                "2026-03-15T11:00:00.000+0000",
                "We should group sources by credential identity and use a semaphore per group. Default concurrency of 3 seems reasonable.",
            )],
        ),
        issue(
            "QUELCH-11",
            "10011",
            "Handle API rate limiting with exponential backoff retry",
            "When Jira or Confluence returns 429 Too Many Requests, retry with exponential backoff. \
             Start with 1 second delay, double on each retry, cap at 60 seconds. \
             Maximum 5 retries before failing the batch. Also handle 503 Service Unavailable. \
             Log each retry attempt at debug level.",
            "Story",
            "Done",
            "Done",
            "High",
            "Emma Andersson",
            "Kristofer Liljeblad",
            &["backend", "reliability"],
            "2026-02-25T10:00:00.000+0000",
            "2026-03-12T13:00:00.000+0000",
            &[
                (
                    "Kristofer Liljeblad",
                    "2026-03-05T09:30:00.000+0000",
                    "Implemented in the HTTP client wrapper. Works for both Jira and Confluence connectors.",
                ),
                (
                    "Emma Andersson",
                    "2026-03-10T16:00:00.000+0000",
                    "Tested by simulating 429 responses with wiremock. Backoff works correctly up to the 60s cap.",
                ),
            ],
        ),
        issue(
            "QUELCH-12",
            "10012",
            "Crash-safe state persistence after every batch",
            "The sync state must be saved to disk after every batch completes, not just at the end of a full sync. \
             This ensures that if quelch crashes or is interrupted, it resumes from the last completed batch \
             rather than re-syncing everything. Use atomic file writes (write to temp, then rename).",
            "Story",
            "Done",
            "Done",
            "High",
            "Kristofer Liljeblad",
            "Sofia Eriksson",
            &["backend", "reliability", "sync"],
            "2026-03-01T11:00:00.000+0000",
            "2026-03-18T10:30:00.000+0000",
            &[
                (
                    "Sofia Eriksson",
                    "2026-03-08T14:00:00.000+0000",
                    "The state file now uses write-then-rename for atomicity. Tested by killing the process mid-sync.",
                ),
                (
                    "Kristofer Liljeblad",
                    "2026-03-15T09:00:00.000+0000",
                    "Confirmed: after a kill -9 during sync, restart picks up exactly where it left off.",
                ),
            ],
        ),
        issue(
            "QUELCH-13",
            "10013",
            "Bug: Status category field not populated in Azure index",
            "The status_category field in the Azure AI Search index is always empty. \
             Root cause: the Jira DC v2 response nests statusCategory inside the status object, \
             but our deserialization was looking for it at the top level of fields. \
             Fix: update JiraStatus struct to include statusCategory as a nested field.",
            "Bug",
            "Done",
            "Done",
            "High",
            "Lars Svensson",
            "Emma Andersson",
            &["bug", "jira"],
            "2026-03-05T15:00:00.000+0000",
            "2026-03-07T11:00:00.000+0000",
            &[
                (
                    "Emma Andersson",
                    "2026-03-05T16:30:00.000+0000",
                    "Reproduced. The status object has { name, statusCategory: { name } } but we were looking for fields.statusCategory.",
                ),
                (
                    "Lars Svensson",
                    "2026-03-06T10:00:00.000+0000",
                    "Fixed by adding statusCategory to the JiraStatus struct. Added test to prevent regression.",
                ),
            ],
        ),
        issue(
            "QUELCH-14",
            "10014",
            "Add URL field linking back to original Jira issue",
            "Each indexed document should include a url field that links back to the original Jira issue \
             in the format {base_url}/browse/{issue_key}. This allows search results to link directly \
             to the source. The URL field should be filterable but not searchable in the index schema.",
            "Task",
            "Done",
            "Done",
            "Medium",
            "Sofia Eriksson",
            "Lars Svensson",
            &["backend", "jira", "ux"],
            "2026-03-08T09:00:00.000+0000",
            "2026-03-10T14:00:00.000+0000",
            &[(
                "Lars Svensson",
                "2026-03-09T11:00:00.000+0000",
                "Added browse_url() method to JiraConnector. Simple string concatenation of base URL and issue key.",
            )],
        ),
        issue(
            "QUELCH-15",
            "10015",
            "Support environment variable substitution in config file",
            "Config values like api_key and pat should support ${ENV_VAR} syntax so secrets are not \
             stored in the YAML file. Use shellexpand crate to resolve environment variables at config load time. \
             If a referenced env var is not set, fail with a clear error message indicating which variable is missing.",
            "Story",
            "Done",
            "Done",
            "Medium",
            "Kristofer Liljeblad",
            "Sofia Eriksson",
            &["backend", "config", "security"],
            "2026-03-12T08:00:00.000+0000",
            "2026-03-20T12:00:00.000+0000",
            &[
                (
                    "Sofia Eriksson",
                    "2026-03-15T10:30:00.000+0000",
                    "shellexpand handles ${VAR} and $VAR syntax. We run it on the raw YAML string before parsing.",
                ),
                (
                    "Kristofer Liljeblad",
                    "2026-03-18T14:00:00.000+0000",
                    "Added validation that produces clear errors like: Environment variable JIRA_PAT is not set (referenced in config).",
                ),
            ],
        ),
        issue(
            "QUELCH-16",
            "10016",
            "Detect and remove orphaned documents from Azure index",
            "When issues are deleted in Jira or pages removed in Confluence, the corresponding documents \
             in Azure AI Search become orphaned. Implement orphan detection by fetching all source IDs \
             and comparing with indexed document IDs. Remove documents that no longer exist in the source. \
             This should run periodically, not on every sync cycle.",
            "Story",
            "To Do",
            "To Do",
            "Medium",
            "Emma Andersson",
            "Kristofer Liljeblad",
            &["backend", "sync", "data-quality"],
            "2026-03-25T09:00:00.000+0000",
            "2026-03-25T09:00:00.000+0000",
            &[(
                "Kristofer Liljeblad",
                "2026-03-25T14:00:00.000+0000",
                "Design: fetch all IDs from source via lightweight API call, diff against index, batch-delete orphans. Need to handle pagination for large projects.",
            )],
        ),
        issue(
            "QUELCH-17",
            "10017",
            "As a user, I want a validate command to check my config without syncing",
            "Add `quelch validate` command that loads and validates the config file without performing any sync. \
             Check: YAML syntax, required fields present, URL formats valid, auth fields present. \
             Print a summary of configured sources and target indexes. Exit with non-zero code on errors.",
            "Story",
            "Done",
            "Done",
            "Low",
            "Sofia Eriksson",
            "Emma Andersson",
            &["cli", "ux"],
            "2026-03-28T10:00:00.000+0000",
            "2026-04-02T15:00:00.000+0000",
            &[
                (
                    "Emma Andersson",
                    "2026-03-30T09:00:00.000+0000",
                    "Simple implementation: load_config() already validates everything. Just catch errors and print them nicely.",
                ),
                (
                    "Sofia Eriksson",
                    "2026-04-01T11:30:00.000+0000",
                    "Added summary output showing Azure endpoint, source count, and index names. Clean and useful.",
                ),
            ],
        ),
    ];

    issues
}

/// Return all 8 Confluence pages in Confluence DC v1 search response format.
pub fn confluence_pages() -> Vec<Value> {
    let base_url = "http://localhost:9999/confluence";

    vec![
        page(
            "100001",
            "Quelch Overview",
            r#"<h1>Quelch Overview</h1>
<p>Quelch is a Rust command-line tool that ingests data from Atlassian products (Jira and Confluence) directly into Azure AI Search indexes. It enables organizations to make their project management data and documentation searchable through Azure's powerful AI-powered search capabilities.</p>

<h2>Architecture</h2>
<p>Quelch follows a direct-ingest architecture with no intermediate storage. Data flows from source systems through connector modules, gets transformed into search documents, and is pushed directly to Azure AI Search via its REST API.</p>
<pre>
  Jira DC/Cloud ──┐
                   ├──▶ Quelch CLI ──▶ Azure AI Search
  Confluence DC/Cloud ─┘
</pre>
<p>This design keeps the system simple, reduces infrastructure requirements, and ensures data freshness. There is no database, message queue, or intermediate storage layer to manage.</p>

<h2>Key Features</h2>
<ul>
<li>Incremental sync with cursor-based change detection</li>
<li>Support for both Atlassian Cloud and Data Center deployments</li>
<li>YAML configuration with environment variable substitution</li>
<li>Crash-safe state persistence after every batch</li>
<li>Confluence page chunking by heading for optimal search relevance</li>
<li>Automatic Azure AI Search index creation and schema management</li>
<li>Continuous watch mode with configurable poll interval</li>
<li>API rate limiting with exponential backoff retry</li>
</ul>

<h2>Supported Sources</h2>
<p>Currently quelch supports two source types:</p>
<ul>
<li><strong>Jira</strong> - Syncs issues with all standard fields including comments. Supports both Cloud (v3 API with ADF) and Data Center (v2 API with plain text).</li>
<li><strong>Confluence</strong> - Syncs pages with body content, labels, and metadata. Handles XHTML storage format conversion and heading-based chunking.</li>
</ul>"#,
            3,
            "Kristofer Liljeblad",
            "2026-01-15T10:00:00.000+0000",
            "2026-04-01T09:00:00.000+0000",
            &[], // no ancestors (top-level)
            &["overview", "architecture"],
            base_url,
        ),
        page(
            "100002",
            "Getting Started",
            r#"<h1>Getting Started with Quelch</h1>
<p>This guide walks you through installing quelch, creating your first configuration, and running your first sync to populate an Azure AI Search index.</p>

<h2>Prerequisites</h2>
<ul>
<li>An Azure AI Search service (any tier, including Free)</li>
<li>An Azure AI Search admin API key</li>
<li>Access to a Jira or Confluence instance (Cloud or Data Center)</li>
<li>A Personal Access Token (DC) or API token (Cloud)</li>
</ul>

<h2>Installation</h2>
<p>Install quelch using cargo:</p>
<ac:structured-macro ac:name="code"><ac:parameter ac:name="language">bash</ac:parameter><ac:plain-text-body><![CDATA[cargo install quelch]]></ac:plain-text-body></ac:structured-macro>

<p>Or download a pre-built binary from the GitHub releases page for your platform (Linux, macOS, Windows).</p>

<h2>Create Configuration</h2>
<p>Generate a starter config file:</p>
<ac:structured-macro ac:name="code"><ac:parameter ac:name="language">bash</ac:parameter><ac:plain-text-body><![CDATA[quelch init]]></ac:plain-text-body></ac:structured-macro>

<p>This creates a <code>quelch.yaml</code> file. Edit it with your Azure and source credentials:</p>
<ac:structured-macro ac:name="code"><ac:parameter ac:name="language">yaml</ac:parameter><ac:plain-text-body><![CDATA[azure:
  endpoint: "https://my-search.search.windows.net"
  api_key: "${AZURE_SEARCH_API_KEY}"

sources:
  - type: jira
    name: "my-jira"
    url: "https://jira.company.com"
    auth:
      pat: "${JIRA_PAT}"
    projects:
      - "PROJ"
    index: "jira-issues"]]></ac:plain-text-body></ac:structured-macro>

<h2>Run First Sync</h2>
<p>Validate your config, create indexes, and sync:</p>
<ac:structured-macro ac:name="code"><ac:parameter ac:name="language">bash</ac:parameter><ac:plain-text-body><![CDATA[# Validate config
quelch validate

# Create Azure AI Search indexes
quelch setup --yes

# Run a one-shot sync
quelch sync]]></ac:plain-text-body></ac:structured-macro>

<p>After the first sync completes, subsequent runs will only fetch changed documents (incremental sync).</p>"#,
            5,
            "Emma Andersson",
            "2026-01-20T14:00:00.000+0000",
            "2026-04-05T11:00:00.000+0000",
            &[("100001", "Quelch Overview")],
            &["getting-started", "installation", "tutorial"],
            base_url,
        ),
        page(
            "100003",
            "Configuration Reference",
            r#"<h1>Configuration Reference</h1>
<p>Quelch is configured via a YAML file (default: <code>quelch.yaml</code>). All string values support <code>${ENV_VAR}</code> syntax for environment variable substitution.</p>

<h2>Top-Level Structure</h2>
<ac:structured-macro ac:name="code"><ac:parameter ac:name="language">yaml</ac:parameter><ac:plain-text-body><![CDATA[azure:
  endpoint: "https://..."    # Required: Azure AI Search endpoint URL
  api_key: "${API_KEY}"      # Required: Admin API key

sources:                     # Required: One or more source definitions
  - type: jira               # or "confluence"
    name: "unique-name"      # Required: Unique identifier for this source
    ...

sync:                        # Optional: Sync behavior overrides
  poll_interval: 300
  batch_size: 100
  max_concurrent_per_credential: 3
  state_file: ".quelch-state.json"]]></ac:plain-text-body></ac:structured-macro>

<h2>Jira Source Configuration</h2>
<p>Configure a Jira source (Cloud or Data Center):</p>
<ac:structured-macro ac:name="code"><ac:parameter ac:name="language">yaml</ac:parameter><ac:plain-text-body><![CDATA[- type: jira
  name: "my-jira"
  url: "https://jira.company.com"     # Base URL (no trailing slash)
  auth:
    # Data Center: use PAT
    pat: "${JIRA_PAT}"
    # Cloud: use email + api_token
    # email: "${JIRA_EMAIL}"
    # api_token: "${JIRA_TOKEN}"
  projects:
    - "PROJ"
    - "HR"
  index: "jira-issues"]]></ac:plain-text-body></ac:structured-macro>

<h2>Confluence Source Configuration</h2>
<ac:structured-macro ac:name="code"><ac:parameter ac:name="language">yaml</ac:parameter><ac:plain-text-body><![CDATA[- type: confluence
  name: "my-confluence"
  url: "https://confluence.company.com"
  auth:
    pat: "${CONFLUENCE_PAT}"
  spaces:
    - "ENG"
    - "OPS"
  index: "confluence-pages"]]></ac:plain-text-body></ac:structured-macro>

<h2>Sync Options</h2>
<p>All sync options are optional and have sensible defaults:</p>
<ul>
<li><strong>poll_interval</strong> (default: 300) - Seconds between sync cycles in watch mode</li>
<li><strong>batch_size</strong> (default: 100) - Number of documents to fetch per API call</li>
<li><strong>max_concurrent_per_credential</strong> (default: 3) - Maximum parallel sources sharing same credentials</li>
<li><strong>state_file</strong> (default: ".quelch-state.json") - Path to the sync state persistence file</li>
</ul>

<h2>Authentication Detection</h2>
<p>Quelch automatically detects whether to use Cloud or Data Center authentication based on which fields are present in the auth block. If <code>pat</code> is set, it uses Bearer token auth (DC). If <code>email</code> and <code>api_token</code> are set, it uses Basic auth (Cloud).</p>"#,
            7,
            "Kristofer Liljeblad",
            "2026-02-01T09:00:00.000+0000",
            "2026-04-08T10:30:00.000+0000",
            &[("100001", "Quelch Overview")],
            &["configuration", "reference", "yaml"],
            base_url,
        ),
        page(
            "100004",
            "Jira Connector",
            r#"<h1>Jira Connector</h1>
<p>The Jira connector fetches issues from Jira Cloud or Data Center and indexes them into Azure AI Search. It supports both REST API v2 (Data Center) and v3 (Cloud) with automatic detection.</p>

<h2>How It Works</h2>
<p>On each sync cycle, the connector:</p>
<ol>
<li>Builds a JQL query with project filter and optional updated-since cursor</li>
<li>Fetches issues in batches using the search API</li>
<li>Maps Jira fields to the Azure AI Search document schema</li>
<li>Uploads documents to the configured index</li>
<li>Updates the sync cursor to the latest updated timestamp</li>
</ol>

<h2>Cloud vs Data Center Differences</h2>
<p>The connector handles key differences between Cloud and DC transparently:</p>
<ul>
<li><strong>Authentication:</strong> Cloud uses Basic auth (email:token), DC uses Bearer token (PAT)</li>
<li><strong>API Version:</strong> Cloud uses /rest/api/3/search/jql, DC uses /rest/api/2/search</li>
<li><strong>Pagination:</strong> Cloud uses cursor-based (nextPageToken), DC uses offset-based (startAt)</li>
<li><strong>Description Format:</strong> Cloud returns ADF (Atlassian Document Format) JSON, DC returns plain text or wiki markup</li>
<li><strong>Comment Format:</strong> Same as description - ADF on Cloud, plain text on DC</li>
</ul>

<h2>Indexed Fields</h2>
<p>Each Jira issue is indexed with these fields:</p>
<ul>
<li><code>id</code> - Unique document ID: {source_name}-{issue_key}</li>
<li><code>url</code> - Browse URL back to the original issue</li>
<li><code>source_name</code>, <code>source_type</code> - Source identification</li>
<li><code>project</code>, <code>issue_key</code> - Jira project and issue key</li>
<li><code>issue_type</code> - Story, Bug, Task, etc.</li>
<li><code>summary</code>, <code>description</code> - Issue content</li>
<li><code>status</code>, <code>status_category</code> - Current status and category</li>
<li><code>priority</code> - Issue priority level</li>
<li><code>assignee</code>, <code>reporter</code> - People fields</li>
<li><code>labels</code> - Issue labels array</li>
<li><code>comments</code> - All comments concatenated as text</li>
<li><code>content</code> - Full searchable content (summary + description + comments)</li>
<li><code>created_at</code>, <code>updated_at</code> - Timestamps in RFC 3339 format</li>
</ul>

<h2>JQL Generation</h2>
<p>The connector builds JQL automatically from your config. For a project "PROJ" with an incremental cursor, it generates:</p>
<ac:structured-macro ac:name="code"><ac:parameter ac:name="language">sql</ac:parameter><ac:plain-text-body><![CDATA[project = PROJ AND updated >= "2026-03-15 14:30" ORDER BY updated ASC]]></ac:plain-text-body></ac:structured-macro>
<p>Multiple projects are combined with OR and wrapped in parentheses.</p>"#,
            4,
            "Lars Svensson",
            "2026-02-10T10:00:00.000+0000",
            "2026-03-20T14:00:00.000+0000",
            &[("100001", "Quelch Overview")],
            &["jira", "connector", "api"],
            base_url,
        ),
        page(
            "100005",
            "Confluence Connector",
            r#"<h1>Confluence Connector</h1>
<p>The Confluence connector fetches pages from Confluence Cloud or Data Center and indexes them into Azure AI Search. Long pages are automatically chunked by heading for better search relevance.</p>

<h2>How It Works</h2>
<p>On each sync cycle, the connector:</p>
<ol>
<li>Builds a CQL query with space filter and optional lastmodified cursor</li>
<li>Fetches pages via the content search API with expanded fields</li>
<li>Converts XHTML storage format body to plain text</li>
<li>Chunks long pages by heading (h1/h2/h3) boundaries</li>
<li>Uploads document chunks to Azure AI Search</li>
<li>Updates the sync cursor</li>
</ol>

<h2>Chunking Strategy</h2>
<p>Confluence pages can be very long. To improve search relevance, quelch splits pages into chunks:</p>
<ul>
<li><strong>Primary: Heading-based splitting</strong> - Pages are split at h1, h2, and h3 tags. Each section becomes a separate search document with the heading as part of the title.</li>
<li><strong>Fallback: Fixed-size splitting</strong> - Pages without headings are split into ~4000 character chunks with 200 character overlap to preserve context at boundaries.</li>
</ul>
<p>Each chunk gets a unique document ID: <code>{source_name}-{page_id}-chunk-{index}</code></p>

<h2>XHTML Handling</h2>
<p>Confluence stores page content in XHTML storage format, which includes:</p>
<ul>
<li>Standard HTML tags (h1-h6, p, ul, ol, li, table, etc.)</li>
<li>Confluence-specific macros (<code>ac:structured-macro</code>)</li>
<li>CDATA sections wrapping code block content</li>
<li>HTML entities (&amp;amp;, &amp;lt;, &amp;nbsp;, etc.)</li>
</ul>
<p>The strip_html function handles all of these, extracting clean plain text suitable for search indexing. CDATA content is preserved, HTML tags are stripped, and entities are decoded.</p>

<h2>CQL Generation</h2>
<p>For a space "ENG" with an incremental cursor:</p>
<ac:structured-macro ac:name="code"><ac:parameter ac:name="language">sql</ac:parameter><ac:plain-text-body><![CDATA[space = "ENG" AND lastmodified >= "2026-03-15 14:30" ORDER BY lastmodified ASC]]></ac:plain-text-body></ac:structured-macro>

<h2>Expanded Fields</h2>
<p>The connector requests these expansions: <code>body.storage,version,history,ancestors,metadata.labels,space</code>. This provides all the metadata needed for rich search documents including author information, parent page hierarchy, and content labels.</p>"#,
            4,
            "Lars Svensson",
            "2026-02-15T11:00:00.000+0000",
            "2026-03-25T16:00:00.000+0000",
            &[("100001", "Quelch Overview")],
            &["confluence", "connector", "chunking"],
            base_url,
        ),
        page(
            "100006",
            "Azure AI Search Integration",
            r#"<h1>Azure AI Search Integration</h1>
<p>Quelch pushes documents directly to Azure AI Search using its REST API. This page documents the index schemas, document format, and API usage patterns.</p>

<h2>Index Schemas</h2>
<p>Quelch creates two types of indexes, one for Jira and one for Confluence. Both share common fields but have source-specific additions.</p>

<h3>Common Fields</h3>
<ul>
<li><code>id</code> (Edm.String, key) - Unique document identifier</li>
<li><code>url</code> (Edm.String, filterable) - Link back to source</li>
<li><code>source_name</code> (Edm.String, filterable, facetable) - Source config name</li>
<li><code>source_type</code> (Edm.String, filterable, facetable) - "jira" or "confluence"</li>
<li><code>content</code> (Edm.String, searchable) - Full text content for search</li>
<li><code>created_at</code> (Edm.DateTimeOffset, sortable) - Creation timestamp</li>
<li><code>updated_at</code> (Edm.DateTimeOffset, sortable, filterable) - Last update timestamp</li>
</ul>

<h3>Jira-Specific Fields</h3>
<ul>
<li><code>project</code>, <code>issue_key</code>, <code>issue_type</code>, <code>summary</code>, <code>description</code></li>
<li><code>status</code>, <code>status_category</code>, <code>priority</code></li>
<li><code>assignee</code>, <code>reporter</code>, <code>labels</code>, <code>comments</code></li>
</ul>

<h3>Confluence-Specific Fields</h3>
<ul>
<li><code>space_key</code>, <code>page_title</code>, <code>chunk_title</code></li>
<li><code>labels</code>, <code>ancestors</code></li>
</ul>

<h2>Document Upload</h2>
<p>Documents are uploaded using the Azure AI Search index documents API:</p>
<ac:structured-macro ac:name="code"><ac:parameter ac:name="language">http</ac:parameter><ac:plain-text-body><![CDATA[POST https://{service}.search.windows.net/indexes/{index}/docs/index?api-version=2024-07-01
Content-Type: application/json
api-key: {admin-key}

{
  "value": [
    { "@search.action": "mergeOrUpload", "id": "...", ... }
  ]
}]]></ac:plain-text-body></ac:structured-macro>
<p>Quelch uses <code>mergeOrUpload</code> action to create new documents or update existing ones. Documents are sent in batches for efficiency.</p>

<h2>Index Creation</h2>
<p>The <code>quelch setup</code> command creates indexes with optimized field configurations. Searchable fields use the standard Lucene analyzer. Filterable and facetable fields are configured for efficient filtering in search queries.</p>"#,
            5,
            "Sofia Eriksson",
            "2026-02-20T10:00:00.000+0000",
            "2026-03-30T13:00:00.000+0000",
            &[("100001", "Quelch Overview")],
            &["azure", "search", "index", "schema"],
            base_url,
        ),
        page(
            "100007",
            "Troubleshooting",
            r#"<h1>Troubleshooting</h1>
<p>This page covers common errors and their solutions when using quelch.</p>

<h2>Authentication Errors</h2>
<h3>401 Unauthorized from Jira/Confluence</h3>
<p>The most common cause is an expired or invalid token. Check that:</p>
<ul>
<li>Your PAT (Data Center) or API token (Cloud) is still valid</li>
<li>The token has the necessary permissions to read issues/pages</li>
<li>Environment variables referenced in your config are set correctly</li>
<li>For Cloud: both email and api_token must be provided</li>
</ul>
<p>Run <code>quelch validate</code> to verify your config loads correctly.</p>

<h3>403 Forbidden</h3>
<p>Your token is valid but lacks permissions. Ensure the token owner has read access to the projects/spaces configured in quelch.yaml.</p>

<h2>Azure AI Search Errors</h2>
<h3>Index Not Found (404)</h3>
<p>Run <code>quelch setup --yes</code> to create missing indexes before syncing.</p>

<h3>Request Entity Too Large (413)</h3>
<p>A document exceeds Azure's size limit. This typically happens with very large Confluence pages. Quelch's chunking should prevent this, but if you encounter it, check for pages with extremely large embedded content (images encoded as base64, etc.).</p>

<h3>Service Unavailable (503)</h3>
<p>Azure AI Search is temporarily overloaded. Quelch will automatically retry with exponential backoff (up to 5 retries). If it persists, check your Azure service tier and scaling settings.</p>

<h2>Sync Issues</h2>
<h3>Sync appears stuck or very slow</h3>
<p>Check the batch size in your config. The default of 100 works well for most cases. For very large Jira projects (10k+ issues), the first full sync may take several minutes. Use <code>-v</code> flag for debug logging to see progress.</p>

<h3>Documents missing after sync</h3>
<p>Verify that all target projects/spaces are listed in your config. Quelch only syncs projects and spaces explicitly configured. Also check that the user associated with your token has access to those projects.</p>

<h3>State file issues</h3>
<p>If sync state seems corrupted, run <code>quelch reset</code> to clear all state and force a full re-sync on the next run. You can also reset a specific source: <code>quelch reset my-jira</code>.</p>"#,
            6,
            "Emma Andersson",
            "2026-03-01T10:00:00.000+0000",
            "2026-04-10T09:30:00.000+0000",
            &[("100001", "Quelch Overview")],
            &["troubleshooting", "errors", "faq"],
            base_url,
        ),
        page(
            "100008",
            "Architecture & Design Decisions",
            r#"<h1>Architecture &amp; Design Decisions</h1>
<p>This page documents the key architectural decisions made during quelch development and the reasoning behind them.</p>

<h2>Why Direct Ingest?</h2>
<p>Quelch pushes data directly from source APIs to Azure AI Search without any intermediate storage (no database, no message queue, no S3 bucket). This was a deliberate choice:</p>
<ul>
<li><strong>Simplicity:</strong> No infrastructure to deploy or manage beyond Azure AI Search itself</li>
<li><strong>Freshness:</strong> Data goes straight from source to search index with minimal latency</li>
<li><strong>Cost:</strong> No additional storage or compute costs for intermediate layers</li>
<li><strong>Reliability:</strong> Fewer moving parts means fewer failure modes</li>
</ul>
<p>The tradeoff is that quelch must handle all transformation inline. This is acceptable because the transformations (HTML stripping, field mapping) are lightweight CPU operations.</p>

<h2>Why No Intermediate Storage?</h2>
<p>Many ETL tools use a staging area (database, S3, etc.) between extraction and loading. We skip this because:</p>
<ul>
<li>Azure AI Search is the only consumer of the data</li>
<li>Source APIs provide reliable pagination and incremental fetch</li>
<li>Crash-safe state persistence means we can resume without re-fetching everything</li>
<li>The data volume (thousands of issues/pages, not millions) fits comfortably in memory per batch</li>
</ul>

<h2>Concurrency Model</h2>
<p>Quelch uses tokio for async I/O with a semaphore-based concurrency model:</p>
<ul>
<li>Each source runs as an independent async task</li>
<li>Sources sharing the same credentials are limited by <code>max_concurrent_per_credential</code></li>
<li>Sources with different credentials can run fully in parallel</li>
<li>Within a source, fetching and uploading alternate (fetch batch, upload batch) to control memory usage</li>
</ul>

<h2>Why Rust?</h2>
<p>Quelch is built in Rust for several reasons:</p>
<ul>
<li><strong>Single binary:</strong> No runtime dependencies, easy to distribute and deploy</li>
<li><strong>Performance:</strong> Fast startup, low memory usage, efficient async I/O</li>
<li><strong>Reliability:</strong> Type system catches many errors at compile time</li>
<li><strong>Ecosystem:</strong> Excellent crates for HTTP (reqwest), async (tokio), CLI (clap), serialization (serde)</li>
</ul>

<h2>State Management</h2>
<p>Sync state is persisted as a JSON file after every batch. The state tracks per-source cursors (last updated timestamp), sync counts, and document counts. The file uses atomic writes (write to temp file, then rename) to prevent corruption from crashes. On startup, if the state file is unreadable, quelch falls back to a full sync rather than failing.</p>"#,
            3,
            "Kristofer Liljeblad",
            "2026-02-25T14:00:00.000+0000",
            "2026-04-12T16:00:00.000+0000",
            &[("100001", "Quelch Overview")],
            &["architecture", "design", "adr"],
            base_url,
        ),
    ]
}

// ---------------------------------------------------------------------------
// Helper: build a single Jira issue JSON value
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn issue(
    key: &str,
    id: &str,
    summary: &str,
    description: &str,
    issue_type: &str,
    status: &str,
    status_category: &str,
    priority: &str,
    assignee: &str,
    reporter: &str,
    labels: &[&str],
    created: &str,
    updated: &str,
    comments: &[(&str, &str, &str)], // (author, created, body)
) -> Value {
    let comment_values: Vec<Value> = comments
        .iter()
        .map(|(author, created, body)| {
            json!({
                "self": format!("http://localhost:9999/jira/rest/api/2/issue/{}/comment/1", key),
                "author": {
                    "displayName": author,
                    "name": author.to_lowercase().replace(' ', "."),
                    "active": true
                },
                "body": body,
                "created": created,
                "updated": created
            })
        })
        .collect();

    json!({
        "expand": "renderedFields,names,schema,operations,editmeta,changelog,versionedRepresentations",
        "id": id,
        "self": format!("http://localhost:9999/jira/rest/api/2/issue/{}", id),
        "key": key,
        "fields": {
            "summary": summary,
            "description": description,
            "status": {
                "self": format!("http://localhost:9999/jira/rest/api/2/status/{}", status.to_lowercase().replace(' ', "-")),
                "name": status,
                "statusCategory": {
                    "self": "http://localhost:9999/jira/rest/api/2/statuscategory/3",
                    "id": match status_category {
                        "Done" => 3,
                        "In Progress" => 4,
                        _ => 2,
                    },
                    "key": match status_category {
                        "Done" => "done",
                        "In Progress" => "indeterminate",
                        _ => "new",
                    },
                    "name": status_category
                }
            },
            "priority": {
                "self": format!("http://localhost:9999/jira/rest/api/2/priority/{}", priority.to_lowercase()),
                "name": priority,
                "id": match priority {
                    "High" => "2",
                    "Medium" => "3",
                    "Low" => "4",
                    _ => "3",
                }
            },
            "issuetype": {
                "self": format!("http://localhost:9999/jira/rest/api/2/issuetype/{}", issue_type.to_lowercase()),
                "name": issue_type,
                "subtask": false
            },
            "assignee": {
                "displayName": assignee,
                "name": assignee.to_lowercase().replace(' ', "."),
                "active": true
            },
            "reporter": {
                "displayName": reporter,
                "name": reporter.to_lowercase().replace(' ', "."),
                "active": true
            },
            "project": {
                "self": "http://localhost:9999/jira/rest/api/2/project/10000",
                "id": "10000",
                "key": "QUELCH",
                "name": "Quelch"
            },
            "labels": labels,
            "created": created,
            "updated": updated,
            "comment": {
                "comments": comment_values,
                "maxResults": comment_values.len(),
                "total": comment_values.len(),
                "startAt": 0
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Helper: build a single Confluence page JSON value
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn page(
    id: &str,
    title: &str,
    body_html: &str,
    version: u32,
    author: &str,
    created: &str,
    updated: &str,
    ancestors: &[(&str, &str)], // (id, title)
    labels: &[&str],
    base_url: &str,
) -> Value {
    let ancestor_values: Vec<Value> = ancestors
        .iter()
        .map(|(anc_id, anc_title)| {
            json!({
                "id": anc_id,
                "type": "page",
                "status": "current",
                "title": anc_title,
                "_links": {
                    "webui": format!("/display/QUELCH/{}", anc_title.replace(' ', "+")),
                    "self": format!("{}/rest/api/content/{}", base_url, anc_id)
                }
            })
        })
        .collect();

    let label_values: Vec<Value> = labels
        .iter()
        .map(|name| {
            json!({
                "prefix": "global",
                "name": name,
                "id": format!("label-{}", name)
            })
        })
        .collect();

    let webui_path = format!("/display/QUELCH/{}", title.replace(' ', "+"));

    json!({
        "id": id,
        "type": "page",
        "status": "current",
        "title": title,
        "space": {
            "id": 1,
            "key": "QUELCH",
            "name": "Quelch Documentation",
            "type": "global",
            "_links": {
                "self": format!("{}/rest/api/space/QUELCH", base_url)
            }
        },
        "body": {
            "storage": {
                "value": body_html,
                "representation": "storage"
            }
        },
        "version": {
            "number": version,
            "when": updated,
            "by": {
                "displayName": author,
                "username": author.to_lowercase().replace(' ', "."),
                "type": "known"
            },
            "message": ""
        },
        "history": {
            "createdBy": {
                "displayName": author,
                "username": author.to_lowercase().replace(' ', "."),
                "type": "known"
            },
            "createdDate": created,
            "latest": true
        },
        "ancestors": ancestor_values,
        "metadata": {
            "labels": {
                "results": label_values,
                "start": 0,
                "limit": 200,
                "size": labels.len()
            }
        },
        "_links": {
            "self": format!("{}/rest/api/content/{}", base_url, id),
            "webui": webui_path,
            "tinyui": format!("/x/{}", id),
            "collection": format!("{}/rest/api/content", base_url)
        }
    })
}
