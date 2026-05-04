# Multi-stage build for quelch.
#
# Stage 1: Build the release binary.
# Stage 2: Minimal runtime image via distroless (glibc + libstdc++, no shell).

FROM rust:slim-bookworm AS builder
WORKDIR /app

# Install build dependencies (needed for linking).
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests and lock file first to leverage layer caching.
COPY Cargo.toml Cargo.lock ./
COPY crates/quelch/Cargo.toml ./crates/quelch/

# Create a stub library so Cargo can resolve the workspace without the full source.
RUN mkdir -p crates/quelch/src && echo "fn main() {}" > crates/quelch/src/main.rs && touch crates/quelch/src/lib.rs

# Fetch and compile dependencies (cached unless Cargo.toml/Cargo.lock change).
# This stub-compile is expected to fail at link time (no real source yet),
# so `|| true` is structurally necessary. We keep stderr visible so genuine
# errors (registry unreachable, missing system libs) still surface in logs.
RUN cargo build --release -p quelch || true

# Now copy the real source and build.
COPY crates/quelch/src ./crates/quelch/src

# Touch main.rs so Cargo knows it changed.
RUN touch crates/quelch/src/main.rs crates/quelch/src/lib.rs

RUN cargo build --release -p quelch

# ─────────────────────────────────────────────────────────────────────────────
# Runtime image — distroless/cc provides glibc + libstdc++, no shell.
# ─────────────────────────────────────────────────────────────────────────────
FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /app/target/release/quelch /usr/local/bin/quelch

ENTRYPOINT ["/usr/local/bin/quelch"]
CMD ["--help"]
