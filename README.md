# BlackRouter

[![CI](https://github.com/AutoWin/blackrouter/actions/workflows/ci.yml/badge.svg)](https://github.com/AutoWin/blackrouter/actions)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Docker](https://img.shields.io/badge/docker-available-0db7ed.svg)](docker-compose.yml)

**BlackRouter** is a self-hosted, OpenAI-compatible **AI gateway / LLM router**
written in Rust. One endpoint and one API key to access many AI providers, with
smart routing, failover, cost control, rate limiting, and full observability.

> Bring your own provider keys. BlackRouter never stores provider credentials
> anywhere except your own database. You stay in control of your data and spend.

## Features

- **Unified OpenAI-compatible API** — point any OpenAI-compatible client
  (Zed, Continue, OpenWebUI, LiteLLM, the OpenAI SDK, …) at `/v1` and route to
  any backend.
- **Many providers** — OpenAI / Codex, Google Gemini, Anthropic Claude,
  GitHub Copilot, Antigravity, and more, configured through the setup UI.
- **OAuth login** — connect provider accounts (Google, Antigravity, OpenAI/Codex,
  GitHub) directly from the browser without copying tokens manually. Callbacks
  work in any deployment (local, LAN, Docker, or behind a reverse proxy) via a
  same-origin `/oauth/callback` relay.
- **Combos & fallback** — compose fallback model chains (e.g. `gpt-4o` →
  `claude-*` → `gemini-*`) with automatic failover.
- **Aliases** — friendly model aliases resolved at request time.
- **API keys & quotas** — issue gateway API keys with per-key tenant IDs, daily
  request/token quotas, monthly cost quotas, and provider/model allowlists.
- **Cost guard & rate limiting** — cap spend and requests; optional shared
  limiting across replicas via Redis.
- **Observability** — Prometheus metrics + Grafana dashboards
  (see `deploy/observability`).
- **Multi-replica** — Redis-backed shared state/cache for horizontally scaled
  deployments.
- **Telegram bot** — link accounts and receive notifications (optional).
- **9Router-compatible** — reuses the 9Router SQLite schema and provider presets
  for an easy migration path.

## Quick start

### Docker (recommended)

```bash
cp .env.example .env        # then edit at least BLACKROUTER_BASE_URL
docker compose up --build
```

Open <http://localhost:20129/setup> and add your provider connections.

### From source

```bash
cargo run -p blackrouter-bin
# then open http://localhost:20129/setup
```

To point at an existing 9Router data directory:

```bash
BLACKROUTER_DATA_DIR=~/.9router cargo run -p blackrouter-bin
```

## Configuration

All configuration is via environment variables (see `.env.example`):

| Variable | Purpose |
|---|---|
| `BLACKROUTER_HOST` / `BLACKROUTER_PORT` | Listen address |
| `BLACKROUTER_BASE_URL` | Public base URL (used for OAuth callbacks if not auto-detected) |
| `BLACKROUTER_DATA_DIR` / `BLACKROUTER_DATABASE_URL` | Storage |
| `BLACKROUTER_REDIS_URL` | Shared state for multi-replica (optional) |
| `BLACKROUTER_REQUIRE_API_KEY` | Require gateway API keys |
| `BLACKROUTER_CONTROL_API_ENABLED` / `BLACKROUTER_CONTROL_TOKEN` | Protect the control plane |
| `OAUTH_*_CLIENT_ID` / `OAUTH_*_CLIENT_SECRET` | Provider OAuth apps |

### OAuth redirect URIs

Register these with your provider's OAuth console:

- `https://<your-host>/oauth/callback` — Google, Antigravity (recommended, single URI)
- `https://<your-host>/api/oauth/{provider}/callback` — legacy per-provider URIs still work
- `http://localhost:1455/auth/callback` — OpenAI/Codex loopback (or use the manual "paste code" fallback when self-hosting remotely)

## Architecture

Rust workspace (axum HTTP server):

- `blackrouter-api` — HTTP API, routing, OAuth, providers, RTK, translators
- `blackrouter-bin` — binary entrypoint
- `blackrouter-config` / `blackrouter-common` / `blackrouter-core` — support crates
- `blackrouter-cli` — CLI helpers

Storage is SQLite (9Router-compatible schema). Redis is optional.

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build --workspace --release
```

CI runs formatting, clippy, tests, and a release build on every push/PR.

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) and our
[Code of Conduct](CODE_OF_CONDUCT.md).

## Security

Found a vulnerability? Please follow our [security policy](SECURITY.md) and
report it privately rather than opening a public issue.

## License

[MIT](LICENSE) © The BlackRouter Authors.
