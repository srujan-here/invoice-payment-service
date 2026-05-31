# Multi-stage build producing both workspace binaries in one slim image.
FROM rust:1-slim-bookworm AS builder
WORKDIR /app

# Cache dependencies: copy manifests first, then sources.
COPY Cargo.toml Cargo.lock ./
COPY crates/common/Cargo.toml crates/common/Cargo.toml
COPY crates/invoice-service/Cargo.toml crates/invoice-service/Cargo.toml
COPY crates/mock-psp/Cargo.toml crates/mock-psp/Cargo.toml
# Stub sources so the dependency layer compiles and caches.
RUN mkdir -p crates/common/src crates/invoice-service/src crates/mock-psp/src \
    && echo "" > crates/common/src/lib.rs \
    && echo "fn main(){}" > crates/invoice-service/src/main.rs \
    && echo "pub fn _x(){}" > crates/invoice-service/src/lib.rs \
    && echo "fn main(){}" > crates/mock-psp/src/main.rs \
    && cargo build --release 2>/dev/null || true

# Real sources + migrations (migrations are embedded at compile time).
COPY crates ./crates
COPY migrations ./migrations
RUN touch crates/*/src/*.rs && cargo build --release

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/invoice-service /usr/local/bin/invoice-service
COPY --from=builder /app/target/release/mock-psp /usr/local/bin/mock-psp

# Default command; overridden per service in docker-compose.
CMD ["invoice-service"]
