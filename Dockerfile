# ---------- Planner: resolve dependencies with cargo-chef ----------
# Uses rust:latest so the toolchain stays current. The resulting recipe pins
# dependency resolution so the planner + builder layers below are cached until
# Cargo.toml/Cargo.lock change.
FROM rust:latest AS chef
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
RUN cargo install cargo-chef --locked

# ---------- Planner: extract dependency graph ----------
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---------- Builder: compile with cached dependencies ----------
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build all workspace dependencies (cached unless the manifest changes).
RUN cargo chef cook --release --recipe-path recipe.json
# Copy the source and build the actual binary.
COPY . .
RUN cargo build --release --bin financial-management

# ---------- Builder: compile the migrations workspace binary ----------
FROM rust:latest AS migrations-builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY migrations/Cargo.toml migrations/Cargo.toml
RUN mkdir -p src && echo "fn main() {}" > src/main.rs
COPY migrations migrations
# Pre-build dependencies only (cached layer).
RUN cargo build --release -p migrations || true
COPY migrations migrations
RUN touch migrations/src/main.rs && cargo build --release -p migrations

# ---------- Runtime: minimal image ----------
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /usr/sbin/nologin appuser

WORKDIR /app
COPY --from=builder /app/target/release/financial-management /usr/local/bin/financial-management
# Migrations run before the server starts (see entrypoint.sh).
COPY migrations/migrations /app/migrations
COPY --from=migrations-builder /app/target/release/migrations /usr/local/bin/migrations
COPY scripts/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

USER appuser
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
