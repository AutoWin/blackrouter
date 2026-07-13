# BlackRouter Implementation Status

_Updated: 2026-07-13_

> Rewritten to reflect the current codebase. The previous version (2026-07-06)
> incorrectly listed SSE streaming, usage tracking, and cost calculation as
> "In Progress". All three are implemented and verified against the Rust source.

## ✅ Completed

### Core Infrastructure
- Rust workspace with **11 crates** (Cargo workspace members)
- SQLite storage with 9Router-compatible schema (`usageHistory`, `usageDaily`, `apiKeys`, `combos`, `providerConnections`, `kv`, plus BlackRouter tables `apiKeyQuotaCounters`, `settingsHistory`, `adminAuditLog`, `telegramLinks`, `runtimeEvents`, `rtk_state`)
- Config loader with env mapping (`BLACKROUTER_*`, legacy `DATA_DIR`, `PORT`)
- Axum HTTP server with graceful shutdown
- Dockerfile & docker-compose.yml
- Config hot-reload + versioning (`settingsHistory`, `list_settings_versions`, `restore_settings_version`)

### API Endpoints
- `GET /health`, `/healthz`, `/readyz`, `/version`, `/api/runtime/status`
- `GET|PUT /api/setup/config` (+ `GET /api/setup/config/versions`, `POST /api/setup/config/versions/{version}/restore`)
- `GET|POST /api/setup/api-keys` (tenant id, quota policy, provider/model allowlist)
- `CRUD /api/setup/providers` (+ toggle / test / models / health)
- `GET /api/setup/provider-catalog`
- `CRUD /api/setup/combos`
- `CRUD /api/setup/aliases` (Phase 4.3 model aliases)
- `GET /v1/models`, `/v1beta/models`
- `POST /v1/chat/completions` (non-streaming **and SSE streaming** proxy)
- `POST /v1/responses`, `/v1/messages` (shells)
- `GET /provider-health`, `/provider-limits`
- `GET /usage`, `/usage/daily`, `/usage/daily/{date}`, `POST /usage/aggregate`
- `GET /conversations`, `/conversations/{session_id}`
- `GET /api/rtk/metrics`, `/api/rtk/status/{provider}/{model}`
- `GET /api/doctor`

### SSE Streaming (verified)
- `translate_sse_stream` translates upstream Claude/Gemini/CommandCode SSE into OpenAI `chat.completions` SSE
- `sse_data` parses `data:` frames; `wrap_session_response` handles **both JSON and SSE** responses for `/v1/chat/completions`
- Streaming translation tests present in `blackrouter-translator` (`stream.rs`)

### Usage Tracking & Cost (verified)
- Per-request usage persisted to SQLite (`usageHistory` via `record_usage` / `record_usages_batch`)
- Per-API-key quota counters (`apiKeyQuotaCounters`) with request/token reservation
- Daily usage aggregation (`usageDaily`, `aggregate_daily_usage`, `list_daily_usage`, `get_daily_usage`)
- Cost calculation from `model_catalog` pricing (`provider_estimated_cost`, `ModelCatalogEntry.price_in_per_million` / `price_out_per_million`)
- Usage API endpoints under `/usage/*`

### Provider Translators (verified)
- OpenAI Chat ↔ Claude Messages
- OpenAI Chat ↔ Gemini
- OpenAI Chat → CommandCode, Cursor, Kiro, Antigravity, GeminiCli
- Response translation back to OpenAI format
- Request/response body transformation
- System message handling per provider

### RTK — Real-Time Kit (verified)
- Rate limiting per key (requests/min, tokens/min, concurrent)
- Request tracking (success/fail, latency, tokens)
- Metrics with percentiles (p95, p99)
- Circuit breaker pattern
- Response cache (LRU + TTL, non-streaming temperature=0)
- Durable snapshot to SQLite `rtk_state`, and Redis when `BLACKROUTER_REDIS_URL` is set
- Builder pattern for configuration; API endpoints for metrics and status

### Multi-Tenancy (verified)
- API key `tenantId` + `ApiKeyPolicy` (requests/day, tokens/day, cost/month, provider/model allowlist)
- Quota enforcement via `apiKeyQuotaCounters`
- Key rotation preserves tenant and policy

### Model Catalog & Routing (verified)
- `model_catalog` table + `ModelCatalogEntry` (context window, modalities, pricing, latency)
- Cost-aware routing (`provider_estimated_cost`) and capability routing (`filter_by_capability`)
- Combo hedge / fallback-by-error (non-streaming)

