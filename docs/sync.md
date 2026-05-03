# Sync correctness

Incremental sync is the most error-prone part of any system that mirrors an external source. Quelch v1 hit specific bugs here, so this document spells out the v2 algorithm in full detail. If you read only one operational document, read this one.

## The problem

Atlassian APIs (both Jira and Confluence) have a hard, well-known constraint:

- The query filter (`updated >= "..."` in JQL, `lastmodified > "..."` in CQL) honours **minute resolution only** — sub-minute precision in the filter value is silently truncated to the minute.
- The `updated` / `lastModifiedDate` field returned in JSON has **second resolution**.
- Atlassian's own indexes lag — an issue updated at 14:23:55 might not be visible to JQL filtering for a few seconds-to-minutes after.

This mismatch between filter precision and field precision, plus the indexing lag, is where most incremental-sync bugs come from.

The naive algorithm — "remember the max `updated` seen, query everything with `updated >= cursor`" — is wrong on every one of these:

- **Sub-minute precision lost.** Cursor `14:23:45.000Z` becomes JQL `updated >= "2026/04/30 14:23"`, returning every update in the entire 14:23 minute, not just those ≥ :45.
- **Mid-minute advance loses updates.** Sync starts at 14:23:30, queries `updated >= "14:23"`, gets 5 results all timestamped at 14:23:00, advances cursor to 14:23:00 (max seen). Two more updates land at 14:23:55. Next sync queries `updated >= "14:23"` again — fine. But if the cursor advanced to anything `> 14:23:00` (e.g. by interpreting cursor naively in seconds), the next minute-rounded query becomes `>= "14:24"` and the 14:23:55 updates are *lost forever*.
- **Pagination drift.** Long pagination of `updated >= cursor ORDER BY updated DESC` (the default) silently drops items as new updates push them out of the page window.
- **Atlassian-side indexing lag.** Asking about minute `M` while minute `M` is still in progress on Atlassian's side returns an incomplete result; advancing the cursor past `M` then loses whatever lands later in `M`.

## Algorithm

We sync in **closed minute-resolution intervals** with a **safety lag** behind real time. Each cycle covers a fixed window decided at cycle start; the cursor only advances when the entire window has been written successfully.

### Variables

Per `(deployment, source, subsource)` triple, stored in the `quelch-meta` Cosmos container:

| Field | Meaning |
|---|---|
| `last_complete_minute` | UTC instant at exact-minute precision (`YYYY-MM-DDTHH:MM:00Z`). **Semantic: every change with `updated <= last_complete_minute` is durably in Cosmos.** |
| `safety_lag_minutes` | Config value (default `2`). How far behind real time the upper bound stays. Absorbs Atlassian indexing lag and clock drift. |
| `backfill_in_progress` | Bool. True only when we're still running the initial backfill. |
| `backfill_target` | Fixed minute timestamp set at backfill start; the upper bound for the entire backfill. |
| `backfill_last_seen` | `(updated, key)` tuple — the last issue successfully written during backfill. Used to resume after a crash. |
| `last_reconciliation_at` | When we last did a deletion reconciliation pass. |

### Per-cycle steps

```
T_now    = current UTC time
T_target = floor(T_now, minute) - safety_lag_minutes minutes

if T_target <= last_complete_minute:
    nothing new yet → sleep, retry next cycle
    return

window_start = last_complete_minute               # inclusive, exact minute
window_end   = T_target                            # inclusive, exact minute

jql = `project = "{key}"
       AND updated >= "{format(window_start, "yyyy/MM/dd HH:mm")}"
       AND updated <= "{format(window_end,   "yyyy/MM/dd HH:mm")}"
       ORDER BY updated ASC, key ASC`

paginate the result set:
    fetch page (with stable startAt/maxResults)
    for each issue in page:
        upsert to Cosmos by id     # idempotent
    after each successful page:
        (no cursor advance yet — only on full window success)

on full window success:
    last_complete_minute = window_end
    write back to quelch-meta atomically
    
on crash mid-pagination:
    do NOT advance last_complete_minute
    next cycle reruns the entire window (idempotent — upserts are no-ops)
```

