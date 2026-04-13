# CI/CD Pipeline & Release Skill Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Set up the Rust project scaffold, CI/CD pipelines, and release skill so `quelch` can be built, tested, and released using the same MKLab workflow as mdeck and rigg.

**Architecture:** Single-crate workspace mirroring mdeck/rigg patterns. CI runs on push/PR (check, test, clippy, fmt). Release triggers on `v*` tags: CI → cross-compile 4 targets → GitHub Release → Homebrew tap → crates.io. A `/release` Claude Code skill automates version bumping, changelog, and tagging.

**Tech Stack:** Rust 1.94+ (edition 2024), GitHub Actions, `clap` for CLI, `softprops/action-gh-release` for releases, `mklab-se/homebrew-tap` for Homebrew distribution.

---

### Task 1: Initialize Rust Workspace

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/quelch/Cargo.toml` (crate manifest)
- Create: `crates/quelch/src/main.rs` (entry point)
- Create: `.gitignore`

- [ ] **Step 1: Create the workspace Cargo.toml**

```toml
[workspace]
resolver = "2"
members = [
    "crates/quelch",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
authors = ["Kristofer Liljeblad <kristofer@mklab.se>"]
license = "MIT"
repository = "https://github.com/mklab-se/quelch"
rust-version = "1.94"

[workspace.dependencies]
# CLI
clap = { version = "4.5", features = ["derive", "wrap_help"] }

# Serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
serde_yaml = "0.9"

# Async runtime
tokio = { version = "1", features = ["full"] }

# HTTP client
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls-native-roots"] }

# Error handling
anyhow = "1.0"
thiserror = "2.0"

# Time
chrono = { version = "0.4", features = ["serde"] }

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# TUI
ratatui = "0.29"
crossterm = "0.28"

# Env var expansion
shellexpand = "3"

# Directories
dirs = "6.0"
```

- [ ] **Step 2: Create the crate directory and Cargo.toml**

Run: `mkdir -p crates/quelch/src`

Create `crates/quelch/Cargo.toml`:

```toml
[package]
name = "quelch"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true
rust-version.workspace = true
description = "Ingest data from Jira, Confluence, and more directly into Azure AI Search"
readme = "../../README.md"
keywords = ["azure", "search", "jira", "confluence", "ingest"]
categories = ["command-line-utilities"]

[[bin]]
name = "quelch"
path = "src/main.rs"

[dependencies]
clap.workspace = true
anyhow.workspace = true

[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/v{ version }/quelch-v{ version }-{ target }.{ archive-format }"
bin-dir = "quelch{ binary-ext }"
pkg-fmt = "tgz"

[package.metadata.binstall.overrides.x86_64-pc-windows-msvc]
pkg-fmt = "zip"
```

- [ ] **Step 3: Create the minimal main.rs with clap**

Create `crates/quelch/src/main.rs`:

```rust
use clap::Parser;

#[derive(Parser)]
#[command(name = "quelch", version, about = "Ingest data directly into Azure AI Search")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Run a one-shot sync of all configured sources
    Sync,
    /// Run continuous sync (polls at configured interval)
    Watch,
    /// Show sync status for all sources
    Status,
    /// Reset sync state (force full re-sync on next run)
    Reset,
    /// Validate config file without running
    Validate,
    /// Generate a starter quelch.yaml config
    Init,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Sync => println!("sync not yet implemented"),
        Commands::Watch => println!("watch not yet implemented"),
        Commands::Status => println!("status not yet implemented"),
        Commands::Reset => println!("reset not yet implemented"),
        Commands::Validate => println!("validate not yet implemented"),
        Commands::Init => println!("init not yet implemented"),
    }

    Ok(())
}
```

- [ ] **Step 4: Create .gitignore**

Create `.gitignore`:

```
/target
.DS_Store
```

- [ ] **Step 5: Verify it builds and runs**

Run: `cargo build --workspace`
Expected: Compiles successfully.

Run: `cargo run -p quelch -- --version`
Expected: Prints `quelch 0.1.0`

Run: `cargo run -p quelch -- --help`
Expected: Prints help with all subcommands listed.

- [ ] **Step 6: Run quality checks**

Run: `cargo fmt --all -- --check`
Expected: No formatting issues.

Run: `cargo clippy --workspace -- -D warnings`
Expected: No warnings.

Run: `cargo test --workspace`
Expected: 0 tests, all pass (no test failures).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/ .gitignore
git commit -m "Initialize Rust workspace with minimal clap CLI"
```

