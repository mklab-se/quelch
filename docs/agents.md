# Agent generation

`quelch agent generate` produces a copy-pasteable bundle of agent-side material — system prompts, tool descriptions, connection details, schema cheatsheets — tailored to your actual deployment.

The point: agents work much better when their instructions are grounded in the *real* schema and the *real* MCP URL of the *real* deployment, not generic boilerplate. Quelch already knows all of that, so it generates it.

## Why this matters

A bare MCP server gives an agent five tools and a list of containers. That's enough to be functional but not enough to be good. To answer "what's planned for the next sprint?" the agent has to know:

- Sprints are a separate Cosmos container called `jira-sprints`.
- The "next" sprint is the future-state sprint with the earliest start date.
- "Planned in" means the issue's `sprint.id` matches.
- "Planned" usually means Stories, Tasks, and Bugs — not Epics or Sub-tasks.

A good agent system prompt encodes this domain knowledge. `quelch agent generate` writes that prompt for you, with the right index names, the right field names, and the right connection details.

## Usage

```bash
quelch agent generate --target <platform> [--output <dir>]
```

Targets:

- `copilot-studio` — Microsoft 365 Copilot Studio.
- `copilot-cli` — GitHub Copilot CLI.
- `vscode-mcp` — VS Code MCP integration.
- `claude-code` — Anthropic Claude Code (`.mcp.json`).
- `markdown` — generic copy-paste-able bundle.

Default `--output` is `./agent-bundle/`.

## What's in a bundle

Every bundle, regardless of target, contains five sections — packaged as files appropriate to the target.

### 1. Connection details

For `vscode-mcp` / `claude-code` / `copilot-cli`, this is a `.mcp.json` snippet you paste into your MCP config:

```jsonc
{
  "mcpServers": {
    "quelch-prod": {
      "type": "streamable-http",
      "url": "https://quelch-prod-mcp.swedencentral.azurecontainerapps.io",
      "headers": {
        "Authorization": "Bearer ${QUELCH_API_KEY}"
      }
    }
  }
}
```

The actual API key value is **not** written to the bundle; the bundle's README explains how to fetch it from Key Vault and set the env var.

### 2. Tool reference

A markdown file describing each tool — `search`, `query`, `get`, `list_sources`, `aggregate` — including a short "when to use" guide. This is identical to [mcp-api.md](mcp-api.md), trimmed to what's relevant to *this* deployment (e.g. only the containers actually exposed).

### 3. Schema cheatsheet

A compact reference of every container exposed by the deployment, with field names, types, common enum values, and example filter expressions. Quelch generates this from the live `list_sources` output of the deployed MCP, so it reflects reality, not a static guess.

Example excerpt:

```markdown
## jira-issues (Jira issues — DO project, INT project)

Fields:
| Field | Type | Notes |
|---|---|---|
| `key` | string | E.g. `DO-1234`. Matches `<project_key>-<number>`. |
| `project_key` | string | Always one of: `DO`, `INT`, `PROD`. |
| `type` | string | One of: `Story`, `Task`, `Bug`, `Epic`, `Sub-task`. |
| `status` | string | Typically: `Open`, `In Progress`, `In Review`, `Done`. |
| `assignee.email` | string | Use this for "issues assigned to ..." queries. |
| `sprint.id` | string | Reference into `jira-sprints` container. |
| `sprint.state` | string | `active`, `future`, or `closed`. |
| `story_points` | integer | Custom field. May be null. |
| `fix_versions[].name` | string | E.g. `iXX-2.7.0`. |
| `created` / `updated` | datetime | UTC ISO 8601. |

Example filters:
- All my open Stories in DO:
  `query(container="jira-issues", where={ project_key:"DO", type:"Story",
   status:{not:"Done"}, "assignee.email":"<me>" })`
- Issues in the active sprint:
  `query(container="jira-issues", where={ "sprint.state":"active",
   project_key:"DO" })`
```

### 4. Domain how-tos

Patterns that recur in real questions. Generated patterns include:

- **"All matching X" — exhaustive results.** Always paginate with `cursor` until `next_cursor` is null. Surface `total` from the response so the user knows how many.
- **"Counts and totals."** Use `aggregate`, not pagination + client-side counting.
- **"The next sprint" / "the current sprint."** First `query(jira-sprints, where={ project_key:X, state:"active"|"future" }, order_by=start_date asc, top=1)`. Then use the returned id in a second tool call.
- **"Planned in a sprint" — issue type filtering.** Default to Stories, Tasks, Bugs. Exclude Epics and Sub-tasks unless the user asks.
- **"Release notes for version X."** First `query(jira-fix-versions, where={ name:"X" })` to confirm the version exists, then `search(indexes=["confluence-pages"], query="release notes <X>")`.
- **"Cross-team / cross-source comparison."** Use `search` over `confluence-pages` for the documents, then summarise.

### 5. Example prompts

Fully formed example prompts the user can try in their agent — taken directly from the use cases in [examples.md](examples.md).

## Per-target packaging

### `copilot-studio`

Generates the same content packaged for Microsoft 365 Copilot Studio:

```
agent-bundle/
├── README.md                       — what to do, in order
├── agent-instructions.md           — paste into the agent's System Prompt
├── topics/
│   ├── search-jira.yaml            — Copilot Studio topic YAML
│   ├── search-confluence.yaml
│   └── ...
├── connection.md                   — how to register the MCP connector in Copilot Studio
└── prompts.md                      — example user prompts
```

This extends what today's `quelch generate-agent` does in v1.

### `copilot-cli`

```
agent-bundle/
├── README.md
├── mcp-server.json                 — paste into ~/.config/copilot/mcp.json
├── system-prompt.md                — paste into the agent's system prompt
└── prompts.md
```

### `vscode-mcp`

```
agent-bundle/
├── README.md
├── .vscode/mcp.json                — copy into the workspace's .vscode/
├── system-prompt.md
└── prompts.md
```

### `claude-code`

```
agent-bundle/
├── README.md
├── .mcp.json                       — copy into the project root
├── CLAUDE.md                       — Claude Code project instructions
└── prompts.md
```

### `markdown`

Generic, target-agnostic bundle:

```
agent-bundle/
├── README.md
├── connection.md
├── tools.md
├── schema.md
├── howtos.md
└── prompts.md
```

## How agents discover schema at runtime

Even with a generated cheatsheet, an agent should still call `list_sources` once per session to pick up the *current* schema (containers added since the bundle was generated, new enum values, etc.). The generated system prompt explicitly tells the agent to do this.

The cheatsheet primes the LLM; `list_sources` keeps it honest.

## Refreshing a bundle

The bundle is generated, not authored. After significant config or deployment changes — new sources, new exposed containers, sprint name shape changes — regenerate:

```bash
quelch agent generate --target copilot-studio --output ./agent-bundle
git diff ./agent-bundle      # review the deltas
git commit ./agent-bundle    # check it in
```

In your agent platform, refresh the system prompt (or topic, or instructions file) from the regenerated bundle.
