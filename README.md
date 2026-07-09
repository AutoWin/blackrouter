# BlackRouter

Rust runtime scaffold for the 9Router-to-BlackRouter migration.

Current implementation status:

- Rust workspace with separate crates for API, binary, config, common utilities, core routing, provider boundaries, RTK, translators, SQLite storage, and Telegram command parsing.
- `blackrouter` binary loads env config, initializes a 9Router-compatible SQLite schema, and starts an Axum HTTP server.
- Minimal setup UI is available at `/setup`.
- Setup UI can write saved config, provider connections, and API keys to the BlackRouter SQLite DB.
- Provider setup can create, edit, enable/disable, delete, run a basic connection check, and fetch supported model IDs into provider `data.models`.
- Cline Router and Command Code use built-in model catalogs when their live `/models` endpoint is unavailable.
- Provider setup includes presets derived from `9router-custom`, including required `commandcode` and `cline` router entries.
- Combo setup can create, edit, delete, list, and resolve fallback model combos stored in the BlackRouter SQLite DB.
- Implemented routes:
  - `GET /` redirecting to `/setup`
  - `GET /setup`
  - `GET /setup.css`
  - `GET /setup.js`
  - `GET /health`
  - `GET /version`
  - `GET /api/runtime/status`
  - `GET|PUT /api/setup/config`
  - `GET|POST /api/setup/api-keys`
  - `GET|POST /api/setup/providers`
  - `GET|PUT|DELETE /api/setup/providers/{id}`
  - `POST /api/setup/providers/{id}/toggle`
  - `POST /api/setup/providers/{id}/test`
  - `POST /api/setup/providers/{id}/models`
  - `GET /api/setup/provider-catalog`
  - `GET|POST /api/setup/combos`
  - `GET|PUT|DELETE /api/setup/combos/{id}`
  - `GET /v1/models`
  - `GET /v1beta/models`
  - `POST /v1/chat/completions` with minimal OpenAI-compatible provider proxy and combo fallback
  - `POST /v1/responses` as a compatibility shell
  - `POST /v1/messages` as a compatibility shell
- Telegram settings are represented in config; bot runtime is intentionally left for the later Telegram phase.
- Telegram command parser supports the read-only/control command vocabulary from the migration plan; network bot runtime is not wired yet.

## Run

Install Rust, then:

```bash
cargo run -p blackrouter-bin
```

The local `.env` currently uses port `20129` and stores data under `/Users/ccm/Documents/blackrouter/data`.

To point at an existing 9Router data directory:

```bash
BLACKROUTER_DATA_DIR=/Users/ccm/.9router cargo run -p blackrouter-bin
```

## Docker

Build and run with Docker Compose:

```bash
docker compose up --build
```

The compose setup reads `.env`, binds `BLACKROUTER_PORT`, and mounts `./data` into the container as `/data`. Inside Docker, these values are forced to container-safe paths:

```env
BLACKROUTER_DATA_DIR=/data
BLACKROUTER_DATABASE_URL=sqlite:///data/blackrouter.db
```

With the current `.env`, open:

```bash
curl http://localhost:20129/health
```

For Zed/OpenAI-compatible clients, use:

```json
"api_url": "http://localhost:20129/v1"
```

## Verify

```bash
curl http://localhost:20129/health
curl http://localhost:20129/setup
curl http://localhost:20129/version
curl http://localhost:20129/api/runtime/status
curl http://localhost:20129/v1/models
```

## Notes

The setup database is independent from `9router-custom` by default. The chat completions route resolves direct `provider/model` IDs and configured combo names, then proxies OpenAI-compatible providers with combo fallback. Provider-specific translators, RTK, and the responses/messages routes are still pending.
