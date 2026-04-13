# Quelch

Quelch ingests data from external sources (Jira, Confluence) directly into Azure AI Search indexes. No intermediate storage — direct source-to-index sync with incremental updates.

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
├── main.rs          # CLI entry point, clap setup
├── cli.rs           # CLI arg definitions (planned)
├── config/          # YAML config loading, validation, env var substitution (planned)
├── sources/         # Source connector trait + implementations (planned)
│   ├── jira.rs
│   └── confluence.rs
├── azure/           # Azure AI Search REST client (planned)
├── sync/            # Sync engine, state persistence, concurrency (planned)
├── tui/             # Ratatui dashboard (planned)
└── transform.rs     # Source doc → Azure doc mapping (planned)
```

## Key Patterns

- **Edition:** 2024, MSRV 1.94
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
