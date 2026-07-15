# syntax=docker/dockerfile:1.7

FROM rust:1.97-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN --mount=type=cache,id=blackrouter-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=blackrouter-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=blackrouter-target,target=/app/target \
    cargo build --release --locked -j 6 -p blackrouter-bin \
    && cp /app/target/release/blackrouter /tmp/blackrouter

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /tmp/blackrouter /usr/local/bin/blackrouter

ENV BLACKROUTER_HOST=0.0.0.0 \
    BLACKROUTER_PORT=20129 \
    BLACKROUTER_DATA_DIR=/data \
    BLACKROUTER_DATABASE_URL=sqlite:///data/blackrouter.db \
    BLACKROUTER_COMPAT_9ROUTER_DB=false \
    BLACKROUTER_REQUIRE_API_KEY=false \
    BLACKROUTER_LOG_LEVEL=info

VOLUME ["/data"]
EXPOSE 20129

CMD ["blackrouter"]
