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

**Response:** "The last released iXX firmware was 2.7.0 (released 2026-04-09). Release notes: https://confluence.example/.../iXX-2.7.0-release-notes . Top changes: ..."

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

## Pattern summary

If you read all ten, the patterns repeat:

- **Exhaustive listing** → `query` with cursor pagination.
- **Counts and totals** → `aggregate`.
- **Fuzzy / cross-document themes** → `search`.
- **Domain concepts (sprints, fix versions, spaces)** → `query` against the companion data source first, then a follow-up call.
- **Two-step "resolve then fetch"** is the most common shape.
- **Always include `source_link`s** in the answer.

Generated bundles ([agent-generation.md](agent-generation.md)) encode all of these as patterns so the assistant doesn't have to re-derive them every conversation.