### Why this works

**Symmetric minute boundaries.**
Both ends of the window are exact-minute. We never advance past a minute we haven't fully covered. The constraint that JQL is minute-precision is matched exactly.

**Safety lag.**
By the time we ask about minute `M`, minute `M` has been sealed in Atlassian's indexes for `safety_lag_minutes` (default 2). New writes that arrive in `M` after we've moved on are impossible because Atlassian's indexes for `M` are stable.

**Stable ordering and fixed upper bound.**
`ORDER BY updated ASC, key ASC` plus `updated <= window_end` (fixed at cycle start) means pagination walks a stable result set even if the source mutates mid-pagination. New mutations land with `updated > window_end` and fall into the *next* cycle's window automatically.

**Idempotent upserts.**
Re-running an entire window is a no-op. Repeated boundary-minute queries (the minute equal to `window_start` is included in every cycle) cost a tiny bit of bandwidth and are otherwise free.

**Crash safety.**
The cursor only advances on full-window success. A crashed worker re-runs its current window from scratch on the next cycle.

### Trade-offs

- **Floor on freshness.** A brand-new issue takes at least `safety_lag_minutes` to land in Cosmos (and another `search.indexer.schedule.interval` after that to land in the AI Search index). For an agent / search use case this is fine. Real-time use cases (live dashboards) would need webhooks; out of scope for v1.
- **Boundary minute is re-queried.** Every cycle re-queries the minute equal to `last_complete_minute`. Idempotent but a small redundancy — typically a handful of issues per cycle. Acceptable.
- **Long-running cycles don't shift their target.** A cycle whose window contains a million issues runs to completion against the fixed `window_end`. New updates between `window_end` and now land in the *next* cycle's window. Predictable; no slipping.
- **Clock-skew tolerance bounded by `safety_lag_minutes`.** If Atlassian's clock is more than `safety_lag_minutes` ahead of ours, we could advance past minutes that haven't actually settled. Default 2 minutes is comfortably more than typical clock skew.

## Initial backfill

When `last_complete_minute` is unset (new source, or after `quelch reset`), the worker runs a one-time backfill before switching to the steady-state algorithm above.

```
on first cycle for this (source, subsource):
    backfill_target      = floor(T_now, minute) - safety_lag_minutes minutes
    backfill_in_progress = true
    backfill_last_seen   = null
    write to quelch-meta

repeat until full success:
    jql_base = `project = "{key}"
                AND updated <= "{format(backfill_target, "yyyy/MM/dd HH:mm")}"`
    
    if backfill_last_seen is null:
        jql = jql_base + ` ORDER BY updated ASC, key ASC`
    else:
        # resume after the last issue we successfully wrote
        jql = jql_base + `
              AND ((updated > "{backfill_last_seen.updated, second precision}")
                   OR (updated = "{backfill_last_seen.updated, second precision}" AND key > "{backfill_last_seen.key}"))
              ORDER BY updated ASC, key ASC`
    
    paginate, upserting to Cosmos
    after each page:
        backfill_last_seen = (updated, key) of last issue in the page
        write checkpoint to quelch-meta

on full backfill success:
    last_complete_minute  = backfill_target
    backfill_in_progress  = false
    backfill_target       = null
    backfill_last_seen    = null
    write to quelch-meta atomically

on crash:
    next cycle reads backfill_in_progress=true and resumes from backfill_last_seen
    backfill_target stays fixed across resumes — result set remains stable
```

The `(updated, key)` tuple gives stable lex order so resume is precise even when many issues share the same `updated` timestamp. The fixed `backfill_target` ensures the result set the resumed query walks is the *same* result set the original query would have walked.

A subtle consequence: the JQL `updated > X` filter in the resume clause uses **second precision** in the filter string. JQL's per-minute truncation means `updated > "2026/04/30 14:23"` actually means `updated > "2026/04/30 14:23:59.999"`. To compensate, the resume query uses second-precision strings *and accepts that the boundary check returns the resume issue and possibly a few duplicates from the same second*. The `OR (updated = X AND key > Y)` clause re-checks at fine grain. Upserts handle the duplicates.

