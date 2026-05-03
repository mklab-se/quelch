# Agent and skill generation

`quelch agent generate` produces a copy-pasteable bundle of agent-side material — system prompts, tool descriptions, connection details, schema cheatsheets — tailored to your actual deployment.

Different agent platforms model "the AI assistant's instructions" differently:

- **Agent platforms** (Microsoft 365 Copilot Studio) want a global agent definition with system instructions and topics. The agent always runs as itself.
- **Skill platforms** (Claude Code, GitHub Copilot CLI, VS Code Copilot Chat) want a focused capability that *activates conditionally* — e.g. "use this when the user asks about Jira / Confluence / sprints / releases". The skill sits dormant until something Quelch-shaped comes up in conversation.

The same generator produces either form. The right form depends on the target.

The point: agents (and skills) work much better when their instructions are grounded in the *real* data sources and the *real* MCP URL of the *real* deployment, not generic boilerplate. Quelch already knows all of that, so it generates it.

## Why this matters

A bare MCP server gives an agent five tools and a list of data sources. That's enough to be functional but not enough to be good. To answer "what's planned for the next sprint?" the agent has to know:

- Sprints are queryable as a separate data source called `jira_sprints`.
- The "next" sprint is the future-state sprint with the earliest start date.
- "Planned in" means the issue's `sprint.id` matches.
- "Planned" usually means Stories, Tasks, and Bugs — not Epics or Sub-tasks.

A good system prompt encodes this domain knowledge. `quelch agent generate` writes that prompt for you, with the right data-source names, the right field names, and the right connection details — all expressed at the MCP layer, with no leaking of physical storage details (containers, indexes, partition keys) the agent should not see.

## Usage

```bash
quelch agent generate --target <platform> [--format <form>] [--output <dir>]
```

Targets and their default form:

| Target | Default `--format` | What gets generated |
|---|---|---|
| `copilot-studio` | `agent` | Agent instructions + topic YAML files |
| `claude-code` | `skill` | A Claude Code skill (frontmatter + instructions) |
| `copilot-cli` | `skill` | A GitHub Copilot CLI skill / prompt |
| `vscode-copilot` | `skill` | VS Code Copilot Chat instructions file |
| `codex` | `skill` | OpenAI Codex `AGENTS.md` + MCP config |
| `markdown` | `both` | Both an agent prompt and a skill, as separate files |

The `--format` flag overrides the default when you want, e.g., a skill for a target whose default is `agent`, or vice versa. Default `--output` is `./agent-bundle/`.

## Agent vs skill — two output forms

### Agent form

Produces a complete, always-on system prompt for an agent that *is* the Quelch interface (or at least has Quelch as a primary capability).

Key properties:

- Long-form system instructions describing role, tools, data sources, domain conventions.
- Always active for that agent.
- The user picks "the Quelch agent" when they want Jira/Confluence answers.

### Skill form

Produces a skill / instruction file that **activates conditionally** when the user mentions Jira-, Confluence-, or knowledge-shaped topics. It does not add latency or context to unrelated conversations.

Key properties:

- A trigger description ("Use when the user asks about Jira issues, Confluence pages, sprints, releases, blockers, sprint planning, or other enterprise knowledge").
- Concise core instructions that load only when triggered.
- Coexists with the user's other skills.

A skill is the right form for general-purpose CLI/IDE assistants where Quelch is *one capability among many*. An agent is the right form when the user wants a focused Quelch-aware persona.

## What's in a bundle

Regardless of form, every bundle contains the same five sections — packaged as files appropriate to the target.

### 1. Connection details

For `vscode-copilot` / `claude-code` / `copilot-cli`, this is a `.mcp.json` snippet you paste into your MCP config:

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

A markdown file describing each tool — `search`, `query`, `get`, `list_sources`, `aggregate` — with a short "when to use" guide. Trimmed to what's relevant to *this* deployment (e.g. only the data sources actually exposed).

### 3. Schema cheatsheet

A compact reference of every data source exposed by the deployment, with field names, types, common enum values, and example calls. Quelch generates this from the live `list_sources` output of the deployed MCP, so it reflects reality, not a static guess.

Example excerpt:

```markdown
## jira_issues (Jira issues — DO project, INT project)

Backed by the configured source instances: jira-internal, jira-cloud.
The MCP server unifies them; queries return matches across both.

Fields:
| Field | Type | Notes |
|---|---|---|
| `key` | string | E.g. `DO-1234`. Matches `<project_key>-<number>`. |
| `project_key` | string | Always one of: `DO`, `INT`, `PROD`. |
| `type` | string | One of: `Story`, `Task`, `Bug`, `Epic`, `Sub-task`. |
| `status` | string | Typically: `Open`, `In Progress`, `In Review`, `Done`. |
| `assignee.email` | string | Use this for "issues assigned to ..." queries. |
| `sprint.id` | string | Reference into the `jira_sprints` data source. |
| `sprint.state` | string | `active`, `future`, or `closed`. |
| `story_points` | integer | Custom field. May be null. |
| `fix_versions[].name` | string | E.g. `iXX-2.7.0`. |
| `source_name` | string | Which configured Jira source: `jira-internal` or `jira-cloud`. |
| `created` / `updated` | datetime | UTC ISO 8601. |

Example calls:
- All my open Stories in DO:
  `query(data_source="jira_issues", where={ project_key:"DO", type:"Story",
   status:{not:"Done"}, "assignee.email":"<me>" })`
- Issues in the active sprint of DO:
  `query(data_source="jira_issues", where={ "sprint.state":"active",
   project_key:"DO" })`
- Only issues from the cloud Jira:
  `query(data_source="jira_issues", where={ source_name:"jira-cloud", ... })`
```

