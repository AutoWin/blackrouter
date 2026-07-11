# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates

RUN cargo build --release -p blackrouter-bin

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/blackrouter /usr/local/bin/blackrouter

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
