# End-to-end examples

This document walks through real questions a user would ask an agent, and shows how the agent uses Quelch's MCP tools to answer them. Each example follows the same shape:

1. **The user's question** — in natural language.
2. **What the user expects** — what success looks like.
3. **The agent's plan** — the tool calls it should make.
4. **The MCP calls** — concrete arguments and the shape of the response.

These examples drove the design of the MCP tool set; if the new architecture can't answer them well, something's wrong.

The examples assume a Quelch deployment that exposes the data sources `jira_issues`, `jira_sprints`, `jira_fix_versions`, `jira_projects`, `confluence_pages`, and `confluence_spaces`.

---

## 1. "Give me all my user stories in Jira"

**User expects:** the complete list of active Stories assigned to them — *exactly* all matching, not a top-K sample.

**Agent's plan:** identify "me" (from agent context), then `query` `jira_issues` with the right filter, paginating until the cursor is exhausted.

**MCP calls:**

```jsonc
// Page 1
query({
  data_source: "jira_issues",
  where: {
    "assignee.email": "kristofer@example.com",
    "type": "Story",
    "status": { "not": "Done" }
  },
  order_by: [{ field: "updated", dir: "desc" }],
  top: 100
})
// → { items: [...], next_cursor: "abc123", total: 142 }

// Page 2
query({ ..., cursor: "abc123" })
// → { items: [...], next_cursor: null, total: 142 }
```

**Response to the user:** "You have 142 active Stories assigned to you. Here are all of them, with links: ..."

The agent must use `query`, not `search`, because `search` returns a top-K ranked sample. `query` paginates exhaustively and returns an exact `total`.

---

## 2. "How many Jira issues are there in the DO project?"

**User expects:** an exact number, ideally broken down by status.

**Agent's plan:** `aggregate` for the total, then a second `aggregate` with `group_by: "status"`.

**MCP calls:**

```jsonc
aggregate({
  data_source: "jira_issues",
  where: { "project_key": "DO" }
})
// → { total: { count: 1842, sum: null }, groups: [] }

aggregate({
  data_source: "jira_issues",
  where: { "project_key": "DO" },
  group_by: "status"
})
// → {
//     total: { count: 1842, sum: null },
//     groups: [
//       { key: "Done",        count: 1421, sum: null },
//       { key: "In Progress", count: 187,  sum: null },
//       { key: "Open",        count: 156,  sum: null },
//       { key: "In Review",   count: 78,   sum: null }
//     ]
//   }
```

**Response:** "DO has 1,842 issues — 1,421 Done, 187 In Progress, 156 Open, 78 In Review. Of the 421 active issues..."

---

## 3. "Give me all Jira issues that cover connection problems in our cameras"

**User expects:** every relevant issue — the agent must understand that "connection problems" includes "WiFi disconnects", "camera offline", "intermittent connectivity", etc.

**Agent's plan:** this is the canonical hybrid case. `search` against `jira_issues` with a natural-language query, paginating exhaustively because the user said "all".

**MCP calls:**

```jsonc
// Page 1
search({
  query: "connection problems camera disconnects wifi offline",
  data_sources: ["jira_issues"],
  where: { "status": { "not": "Done" } },
  top: 50
})
// → { items: [...], next_cursor: "def456", total_estimate: 78 }

// Page 2
search({ ..., cursor: "def456" })
// → { items: [...], next_cursor: null, total_estimate: 78 }
```

**Response:** a clickable list of all matching issues, each with its `source_link`, summary, and a short reason why it matched.

The agent should de-emphasise `total_estimate` ("about 78 issues") because semantic-search counts are estimates; if the user demands an exact number, follow up with `aggregate` on the same filter.

---

## 4. "What's planned in the next sprint in the DO project?"

**User expects:** the list of Stories, Tasks, and Bugs scheduled for the next sprint — not Epics, not Sub-tasks.

**Agent's plan:** two-step. First find the sprint id, then list its issues.