---

### Task 2: CI Workflow

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Create the CI workflow**

Run: `mkdir -p .github/workflows`

Create `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  check:
    name: Check
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo check --workspace

  test:
    name: Test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace

  clippy:
    name: Clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: stable
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --workspace -- -D warnings

  format:
    name: Format
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: stable
          components: rustfmt
      - run: cargo fmt --all -- --check
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "Add CI workflow (check, test, clippy, fmt)"
```

---

### Task 3: Release Workflow

**Files:**
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: Create the release workflow**

Create `.github/workflows/release.yml`:

```yaml
name: Release

on:
  push:
    tags:
      - "v*"

env:
  CARGO_TERM_COLOR: always

permissions:
  contents: write

jobs:
  # Run the full CI suite before building release artifacts
  ci:
    name: CI
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all -- --check
      - run: cargo clippy --workspace -- -D warnings
      - run: cargo test --workspace

  # Build release binaries for each platform
  build:
    name: Build ${{ matrix.target }}
    needs: ci
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            os: ubuntu-latest
            archive: tar.gz
          - target: x86_64-apple-darwin
            os: macos-latest
            archive: tar.gz
          - target: aarch64-apple-darwin
            os: macos-latest
            archive: tar.gz
          - target: x86_64-pc-windows-msvc
            os: windows-latest
            archive: zip
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - uses: Swatinem/rust-cache@v2
        with:
          key: ${{ matrix.target }}

      - name: Build release binary
        shell: bash
        run: cargo build --release --target "$TARGET"
        env:
          TARGET: ${{ matrix.target }}

      - name: Package (unix)
        if: matrix.archive == 'tar.gz'
        shell: bash
        run: |
          cd "target/${TARGET}/release"
          tar czf "../../../quelch-${RELEASE_TAG}-${TARGET}.tar.gz" quelch
          cd ../../..
        env:
          TARGET: ${{ matrix.target }}
          RELEASE_TAG: ${{ github.ref_name }}

      - name: Package (windows)
        if: matrix.archive == 'zip'
        shell: pwsh
        run: |
          Compress-Archive -Path "target/$env:TARGET/release/quelch.exe" -DestinationPath "quelch-$env:RELEASE_TAG-$env:TARGET.zip"
        env:
          TARGET: ${{ matrix.target }}
          RELEASE_TAG: ${{ github.ref_name }}

      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: quelch-${{ matrix.target }}
          path: quelch-${{ github.ref_name }}-${{ matrix.target }}.*

  # Create GitHub Release with all binaries
  github-release:
    name: GitHub Release
    needs: build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Download all artifacts
        uses: actions/download-artifact@v4
        with:
          path: artifacts
          merge-multiple: true

      - name: Create release
        uses: softprops/action-gh-release@v2
        with:
          generate_release_notes: true
          files: artifacts/*

  # Update Homebrew tap with new version and SHA256 hashes
  homebrew:
    name: Update Homebrew Tap
    needs: github-release
    runs-on: ubuntu-latest
    steps:
      - name: Update formula
        env:
          GH_TOKEN: ${{ secrets.HOMEBREW_TAP_TOKEN }}
        run: |
          set -e
          VERSION="${GITHUB_REF_NAME#v}"
          TAG="${GITHUB_REF_NAME}"
          BASE_URL="https://github.com/mklab-se/quelch/releases/download/${TAG}"

          # Download release artifacts and compute SHA256
          curl -sL "${BASE_URL}/quelch-${TAG}-aarch64-apple-darwin.tar.gz" -o aarch64-darwin.tar.gz
          curl -sL "${BASE_URL}/quelch-${TAG}-x86_64-apple-darwin.tar.gz" -o x86_64-darwin.tar.gz
          curl -sL "${BASE_URL}/quelch-${TAG}-x86_64-unknown-linux-gnu.tar.gz" -o x86_64-linux.tar.gz

          SHA_AARCH64=$(sha256sum aarch64-darwin.tar.gz | cut -d' ' -f1)
          SHA_X86_64=$(sha256sum x86_64-darwin.tar.gz | cut -d' ' -f1)
          SHA_LINUX=$(sha256sum x86_64-linux.tar.gz | cut -d' ' -f1)

          # Generate formula
          cat > formula.rb <<RUBY
          class Quelch < Formula
            desc "Ingest data from Jira, Confluence, and more directly into Azure AI Search"
            homepage "https://github.com/mklab-se/quelch"
            version "${VERSION}"
            license "MIT"

            on_macos do
              if Hardware::CPU.arm?
                url "https://github.com/mklab-se/quelch/releases/download/v#{version}/quelch-v#{version}-aarch64-apple-darwin.tar.gz"
                sha256 "${SHA_AARCH64}"
              else
                url "https://github.com/mklab-se/quelch/releases/download/v#{version}/quelch-v#{version}-x86_64-apple-darwin.tar.gz"
                sha256 "${SHA_X86_64}"
              end
            end

            on_linux do
              url "https://github.com/mklab-se/quelch/releases/download/v#{version}/quelch-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
              sha256 "${SHA_LINUX}"
            end

            def install
              bin.install "quelch"
            end

            test do
              assert_match version.to_s, shell_output("#{bin}/quelch --version")
            end
          end
          RUBY

          # Push to homebrew-tap repo via GitHub API
          CONTENT=$(base64 -w 0 < formula.rb)
          FILE_SHA=$(gh api repos/mklab-se/homebrew-tap/contents/Formula/quelch.rb --jq '.sha' 2>/dev/null || echo "")

          if [ -n "$FILE_SHA" ]; then
            gh api repos/mklab-se/homebrew-tap/contents/Formula/quelch.rb \
              -X PUT \
              -f message="Update quelch to ${VERSION}" \
              -f content="$CONTENT" \
              -f sha="$FILE_SHA" \
              && echo "Homebrew formula updated to ${VERSION}" \
              || echo "::warning::Failed to update Homebrew tap. Add HOMEBREW_TAP_TOKEN secret with repo scope for mklab-se/homebrew-tap."
          else
            # Create file for the first time
            gh api repos/mklab-se/homebrew-tap/contents/Formula/quelch.rb \
              -X PUT \
              -f message="Add quelch ${VERSION}" \
              -f content="$CONTENT" \
              && echo "Homebrew formula created for ${VERSION}" \
              || echo "::warning::Failed to create Homebrew formula. Add HOMEBREW_TAP_TOKEN secret with repo scope for mklab-se/homebrew-tap."
          fi

  # Publish to crates.io
  crates-io:
    name: Publish to crates.io
    needs: ci
    runs-on: ubuntu-latest
    environment: crates-io
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2

      - name: Publish quelch
        run: cargo publish -p quelch
        env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "Add release workflow (build, GitHub Release, Homebrew, crates.io)"
```

