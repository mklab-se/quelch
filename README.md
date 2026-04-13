<p align="center">
  <img src="https://raw.githubusercontent.com/mklab-se/quelch/main/media/quelch.png" alt="quelch" width="600">
</p>

<h1 align="center">Quelch</h1>

<p align="center">
  Ingest data from Jira, Confluence, and more directly into Azure AI Search.<br>
  No intermediate storage. Just source-to-index sync.
</p>

<p align="center">
  <a href="https://github.com/mklab-se/quelch/actions/workflows/ci.yml"><img src="https://github.com/mklab-se/quelch/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://crates.io/crates/quelch"><img src="https://img.shields.io/crates/v/quelch.svg" alt="crates.io"></a>
  <a href="https://github.com/mklab-se/quelch/releases/latest"><img src="https://img.shields.io/github/v/release/mklab-se/quelch" alt="GitHub Release"></a>
  <a href="https://github.com/mklab-se/homebrew-tap/blob/main/Formula/quelch.rb"><img src="https://img.shields.io/badge/dynamic/regex?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmklab-se%2Fhomebrew-tap%2Fmain%2FFormula%2Fquelch.rb&search=%5Cd%2B%5C.%5Cd%2B%5C.%5Cd%2B&label=homebrew&prefix=v&color=orange" alt="Homebrew"></a>
  <a href="https://github.com/mklab-se/quelch/blob/main/LICENSE"><img src="https://img.shields.io/crates/l/quelch.svg" alt="License"></a>
</p>

<p align="center">
  <a href="CHANGELOG.md"><strong>Changelog</strong></a>
</p>

---

## What is Quelch?

Quelch is a Rust CLI tool that ingests data from external sources (Jira, Confluence) directly into Azure AI Search indexes — no intermediate storage like Blob Storage or Cosmos DB. It runs as a one-shot sync or a continuous background process, with incremental sync, smart concurrency, and a rich terminal UI.

The name "Quelch" evokes quenching a search index's thirst for data.

## Features

- **Direct ingest** — Source data goes straight into Azure AI Search indexes
- **Incremental sync** — Only fetches changes since last sync using high-water marks
- **Crash-safe** — State persisted after every batch; restart and pick up where you left off
- **Smart concurrency** — Parallel sync across servers, throttled per credential
- **Rich TUI** — Live dashboard showing sync status, rates, and logs (via Ratatui)
- **Multiple sources** — Jira and Confluence connectors, with a clean trait for adding more
- **Minimal config** — Get started with a 10-line YAML file; smart defaults everywhere

## Installation

### Homebrew (macOS/Linux)

```bash
brew install mklab-se/tap/quelch
```

### Cargo

```bash
cargo install quelch
```

### Binary download

Download pre-built binaries from the [latest release](https://github.com/mklab-se/quelch/releases/latest).

## Quick Start

```bash
# Generate a starter config
quelch init

# Edit quelch.yaml with your sources and Azure endpoint
# Then run a one-shot sync
quelch sync

# Or run continuous sync
quelch watch
```

## Usage

```
quelch — Ingest data directly into Azure AI Search

COMMANDS:
    sync        Run a one-shot sync of all configured sources
    watch       Run continuous sync (polls at configured interval)
    status      Show sync status for all sources
    reset       Reset sync state (force full re-sync on next run)
    validate    Validate config file without running
    init        Generate a starter quelch.yaml config

OPTIONS:
    -c, --config <PATH>    Config file path (default: quelch.yaml)
    -v, --verbose          Increase verbosity
    -q, --quiet            Suppress TUI, only log errors
    --json                 Output logs as JSON
    --version              Print version
    --help                 Print help
```

## License

MIT