**MCP calls:**

```jsonc
// Step 1: identify "next" sprint = the future-state sprint with the earliest start date
query({
  data_source: "jira_sprints",
  where: { "project_key": "DO", "state": "future" },
  order_by: [{ field: "start_date", dir: "asc" }],
  top: 1
})
// → { items: [{ id: "204", name: "DO Sprint 43", start_date: "2026-05-12T00:00:00Z", ... }], total: 4 }

// Step 2: issues in that sprint, restricted to plannable types
query({
  data_source: "jira_issues",
  where: {
    "project_key": "DO",
    "sprint.id": "204",
    "type": ["Story", "Task", "Bug"]
  },
  top: 200
})
// → { items: [...], next_cursor: null, total: 23 }
```

**Response:** "DO Sprint 43 (starts 2026-05-12) has 23 items planned: 14 Stories, 6 Tasks, 3 Bugs. Here they are: ..."

---

## 5. "Summarize the next sprint as a sprint goal"

**User expects:** a concise summary that captures the theme of the planned work.

**Agent's plan:** same first two steps as Example 4 to get the issues, then summarise their `summary` and `description` fields client-side.

**MCP calls:** identical to Example 4. The agent reads the returned `summary` / `description` for each issue and synthesises a goal locally — no further MCP calls needed.

**Response:** "Sprint goal for DO Sprint 43: *Stabilise iXX firmware connectivity and ship the new battery-status diagnostic.* The 23 planned items cluster around three themes: (a) WiFi / cellular reliability fixes, (b) the new battery telemetry feature, (c) a few hardening tasks left over from sprint 42."

---

## 6. "How much work is left to do in the current sprint?"

**User expects:** an exact story-point total for active items in the active sprint.

**Agent's plan:** find the active sprint, then `aggregate` with `sum_field: "story_points"` over its incomplete issues.

**MCP calls:**

```jsonc
// Step 1: active sprint
query({
  data_source: "jira_sprints",
  where: { "project_key": "DO", "state": "active" },
  top: 1
})
// → { items: [{ id: "203", name: "DO Sprint 42", ... }], total: 1 }

// Step 2: sum story points of incomplete issues in it
aggregate({
  data_source: "jira_issues",
  where: {
    "project_key": "DO",
    "sprint.id": "203",
    "status": { "not": "Done" }
  },
  count: true,
  sum_field: "story_points"
})
// → { total: { count: 14, sum: 47 }, groups: [] }

// Step 3 (optional): list those issues so the user can see what's outstanding
query({
  data_source: "jira_issues",
  where: {
    "project_key": "DO", "sprint.id": "203", "status": { "not": "Done" }
  },
  order_by: [{ field: "status", dir: "asc" }],
  top: 50
})
```

**Response:** "47 story points across 14 active issues remain in DO Sprint 42. Outstanding work: ..."

---

## 7. "What are the top 3 risks for the next sprint in the DO project?"

**User expects:** a qualitative analysis grounded in real issues, with references.

**Agent's plan:** retrieve the planned issues (Examples 4–5), then analyse their content for risk indicators — heavy story-point items, unresolved blockers, items with many linked dependencies, items with comments mentioning known risk language.

**MCP calls:**

```jsonc
// Same as Example 4 to get the planned items
query({ data_source: "jira_sprints", where: {...}, top: 1 })
query({ data_source: "jira_issues", where: { "sprint.id": "204", ... }, top: 200 })

// Then a semantic search over the same set for risk language
search({
  query: "blocker dependency unclear scope external risk",
  data_sources: ["jira_issues"],
  where: { "sprint.id": "204" },
  top: 25
})
```

**Response:** "Top 3 risks for DO Sprint 43: (1) DO-1182 *Camera firmware OTA path* — 13 story points, depends on DO-1170 still open in this sprint. (2) DO-1199 *Cellular handoff regression* — semantic match on 'unclear scope' in comments. (3) DO-1210 *Battery telemetry contract* — links to an unfinished spec page in Confluence. [links to all three]"