---

### Task 4: CHANGELOG.md

**Files:**
- Create: `CHANGELOG.md`

- [ ] **Step 1: Create the changelog**

Create `CHANGELOG.md`:

```markdown
# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added
- Initial project scaffold with CLI subcommands (sync, watch, status, reset, validate, init)
- CI workflow (check, test, clippy, fmt)
- Release workflow (cross-platform binaries, GitHub Release, Homebrew tap, crates.io)
```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "Add CHANGELOG.md"
```

---

### Task 5: Release Skill

**Files:**
- Create: `.claude/skills/release/SKILL.md`

- [ ] **Step 1: Create the skills directory**

Run: `mkdir -p .claude/skills/release`

- [ ] **Step 2: Create the release skill**

Create `.claude/skills/release/SKILL.md`:

```markdown
---
name: release
description: "Release a new version: bump version, update docs, commit, push, and tag"
argument-hint: "<major|minor|patch>"
---

Release a new version of quelch.

## Input

$ARGUMENTS must be one of: `major`, `minor`, `patch`. If empty or invalid, stop and ask.

## Steps

### 1. Determine the new version

- Read the current version from the `version` field in the workspace `Cargo.toml`
- Apply the semver bump based on $ARGUMENTS:
  - `patch`: 0.1.0 -> 0.1.1
  - `minor`: 0.1.0 -> 0.2.0
  - `major`: 0.1.0 -> 1.0.0
- Show the user: "Releasing quelch v{OLD} -> v{NEW}"

### 2. Update dependencies

- Run `cargo update` to update all dependencies to the latest compatible versions
- This ensures the release ships with up-to-date dependencies

### 3. Pre-flight checks