### 4. Domain how-tos

Patterns that recur in real questions:

- **"All matching X" — exhaustive results.** Always paginate with `cursor` until `next_cursor` is null. Surface `total` from the response so the user knows how many.
- **"Counts and totals."** Use `aggregate`, not pagination + client-side counting.
- **"The next sprint" / "the current sprint."** First `query(data_source="jira_sprints", where={ project_key:X, state:"active"|"future" }, order_by=[{field:"start_date", dir:"asc"}], top=1)`. Then use the returned id in a second tool call.
- **"Planned in a sprint" — issue type filtering.** Default to Stories, Tasks, Bugs. Exclude Epics and Sub-tasks unless the user asks.
- **"Release notes for version X."** First `query(data_source="jira_fix_versions", where={ name:"X" })` to confirm the version exists, then `search(data_sources=["confluence_pages"], query="release notes <X>")`.
- **"Cross-team / cross-source comparison."** Use `search` over `confluence_pages` for the documents, then summarise.

### 5. Example prompts

Fully formed example prompts the user can try, taken directly from the use cases in [examples.md](examples.md).

## Per-target packaging

### `copilot-studio` (agent form)

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

### `claude-code` (skill form)

```
agent-bundle/
├── README.md
├── .claude/
│   └── skills/
│       └── quelch/
│           └── SKILL.md            — frontmatter (name, description) + instructions
├── .mcp.json                       — copy into the project root
└── prompts.md
```

`SKILL.md` looks roughly like:

```markdown
---
name: quelch
description: |
  Use when the user asks about Jira issues, Confluence pages, sprints,
  releases, blockers, or any other enterprise knowledge. Connect to the
  configured Quelch MCP server and answer using its tools.
---

# Quelch — enterprise knowledge skill

You have access to a Quelch MCP server. Use these tools:
... (tool descriptions, schema cheatsheet, domain how-tos)
```

### `copilot-cli` (skill form)

```
agent-bundle/
├── README.md
├── mcp-server.json                 — paste into ~/.config/copilot/mcp.json
├── skill.md                        — Copilot CLI skill / prompt extension
└── prompts.md
```

### `vscode-copilot` (skill form)

```
agent-bundle/
├── README.md
├── .vscode/mcp.json                — copy into the workspace's .vscode/
├── .github/
│   └── copilot-instructions.md     — VS Code Copilot Chat instructions (skill-shaped)
└── prompts.md
```

### `codex` (skill form)

OpenAI Codex picks up a project-local `AGENTS.md` (the cross-vendor convention shared with several other CLI agents) and reads MCP servers from its standard config:

```
agent-bundle/
├── README.md
├── AGENTS.md                       — project-local Codex instructions (skill-shaped)
├── codex-mcp.toml                  — MCP server entry (paste into ~/.codex/config.toml)
└── prompts.md
```

The `AGENTS.md` has a clear "use this when…" trigger phrasing at the top, the schema cheatsheet and domain how-tos beneath, and the connection details. If your repo already has an `AGENTS.md` for Codex, the bundle's file is named `AGENTS.quelch.md` instead and the README explains how to merge it.

### `markdown` (both forms)

Generic, target-agnostic bundle producing both:

```
agent-bundle/
├── README.md
├── connection.md
├── tools.md
├── schema.md
├── howtos.md
├── agent-prompt.md                 — long-form, always-on (paste into an agent platform)
├── skill.md                        — frontmatter + instructions (paste into a skills system)
└── prompts.md
```

## What's deliberately *not* in a bundle

The generated material describes the system at the **MCP layer only**. It does not mention:

- Cosmos DB or any container names.
- Azure AI Search or any index names.
- Bicep, resource group names, subscription ids, or any other Azure resource detail.

If your agent's instructions ever reference any of those, that's a sign the abstraction has leaked and the bundle should be regenerated from a corrected Quelch.

## How agents/skills discover schema at runtime

Even with a generated cheatsheet, a good agent or skill should still call `list_sources` once per session to pick up the *current* schema (data sources added since the bundle was generated, new enum values, etc.). The generated instructions explicitly tell the assistant to do this.

The cheatsheet primes the LLM; `list_sources` keeps it honest.

## Refreshing a bundle

The bundle is generated, not authored. After significant config or deployment changes — new sources, new exposed data sources, sprint name shape changes — regenerate:

```bash
quelch agent generate --target claude-code --output ./agent-bundle
git diff ./agent-bundle      # review the deltas
git commit ./agent-bundle    # check it in
```

In your agent platform (or skills system), refresh the instructions from the regenerated bundle.