---

## 8. "I'm looking for the release notes for the last version of iXX camera firmware"

**User expects:** a direct link to the release notes page.

**Agent's plan:** confirm what "last iXX firmware" means via `jira_fix_versions`, then `search` Confluence for release notes mentioning that version.

**MCP calls:**

```jsonc
// Step 1: latest released iXX version
query({
  data_source: "jira_fix_versions",
  where: { "name": { "like": "iXX-%" }, "released": true },
  order_by: [{ field: "release_date", dir: "desc" }],
  top: 1
})
// → { items: [{ name: "iXX-2.7.0", release_date: "2026-04-09T00:00:00Z", ... }], total: 12 }

// Step 2: Confluence release notes for that version
search({
  query: "release notes iXX 2.7.0",
  data_sources: ["confluence_pages"],
  top: 5
})
// → { items: [{ source_link: "https://confluence.example/.../iXX-2.7.0-release-notes", ... }], ... }
```

**Response:** "The last released iXX firmware was 2.7.0 (released 2026-04-09). Release notes: <https://confluence.example/.../iXX-2.7.0-release-notes>. Top changes: ..."

---

## 9. "Across all teams, what are the commonalities and differences in their documented 'way of working'?"

**User expects:** a comparative analysis of multiple Confluence pages, with references.

**Agent's plan:** semantic search across Confluence for "way of working" docs, retrieve full text via `get`, summarise.

**MCP calls:**

```jsonc
// Step 1: discover the docs
search({
  query: "team way of working development process agile practices",
  data_sources: ["confluence_pages"],
  top: 25
})
// → many candidates

// Step 2: pull full content for the top hits
get({ data_source: "confluence_pages", id: "<page-id-1>" })
get({ data_source: "confluence_pages", id: "<page-id-2>" })
// ...
```

**Response:** a table or bulleted comparison ("All four teams use Scrum but only two run pre-refinement; teams A and C use story points while B and D use t-shirt sizes; ..."), with links to each source page.

---

## 10. "What are the top 5 most common blockers that teams have documented in the last 6 months?"

**User expects:** a ranked list of recurring blocker themes, with references.

**Agent's plan:** a blocker can live in either Jira (issues with `status: "Blocked"` or `labels: "blocker"`) or Confluence (retrospective pages, post-mortems). Search both, then aggregate by theme client-side or use `aggregate` to bucket by structured fields.

**MCP calls:**

```jsonc
// Jira-side: issues marked as blocked in the last 6 months, grouped by label
aggregate({
  data_source: "jira_issues",
  where: {
    "or": [
      { "status": "Blocked" },
      { "labels": "blocker" }
    ],
    "created": { "gte": "6 months ago" }
  },
  group_by: "labels",
  top_groups: 20
})

// Confluence-side: semantic search over retrospectives
search({
  query: "blocker blocked impediment retrospective",
  data_sources: ["confluence_pages"],
  where: { "updated": { "gte": "6 months ago" } },
  top: 50
})
```

**Response:** "Top 5 blockers in the last 6 months: (1) *Test environment instability* — 14 Jira issues, 7 retrospective mentions; (2) *Third-party SDK API churn* — 11 + 4; ...". Each item links to the underlying issues and pages.

---

## 11. "Summarize what is written about connection problems in Jira and Confluence"

**User expects:** a coherent summary that pulls from *all* relevant material across both systems, written in the agent's voice, with citations.

**Agent's plan:** one `search` call across both data sources with `include_content: "full"` so the whole document body of each hit comes back in a single round-trip. Then synthesise locally — the calling agent's strength is exactly this kind of synthesis, and it has the user's conversational context to tailor the answer.

**MCP calls:**