- Run `cargo fmt --all -- --check` — abort if formatting issues
- Run `cargo clippy --workspace -- -D warnings` — abort if warnings
- Run `cargo test --workspace` — abort if any test fails
- Run `git status` — abort if there are uncommitted changes that are NOT documentation, version, or dependency files

### 4. Bump version numbers

- Update `version` in the root `Cargo.toml` `[workspace.package]` section

### 5. Update CHANGELOG

- **CHANGELOG.md**: Rename the `[Unreleased]` section to `[{NEW_VERSION}] - {TODAY}` (YYYY-MM-DD format). If there is no `[Unreleased]` section, create a new dated entry summarizing changes since the last release

### 6. Verify the build

- Run `cargo build --workspace` to ensure everything compiles with the new version
- Run `cargo test --workspace` once more after version bump

### 7. Commit, push, and tag

- Stage all changed files: `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, and any updated docs
- Commit with message: `Release v{NEW_VERSION}`
- Push to main: `git push`
- Create and push tag: `git tag v{NEW_VERSION} && git push origin v{NEW_VERSION}`

### 8. Confirm

- Tell the user the release is tagged and pushed
- Remind them that the GitHub Actions release workflow will now build binaries, publish to crates.io, and update the Homebrew tap
```

- [ ] **Step 3: Commit**

```bash
git add .claude/skills/release/SKILL.md
git commit -m "Add /release skill for version bumping and tagging"
```

---

### Task 6: Claude Code Settings

**Files:**
- Create: `.claude/settings.local.json`

- [ ] **Step 1: Create the settings file**

Create `.claude/settings.local.json`:

```json
{
  "permissions": {
    "allow": [
      "Bash(cargo fmt --all -- --check)",
      "Bash(cargo clippy --workspace -- -D warnings)",
      "Bash(cargo fmt --all)",
      "Bash(cargo test:*)"
    ]
  }
}
```

- [ ] **Step 2: Commit**

```bash
git add .claude/settings.local.json
git commit -m "Add Claude Code permission settings"
```

---

### Task 7: CLAUDE.md

**Files:**
- Create: `CLAUDE.md`

- [ ] **Step 1: Create CLAUDE.md**

Create `CLAUDE.md`:

```markdown
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
```

- [ ] **Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "Add CLAUDE.md with project overview and conventions"
```

---

### Task 8: Push and Verify CI

- [ ] **Step 1: Run all quality checks locally**

Run: `cargo fmt --all -- --check`
Expected: No issues.

Run: `cargo clippy --workspace -- -D warnings`
Expected: No warnings.

Run: `cargo test --workspace`
Expected: All pass.

- [ ] **Step 2: Push to main**

Run: `git push`

- [ ] **Step 3: Verify CI passes on GitHub**

Check: Open `https://github.com/mklab-se/quelch/actions` and confirm all 4 CI jobs (Check, Test, Clippy, Format) pass green.

---

### Task 9: Test Release Flow (v0.1.0)

- [ ] **Step 1: Ensure GitHub secrets are configured**

The following secrets must be set on the `mklab-se/quelch` repository:
- `CARGO_REGISTRY_TOKEN` — in a `crates-io` environment
- `HOMEBREW_TAP_TOKEN` — repository secret

If not set, the release will partially succeed (binaries + GitHub Release will work, but Homebrew and crates.io steps will fail with warnings).

- [ ] **Step 2: Use the release skill**

Run: `/release patch` (this will tag v0.1.0 → v0.1.1, or if you want the first release to be v0.1.0, manually tag it)

Alternatively, for the initial release, manually tag and push:

```bash
git tag v0.1.0
git push origin v0.1.0
```

- [ ] **Step 3: Verify the release workflow**

Check: Open `https://github.com/mklab-se/quelch/actions` and confirm:
1. CI job passes
2. Build jobs complete for all 4 targets
3. GitHub Release is created with 4 binary artifacts
4. Homebrew tap is updated (if secret is configured)
5. crates.io publish succeeds (if secret is configured)

- [ ] **Step 4: Verify the GitHub Release page**

Check: `https://github.com/mklab-se/quelch/releases/tag/v0.1.0` shows:
- Auto-generated release notes
- `quelch-v0.1.0-aarch64-apple-darwin.tar.gz`
- `quelch-v0.1.0-x86_64-apple-darwin.tar.gz`
- `quelch-v0.1.0-x86_64-unknown-linux-gnu.tar.gz`
- `quelch-v0.1.0-x86_64-pc-windows-msvc.zip`