## Deletions

Atlassian's change feed does not surface deletions. To detect them, Quelch runs a **periodic full reconciliation**:

```
every N cycles (config: ingest.reconcile_every, default 12):
  for each subsource:
      source_ids   = list of all current ids in source (page through full set)
      cosmos_ids   = list of all ids in our Cosmos container for this subsource
      missing_ids  = cosmos_ids - source_ids
      
      for each id in missing_ids:
          set _deleted = true and _deleted_at = now on the Cosmos doc
  
  write last_reconciliation_at = now to quelch-meta
```

Cosmos docs are **soft-deleted** — we keep the document and set a flag. The AI Search Indexer is configured (via rigg-generated indexer settings) with a **soft-delete column policy** mapping `_deleted == true → delete from index`. The Indexer removes them from the search index on its next run.

Why soft delete rather than hard delete:

- **Auditability.** We can answer "what was deleted from Jira this week?" by querying Cosmos.
- **Recovery.** If a reconciliation bug ever wrongly marks too much as deleted, we can clear `_deleted` and the AI Search Indexer puts them back.
- **Compatibility.** This is the canonical Azure pattern for Cosmos → AI Search indexers.

Hard compaction is a future operation (`quelch azure compact`, post-v1) that hard-deletes Cosmos docs whose `_deleted_at` is older than a configurable retention.

### Reconciliation cost

Reconciliation requires listing all ids in the source. For Jira this is a JQL `project = "{key}"` paginating through all issues; for Confluence it's a CQL `space = "{key}"` doing the same. For large projects this is genuinely expensive — millions of API calls' worth of work potentially. The default `reconcile_every: 12` (with `poll_interval: 300s`) means reconciliation runs every ~60 minutes. Tune downward (less frequent) for large projects with low delete rates.

## State stored per (source, subsource)

The full schema of a `quelch-meta` document for a sync cursor:

```json
{
  "id": "ingest-onprem-jira-ak::jira-internal::DO",
  "deployment_name": "ingest-onprem-jira-ak",
  "source_name": "jira-internal",
  "subsource": "DO",

  "last_complete_minute": "2026-04-30T14:23:00Z",
  "documents_synced_total": 12894,
  "last_sync_at": "2026-04-30T14:25:11Z",
  "last_error": null,

  "backfill_in_progress": false,
  "backfill_target": null,
  "backfill_last_seen": null,

  "last_reconciliation_at": "2026-04-30T03:00:00Z",
  "last_reconciliation_deleted": 0,

  "_partition_key": "ingest-onprem-jira-ak"
}
```

That's enough to recover from any crash, audit any sync, and resume any backfill.

## Confluence specifics

The same algorithm applies to Confluence, with two field renames and one API quirk:

- Cursor field is `last_modified_minute` (semantic identical to `last_complete_minute`).
- Filter language is CQL: `space = "{key}" AND lastmodified >= "{...}" AND lastmodified <= "{...}" ORDER BY lastmodified ASC`.
- Confluence's CQL doesn't have a stable secondary sort field comparable to Jira's `key ASC`. We use `id ASC` as the secondary sort. Same shape; same correctness story.
- Confluence pages can be moved between spaces. A page that moves out of an ingested space looks like a delete from the perspective of the source-space subsource — the periodic reconciliation handles it.

## Rate limits and backoff

Atlassian rate-limits source APIs aggressively. Jira Cloud especially is unforgiving — sustained 429s can lock an account for minutes. Ingest workers handle this transparently:

