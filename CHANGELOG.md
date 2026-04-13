# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

## [0.2.0] - 2026-04-14

### Added
- **Config module** — YAML config loading with environment variable substitution (`${VAR}`)
- **Jira connector** — Supports both Cloud (v3 API with ADF) and Data Center (v2 API, versions 9.12-10.x)
- **Confluence connector** — Cloud and Data Center support with heading-based page chunking
- **Azure AI Search client** — Index creation, document push with mergeOrUpload, retry with exponential backoff
- **Vector search** — Embeddings via ailloy (Azure OpenAI), HNSW index with scalar quantization, semantic reranking
- **Sync engine** — Incremental sync with cursor-based high-water marks, crash-safe state persistence
- **Labeled content** — Structured content field with field labels (Assignee:, Reporter:, Status:, etc.) for better semantic search
- **CLI commands** — sync, watch, setup, status, reset, reset-indexes, validate, init, search, mock, ai
- **Search command** — Semantic search from the terminal with colored output, relevance bars, and clickable URLs
- **Mock server** — Built-in Jira DC and Confluence DC mock server with 17 issues and 8 pages about quelch
- **AI integration** — `quelch ai` command for embedding model configuration via ailloy
- **Dual auth** — Cloud (email + API token, Basic Auth) and Data Center (PAT, Bearer) authentication
- **Index management** — `setup` creates indexes with vector search config, `reset-indexes` deletes and clears state
- CI workflow (check, test, clippy, fmt)
- Release workflow (cross-platform binaries, GitHub Release, Homebrew tap, crates.io)
- 84 tests (77 unit + 7 integration with wiremock)

## [0.1.0] - 2026-04-13

### Added
- Initial project scaffold with CLI subcommands
- CI and release workflows
