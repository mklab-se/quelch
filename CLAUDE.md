# Quelch

Quelch ingests data from external sources (Jira, Confluence) into **Cosmos DB** as the system of record, lets **Azure AI Search** index it (via the embedded [rigg](https://github.com/mklab-se/rigg) library — indexes, skillsets, indexers, knowledge sources, knowledge bases), and exposes a **five-tool MCP server** (Streamable HTTP) that agents call directly. The MCP fans out per tool: `search` → AI Search **Knowledge Base** (Agentic Retrieval); `query` / `get` / `aggregate` → Cosmos DB; `list_sources` → cached schema catalog.

## Build & Test Commands

```bash
cargo build --workspace          # Build all crates
cargo test --workspace           # Run all tests
cargo clippy --workspace -- -D warnings  # Lint
cargo fmt --all -- --check       # Check formatting
cargo fmt --all                  # Fix formatting
cargo run -p quelch -- --help    # Run the CLI
```

## Pre-Push Verification (REQUIRED)

Before pushing any changes, always run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

## Architecture

Single-crate workspace: `crates/quelch/`.

```
crates/quelch/src/
├── main.rs            # CLI entry point, clap setup
├── cli.rs             # CLI arg definitions
├── config/            # YAML config loading, validation, slicing, data-source resolution
├── sources/           # SourceConnector trait + Jira/Confluence connectors
├── ingest/            # Per-cycle algorithm, backfill resume, deletion reconciliation, worker
├── cosmos/            # Cosmos DB client (real + in-memory test backend), cursor state
├── mcp/               # Streamable HTTP server, 5 tools, where-grammar parser, expose filter
├── azure/
│   ├── deploy/        # Bicep generator, az shell-outs (plan/deploy/indexer/logs/destroy)
│   └── rigg/          # Generates rigg files from quelch.yaml; wraps rigg-core/rigg-client
├── agent/             # Agent + skill bundle generator (6 targets)
├── commands/          # Operator CLI handlers (status, query, search, get, reset, etc.)
├── onprem/            # Generate docker / systemd / k8s artefacts
├── init/              # Interactive `quelch init` wizard
├── dev/               # `quelch dev` (sim + in-memory backends + ingest + MCP, all in one process)
├── tui/               # Fleet dashboard polling quelch-meta
├── sim/, mock/        # Activity simulator + local Jira/Confluence mock servers (powers dev mode + tests)
└── ai.rs              # ailloy integration (reserved for future AI features)
```

See [docs/architecture.md](docs/architecture.md) for the canonical architecture reference.

## Key Patterns

- **Edition:** 2024, MSRV 1.95
- **Error handling:** `thiserror` for typed errors per module, `anyhow` at CLI boundary
- **Async:** `tokio` with full features
- **HTTP:** `reqwest` with `rustls-tls-native-roots`
- **CLI:** `clap` with derive macros
- **TUI:** `ratatui` + `crossterm`
- **Logging:** `tracing` + `tracing-subscriber`
- **Config:** YAML via `serde_yaml`, env var substitution via `shellexpand`

## Code Style

- `cargo clippy` must pass with no warnings (`-D warnings`)
- `cargo fmt` enforced
- No `.unwrap()` in library code — proper error propagation
- All public types and functions documented with `///` doc comments
- Keep files focused and under ~500 lines

## Test Your Work Against the User's Actual Requirements

**Before claiming any task is done, you must personally verify it satisfies the user's stated requirements — not just that `cargo test` passes.** Tests prove the code compiles and internal invariants hold. They do NOT prove the user's experience works.

- If the user said "the TUI should look like X": **run the binary, capture its output as an artifact (e.g. `quelch sim --snapshot-to FILE`), and read the artifact yourself**. Confirm every element the user asked for is actually there. If you can't launch an interactive terminal, use a headless renderer. If you can't verify something at all, say so explicitly rather than assuming.
- If the user said "the log output should be useful": **run the binary, capture stdout/stderr, read it line by line**. For each log line ask: who reads this? what do they learn? are they helped? If the answer is "nobody" or "nothing useful", remove the line or demote its level.
- If the user said "Ctrl-C should be responsive": **press Ctrl-C yourself and measure**. Don't claim it's fixed because "the channel is now wired".
- Unit + integration tests are necessary but not sufficient. They catch what you remembered to check. The user-experience audit catches what you forgot.

When a user has to show you that your own work is broken, that is a process failure. Ship only what you have personally exercised against the original request.

### Audit every log line

For every `println!`, `info!`, `warn!`, `error!`, `debug!` you add or touch:

1. **Who is the audience?** (operator running the binary / developer debugging a specific problem / nobody)
2. **What does it teach them?** (a state transition / a failure that needs action / internal noise)
3. **Is it helpful at default verbosity, or is it only helpful when they asked for more detail?** If the latter, it belongs at `debug!` / `trace!`, not `info!` / `warn!`.
4. **Does it fire once per run, once per cycle, or once per item?** Per-item logs at `info!` are almost always wrong.

If you can't answer #1 and #2 with a concrete human-useful sentence, the line should not exist.

## Releasing

Use the `/release <major|minor|patch>` skill. This bumps the version, updates the changelog, commits, tags, and pushes. The GitHub Actions release workflow then:

1. Runs full CI
2. Builds binaries for Linux, macOS (x86_64 + ARM), Windows
3. Creates a GitHub Release with all binaries
4. Updates the Homebrew tap (`mklab-se/homebrew-tap`)
5. Publishes to crates.io

**Required GitHub Secrets:**
- `CARGO_REGISTRY_TOKEN` — crates.io API token (in `crates-io` environment)
- `HOMEBREW_TAP_TOKEN` — GitHub PAT with repo scope for `mklab-se/homebrew-tap`