### Conversation Memory (verified, partial)
- Session-scoped memory via `x-session-id` header (kv store, `scope="conversation"`)
- Context assembly + trimming (`assemble_session_body`, `trim_context_to_fit`)
- **Currently wired only into `/v1/chat/completions`**; `/v1/messages` and `/v1/responses` are follow-ups (see Not Yet Implemented)

### Proactive Provider Health (verified)
- `spawn_provider_health_prober` continuously probes configured providers
- `/provider-health` exposes latest probe summary
- `/readyz` integrates SQLite, active providers, probe state, RTK, and Redis when configured

### Horizontal Scaling Foundation (verified)
- Optional Redis shared RTK/cache via `BLACKROUTER_REDIS_URL` (`BLACKROUTER_SHARED_STATE_PREFIX`)
- Single-node still runs entirely on SQLite + in-memory state

### Observability (verified)
- OpenTelemetry traces (`telemetry.rs`, gated by `OTEL_EXPORTER_OTLP_ENDPOINT`)
- `x-request-id` middleware
- Grafana dashboard (`deploy/observability/grafana/dashboards/blackrouter.json`)
- Prometheus scrape config

### Telegram Bot
- Full Telegram Bot API client
- Long polling runtime (webhook mode not implemented)
- Command parser (25+ commands): read-only (`/status`, `/health`, `/version`, `/providers`, `/models`, `/combos`, `/usage`, `/logs`) and control (`/enable`, `/disable`, `/test`, `/rtk`, `/reload`, `/shutdown`, `/link`)
- Admin authorization with `is_authorized_chat()`
- HTML formatted responses; help formatting

### Setup UI
- Static HTML/CSS/JS interface
- Provider, combo, API key, alias, and config management

### Other
- Built-in provider catalogs (Cline, CommandCode, etc.)
- Load balancing strategies (round-robin, weighted, least-connections, response-time)
- Request queuing (retry with backoff)
- Error handling with OpenAI-compatible format

### CI (verified)
- `.github/workflows/ci.yml` runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo build --release`

---

## 🚧 In Progress

_None of the previously listed items remain here — SSE streaming, usage tracking, and cost calculation are done. The only active follow-ups are listed under Not Yet Implemented._

---

## 📋 Not Yet Implemented

### Outstanding follow-ups (the only real gaps)
- [ ] **Wire conversation memory into `/v1/messages` and `/v1/responses`** — session context via `x-session-id` is currently only active on `/v1/chat/completions`; `/v1/messages` and `/v1/responses` are shells without memory/trimming.
- [ ] **8.3 Semantic Memory** — optional vector store / embeddings-based recall. Explicitly not implemented yet.

### Not started / optional (deferred, not on the active critical path)
- [ ] Plugin system
- [ ] Telegram webhook mode (long polling is implemented)
- [ ] OAuth login tương tác chỉ có cho `github`, `codex`/`openai`, `google`/`gemini`, `antigravity`. `cursor` và `kiro` đánh dấu `auth_type: oauth` trong catalog nhưng **chưa có luồng login server-side** — cần paste token thủ công.
- [ ] Account fallback (multiple connections per provider) — health probing exists, multi-connection failover does not
- [ ] Docker image optimization (functional today, not slimmed)

---

## 🧪 Test Coverage

Counts are from `#[test]` / `#[tokio::test]` attributes across the workspace (measured 2026-07-13). The previous "30" total was stale.

| Crate | Tests | Status |
|-------|-------|--------|
| blackrouter-api | 33 | ✅ Pass |
| blackrouter-common | 2 | ✅ Pass |
| blackrouter-config | 1 | ✅ Pass |
| blackrouter-core | 1 | ✅ Pass |
| blackrouter-providers | 3 | ✅ Pass |
| blackrouter-rtk | 5 | ✅ Pass |
| blackrouter-storage | 14 | ✅ Pass |
| blackrouter-telegram | 10 | ✅ Pass |
| blackrouter-translator | 25 | ✅ Pass |
| blackrouter-cli | 0 | — |
| blackrouter-bin | 0 | — |
| **Total** | **94** | **✅ All Pass** |

---

## 📊 Metrics

- **Lines of Code:** ~5,000+ (growing; not re-measured this pass)
- **Crates:** 11
- **API Endpoints:** 30+
- **Supported Providers:** 15+
- **Wire Formats:** 9
- **Automated Tests:** 94

---

## 🔗 Related Documents

- [Development Plan](./DEVELOPMENT_PLAN.md)
- [README](../README.md)