```jsonc
search({
  query: "connection problems camera disconnects wifi offline",
  data_sources: ["jira_issues", "confluence_pages"],
  where: { "status": { "not": "Done" } },   // optional — bias toward live material
  top: 30,
  include_content: "full"                    // <-- key: full body per hit, no follow-up get's
})
// → { items: [{ id, score, data_source, source_link, snippet, body, fields }, ...],
//     next_cursor: "...",
//     total_estimate: 78 }
```

**Response:** "Across roughly 78 Jira issues and Confluence pages discussing connection problems, three themes emerge: (a) **Wi-Fi / cellular handoff regressions** in the iXX firmware family — see DO-1199, DO-1182; the architecture rationale is on the *Camera Connectivity Pipeline* page; (b) **Power-state handling during sleep** causing dropped sessions, with a workaround documented on *iXX Power Management*; (c) **NTP drift** as a downstream symptom rather than a root cause — covered briefly on *Field Diagnostics Playbook*. Most active investigation is in DO-1210. [links to all referenced issues and pages]"

The agent reads each `body` and writes a tailored summary. No `get` round-trips, no `summarise` MCP tool, no Layer-2 synthesis-in-MCP — the calling agent does what calling agents are good at: synthesis with audience awareness.

**Alternative (cheap, less control):** if the user wants a quick paragraph and is happy with the Knowledge Base's own synthesis voice:

```jsonc
search({
  query: "connection problems camera disconnects wifi offline",
  data_sources: ["jira_issues", "confluence_pages"],
  include_content: "agentic_answer"
})
// → { answer: "Most discussion of camera connection issues...", citations: [...], items: [...] }
```

Use this when the answer doesn't need to fit the conversation's tone; surface the `answer` and the `citations` directly.

---

## 12. "What changed in the DO project in the last hour?"

**User expects:** every recent update — issues, comments, status changes — over a tight time window. Useful for "catch me up" use cases.

**Agent's plan:** simple `query` with a recency filter on `updated`. Sort descending so the most recent surface first.

**MCP calls:**

```jsonc
query({
  data_source: "jira_issues",
  where: {
    "project_key": "DO",
    "updated": { "gte": "1 hour ago" }
  },
  order_by: [{ field: "updated", dir: "desc" }],
  top: 100
})
// → { items: [...], next_cursor: "...", total: 17 }
```

**Response:** "17 issues in DO have been updated in the last hour. Most recent: DO-1234 *Camera disconnects* moved to In Review (5 min ago); DO-1241 *Battery telemetry* commented by Ana (12 min ago); ... [click any to open in Jira]"

The `updated` field is set on any meaningful Jira mutation — status change, assignment change, new comment, edited description. So a filter on `updated >= 1h ago` reliably catches "recent activity" of any kind.

To extend across both source systems, the agent issues two `query` calls (one per data source) and merges client-side. `query` is single-data-source by design — recency questions are not semantic, so they belong on `query`, not `search`:

```jsonc
query({
  data_source: "jira_issues",
  where: { "updated": { "gte": "1 hour ago" } },
  order_by: [{ field: "updated", dir: "desc" }],
  top: 50
})

query({
  data_source: "confluence_pages",
  where: { "updated": { "gte": "1 hour ago" } },
  order_by: [{ field: "updated", dir: "desc" }],
  top: 50
})
```

The agent merges both result sets and presents them sorted by `updated` desc with the `data_source` field as a label. This costs one extra MCP call but produces exact, exhaustive recency results — `search` is the wrong tool here because there's nothing semantic to rank.

---

## 13. "Show me issues that have been stuck — In Progress for more than 14 days with no recent updates"

**User expects:** a list of stalled work, ranked oldest-first, that the team can use to unstick or close out.

**Agent's plan:** `query` with two time filters — status is currently In Progress, *and* `updated` hasn't moved in 14 days.

**MCP calls:**

```jsonc
query({
  data_source: "jira_issues",
  where: {
    "and": [
      { "project_key": "DO" },
      { "status_category": "In Progress" },        // covers "In Progress", "In Review", etc.
      { "updated": { "lt": "14 days ago" } }
    ]
  },
  order_by: [{ field: "updated", dir: "asc" }],   // oldest stale first
  top: 100
})
// → { items: [...], next_cursor: null, total: 9 }
```