- **Honour `Retry-After`.** When the source responds 429 (or 5xx with `Retry-After`), the worker waits exactly that long before retrying. No exponential overshoot.
- **Exponential backoff for transient 5xx without `Retry-After`.** Starts at 1s, doubles, capped at 60s. Up to 5 retries per request before failing the cycle.
- **Per-source concurrency cap.** Default 1 concurrent in-flight request per source instance — Atlassian's rate limiter is per-account, so concurrency just makes 429s more likely. Tune via `ingest.max_concurrent_per_source` if your account has high quota.
- **Cycle is paused, not abandoned, on 429 storms.** A worker that hits a sustained 429 condition logs at `warn` and waits — it does *not* advance the cursor mid-storm and does *not* burn the rest of `poll_interval` retrying. The next cycle starts fresh.
- **Backfill respects rate limits the same way.** A backfill of a 50K-issue project might genuinely take hours under 429 pressure; `backfill_in_progress` stays true the whole time and the worker survives crashes via `backfill_last_seen`.

If you see your worker stuck in 429 storms, check `quelch azure logs` — every retry is logged at `debug` and every backoff at `info`. Long-term remedies: increase `poll_interval`, narrow `projects:` per worker, or contact Atlassian support to raise quota.

## Configuration knobs

All defaults live under the global `ingest:` section of `quelch.yaml`; overridable per source if needed. See [configuration.md](configuration.md#ingest):

| Knob | Default | What it controls |
|---|---|---|
| `ingest.poll_interval` | `300s` | Cycle cadence — how often a worker tries to advance its window. |
| `ingest.safety_lag_minutes` | `2` | How far behind real time the per-cycle window's upper bound stays. |
| `ingest.batch_size` | `100` | Page size for source API calls. |
| `ingest.reconcile_every` | `12` | Reconciliation runs every Nth cycle. With default `poll_interval`, that's ~60 minutes. |
| `ingest.max_cycle_duration` | `30m` | If a cycle takes longer than this, log a warning. (Won't abort — long cycles are valid for big windows.) |
| `ingest.max_concurrent_per_source` | `1` | In-flight source-API requests per source instance. Atlassian rate-limits per account, so concurrency rarely helps. |
| `ingest.max_retries` | `5` | Per-request retry cap for transient 5xx without `Retry-After`. |

## Operator FAQ

**Q: A worker crashed mid-cycle. What state is the system in?**
A: `last_complete_minute` did not advance. The next cycle reruns the same window. Upserts are idempotent, so repeated documents are written again at no cost. There is no data loss and no inconsistent state.

**Q: A worker ran for 10 minutes due to a huge window. Did we miss anything?**
A: No. `window_end` was fixed at cycle start. Anything that landed in the source between `window_end` and now is in the *next* cycle's window. The 10-minute run covered exactly the window it set out to cover.

**Q: An issue was created and then deleted between two cycles. Will it appear in Cosmos?**
A: Yes briefly, then no. The first cycle picks up the create (it has `updated` in our window). The next reconciliation finds the id in Cosmos but not in the source, sets `_deleted=true`. The AI Search Indexer removes it from the search index. The Cosmos doc lingers as a soft-deleted record until compaction.

**Q: Can two workers safely cover the same subsource?**
A: No, by design. Each `(source, subsource)` is owned by exactly one ingest deployment, validated by `quelch validate`. Two workers writing to the same Cosmos container is fine (upserts handle it) but they'd both incur Atlassian rate-limit pressure for the same data — wasteful, not unsafe.

**Q: I changed `safety_lag_minutes` from 2 to 5. What happens?**
A: The next cycle's `T_target` is computed with the new value. If `last_complete_minute > T_target` (because the cursor was ahead under the old shorter lag), the cycle is a no-op — the cursor doesn't move backward. If `last_complete_minute < T_target` it advances normally with the new lag. Safe to change live.

**Q: I want to force a full re-sync of one subsource.**
A: `quelch reset --source jira-internal --subsource DO`. This clears `last_complete_minute` and the `backfill_*` fields for that one tuple. The next cycle starts with a fresh backfill against the current `T_target`.

**Q: How do I tell what's actually been synced?**
A: `quelch status --deployment <name>` reads `quelch-meta` and shows `last_complete_minute`, `documents_synced_total`, and `last_reconciliation_at` for every (source, subsource) the deployment owns. `--tui` makes it live-updating.