**Response:** "9 issues in DO have been In Progress with no activity for 14+ days: (1) DO-1102 *Field diagnostic batching* — 47 days stale, assignee Ana, 5 story points. (2) DO-1131 *Cellular fallback handling* — 28 days stale, unassigned. ... [click any to open]"

The agent uses `status_category` rather than `status` so the query is robust across Jira workflows that rename "In Progress" or add intermediate statuses (`In Review`, `Code Review`, etc.) — they all carry `status_category: "In Progress"`.

Soft-deleted docs are excluded by default (see [mcp-api.md "Soft-delete handling"](mcp-api.md#soft-delete-handling)). The agent doesn't need to filter `_deleted` explicitly.

---

## 14. "Generate the release notes for iXX-2.7.0"

**User expects:** a structured changelog grouped by issue type, listing every Jira issue tagged with `fix_versions = iXX-2.7.0`, ready to paste into a release announcement.

**Agent's plan:** `aggregate` to get the breakdown by type, then `query` to list each group's issues. Or do it all in one `query` call grouped client-side.

**MCP calls:**

```jsonc
// One query — all issues for the version, sorted by type then key
query({
  data_source: "jira_issues",
  where: {
    "fix_versions[].name": "iXX-2.7.0",
    "resolution": { "not": null }                  // only resolved issues belong in release notes
  },
  order_by: [
    { field: "type", dir: "asc" },
    { field: "key",  dir: "asc" }
  ],
  top: 500
})
// → { items: [...], next_cursor: null, total: 42 }
```

**Response:** a markdown changelog grouped by type:

```markdown
# iXX-2.7.0 release notes

## New features (8)
- DO-1182 — Camera firmware OTA path
- DO-1199 — Cellular handoff for outdoor cameras
- ...

## Bug fixes (29)
- DO-1170 — WiFi reconnect storm after power-cycle
- DO-1188 — Battery reading drift at low temperatures
- ...

## Improvements (5)
- DO-1200 — Faster diagnostic export
- ...
```

The filter on `resolution: { not: null }` excludes issues that were tagged for the version but later moved out without being completed — a real Jira-data hazard.

If the user wants more polish, the agent can pair this with Example 8 (`search confluence_pages "release notes iXX 2.7.0"`) to find an existing manual narrative and merge.

---

## 15. "What's blocking DO-1182?"

**User expects:** the dependency chain — issues that must complete before DO-1182 can ship.

**Agent's plan:** `get` DO-1182, read its `issuelinks`, then `get` (or `query`) for the linked issues to enrich with status. Two-step.

**MCP calls:**

```jsonc
// Step 1: fetch the issue and read its issuelinks
get({
  data_source: "jira_issues",
  id: "jira-internal-DO-1182"
})
// → { document: {
//      key: "DO-1182", summary: "Camera firmware OTA path", ...,
//      issuelinks: [
//        { type: "is blocked by", direction: "inward",  target_key: "DO-1170", target_summary: "WiFi reconnect storm" },
//        { type: "is blocked by", direction: "inward",  target_key: "DO-1175", target_summary: "Battery telemetry contract" },
//        { type: "blocks",        direction: "outward", target_key: "DO-1199", target_summary: "Cellular handoff" }
//      ],
//      ...
//    } }

// Step 2: fetch each blocker to know its current status
query({
  data_source: "jira_issues",
  where: { "key": ["DO-1170", "DO-1175"] },
  top: 10
})
// → { items: [
//      { key: "DO-1170", status: "Done",        resolved: "...", assignee: {...} },
//      { key: "DO-1175", status: "In Progress", resolved: null,  assignee: {...} }
//    ] }
```

**Response:** "DO-1182 is blocked by 2 issues: ✅ DO-1170 *WiFi reconnect storm* — Done (resolved 2 days ago); 🔄 DO-1175 *Battery telemetry contract* — In Progress, assigned to Ana, no recent update. The remaining blocker is DO-1175. [links]"

Filtering with `key: [...]` is membership — the agent asks for several specific keys in one call rather than N gets.

To go deeper (transitive blockers — what blocks the blockers), the agent recurses: take DO-1175's `issuelinks`, repeat. Bound the depth (say, 3 levels) to avoid runaway expansion.

---

## 16. "What projects and spaces do we have? What can I ask about?"

**User expects:** an inventory of available data so they know what's actually queryable. Onboarding question; also useful at the start of a new conversation.

**Agent's plan:** `list_sources` first (data-source inventory + schemas), then `query` the metadata data sources (`jira_projects`, `confluence_spaces`) to enumerate concrete projects/spaces.

**MCP calls:**

```jsonc
list_sources({})
// → { data_sources: [
//      { name: "jira_issues",        kind: "jira_issue",        searchable: true,  source_instances: ["jira-internal", "jira-cloud"], schema: [...] },
//      { name: "jira_sprints",       kind: "jira_sprint",       searchable: false, ... },
//      { name: "jira_fix_versions",  kind: "jira_fix_version",  searchable: false, ... },
//      { name: "jira_projects",      kind: "jira_project",      searchable: false, ... },
//      { name: "confluence_pages",   kind: "confluence_page",   searchable: true,  ... },
//      { name: "confluence_spaces",  kind: "confluence_space",  searchable: false, ... }
//    ] }

query({
  data_source: "jira_projects",
  order_by: [{ field: "key", dir: "asc" }],
  top: 100
})
// → { items: [
//      { key: "DO",   name: "DataOps",         project_type_key: "software" },
//      { key: "INT",  name: "Integrations",    project_type_key: "software" },
//      { key: "PROD", name: "Product Mgmt",    project_type_key: "business" }
//    ], total: 3 }

query({
  data_source: "confluence_spaces",
  order_by: [{ field: "key", dir: "asc" }],
  top: 100
})
// → { items: [
//      { key: "ENG", name: "Engineering",       type: "global" },
//      { key: "OPS", name: "Operations",        type: "global" }
//    ], total: 2 }
```

**Response:** "You have access to 3 Jira projects (DO – DataOps, INT – Integrations, PROD – Product Management) and 2 Confluence spaces (ENG – Engineering, OPS – Operations). Try asking things like:

- 'What's planned for next sprint in DO?'
- 'Find anything we've documented about WiFi reliability'
- 'How many issues are open across all projects?'
- 'Summarise our way-of-working pages'"

This is the canonical "first-call-of-the-conversation" pattern — the agent caches `list_sources` once per session and uses it to ground all subsequent calls. Generated agent bundles ([agent-generation.md](agent-generation.md)) tell the assistant to do this proactively at session start.

---

## Pattern summary

If you read all sixteen, the patterns repeat:

- **Exhaustive listing** → `query` with cursor pagination.
- **Counts and totals** → `aggregate`.
- **Fuzzy / cross-document themes** → `search`.
- **Summarise across many documents** → `search` with `include_content: "full"`; agent synthesises.
- **Recency / freshness** → `query` with `updated >= "X ago"`.
- **Staleness / stuck work** → `query` with `updated < "X ago"` plus a status condition.
- **Domain concepts (sprints, fix versions, spaces)** → `query` against the companion data source first, then a follow-up call.
- **Issue dependencies** → `get` to read `issuelinks`, then `query` with `key: [...]` to enrich.
- **Discovery / "what's available"** → `list_sources` then `query` the metadata data sources.
- **Two-step "resolve then fetch"** is the most common shape.
- **Always include `source_link`s** in the answer.

Generated bundles ([agent-generation.md](agent-generation.md)) encode all of these as patterns so the assistant doesn't have to re-derive them every conversation.
