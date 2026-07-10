# BlackRouter — Roadmap Tiếp Theo (Phase 5+)

> Tài liệu này nối tiếp [`DEVELOPMENT_PLAN.md`](./DEVELOPMENT_PLAN.md) (đã hoàn thành Phase 0–4).
> Mục tiêu: định hướng phát triển dài hạn theo thứ tự ưu tiên, chia nhỏ thành các phase
> có thể thực hiện dần dần. Mỗi phase có mục tiêu, scope, và các bước công việc cụ thể.

## Bối cảnh hiện tại (thực tế từ code)

BlackRouter là LLM gateway/proxy viết Rust (Axum), workspace 10 crates. Đã vững:

- **Routing/proxy**: OpenAI chat/responses, Anthropic messages, Codex, CommandCode + format translation.
- **Resilience**: load-balancing (round-robin/weighted/least-conn/response-time), circuit breaker, RTK rate-limit (sliding window in-memory).
- **Cost guard**, **combos** (fallback chains), **context window trimming** (tiktoken/cl100k_base).
- **Persistence**: SQLite (`blackrouter-storage`) — providers, usage, requestDetails, kv, settings.
- **Observability sơ khởi**: Prometheus `/metrics`, RTK metrics; deps OpenTelemetry đã có nhưng chưa wired.
- **Control plane**: token-guarded API + setup UI; `/health`, `/version`.
- **Deployment**: Dockerfile + docker-compose.

Khoảng trống lớn:

- Không có CI / test harness diện rộng.
- OTel chưa nối vào traces.
- Health chỉ liveness (thiếu readiness / dependency checks).
- RTK + circuit-breaker state in-memory → mất khi restart.
- Không có conversation/memory layer, không multi-tenant, auth provider chưa granular.

## Nguyên tắc ưu tiên

1. **Khóa chất lượng trước, mở rộng sau** — không thêm tính năng thông minh khi chưa có CI + state bền vững.
2. **Đóng vòng quan sát** — mọi thay đổi lớn phải đo được (traces/metrics/dashboard).
3. **Tương thích ngược** — schema SQLite luôn migrate an toàn (đã có `migrate_schema`).
4. **Nhỏ và giao** — mỗi phase merge được độc lập, không block release.

---

## Phase 5 — Nền tảng tin cậy (P0, làm ngay)

Mục tiêu: build có thể tin cậy và deploy an toàn.

### 5.1 CI + Quality Gates
- GitHub Actions: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo build --release`.
- Chạy trên matrix (stable + MSRV nếu định nghĩa).
- Artifact: image docker từ `Dockerfile` sẵn có.
- Thêm `dev-dependencies` chuẩn (tokio test-util, mockito/hiper mock hoặc wiremock) vào `blackrouter-api`.

### 5.2 Health / Readiness phân biệt
- `/healthz` (liveness):进程 sống.
- `/readyz` (readiness): SQLite mở được + ≥1 provider active + RTK sẵn sàng.
- Dùng trong K8s/docker-compose `healthcheck`.

### 5.3 Durable RTK & Circuit Breaker
- Thêm bảng `rtk_state (key TEXT PK, kind TEXT, data TEXT, updated_at INTEGER)`.
- Snapshot định kỳ (5–10s) từ `RtkInner` (rate_limits, circuit_breakers) xuống SQLite.
- Restore lúc boot để không mất window sau restart/deploy.
- Key nên bao gồm `connection_id` (đã có sẵn ởusage) để per-connection đúng nghĩa.

**Đã implement (done):** bảng `rtk_state` + `Storage` load/save/prune; `Rtk::snapshot`/`restore` (+ blocking variants) và `RtkSnapshot`. Boot restore qua `AppState::new`; background snapshotter 10s. Restore conservative (window tái tạo như vừa bắt đầu, không over-report capacity); circuit giữ Open/HalfOpen. Test round-trip xanh.

**Definition of Done (Phase 5):** push lên main không break CI; deploy mới không gây rate-limit burst; `/readyz` đúng trạng thái.

---

## Phase 6 — Observability & Debug (P1, ROI cao)

Mục tiêu: thấy hệ thống thực sự làm gì.

### 6.1 Wire OpenTelemetry Traces
- Dùng deps sẵn có (`opentelemetry`, `tracing-opentelemetry`, `opentelemetry-otlp`).
- Mỗi request = 1 span: `proxy → translate → upstream call → stream/translate-back`.
- Gắn `trace_id` vào `requestDetails.data` để tra cứu sau.
- Cấu hình qua env (`OTEL_EXPORTER_OTLP_ENDPOINT`, on/off).

### 6.2 Request ID & Structured Logs
- Thêm `x-request-id` (ưu tiên inbound, nếu không có thì sinh UUID).
- Chuẩn hóa `tracing` span qua mọi proxy path (hiện `proxy_with_specific_provider`, `messages_proxy` chưa đồng bộ span).
- Log lỗi provider kèm request_id + provider + model.

### 6.3 Grafana Dashboard Template
- Dashboard đi kèm `/metrics`: `blackrouter_requests_total`, `request_duration_seconds`, `tokens_total`, `stream_ttfb_seconds`.
- Biến: provider / model / status.
- Panel cost (từ `usageHistory`) + rate-limit (từ RTK metrics).
- File `docs/grafana/blackrouter.json` + ví dụ Prometheus scrape config trong `docker-compose.yml`.

**Đã implement (done):**
- **6.1** — `telemetry.rs`: `init_layer()` build OTLP `TracerProvider` (tonic) khi `OTEL_EXPORTER_OTLP_ENDPOINT` set; gắn `tracing_opentelemetry::layer` vào subscriber qua `main.rs` (`registry().with(otel_layer)...`). Provider giữ sống suốt process qua `OnceLock`. Trace spans sinh tự động từ `tracing` (middleware `request` span + `TraceLayer`).
- **6.2** — middleware `request_id_middleware`: ưu tiên `x-request-id` inbound, sinh UUID nếu thiếu; ghi vào `Span` field `request_id` + `Extensions`; echo response header `x-request-id`. `request_id` truyền qua chain (`chat_completions_shell`/`responses_proxy`/`messages_proxy` → `proxy_chat_completions` → `proxy_single_chat_completion` → `proxy_with_specific_provider`) và gắn vào `requestDetails.data.request_id`.
- **6.3** — `deploy/observability/`: `prometheus.yml` (scrape `/metrics`), `grafana/dashboards/blackrouter.json` (4 panel: request rate, p95 duration, tokens/s, stream ttfb p95; biến provider/model), `grafana/provisioning/` (datasource + dashboard provider). `docker-compose.yml` thêm service `prometheus` + `grafana`.

Kích hoạt: `OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 cargo run`. Dashboard: `deploy/observability/grafana/...`.

**Definition of Done (Phase 6):** 1 request lỗi có thể trace end-to-end qua OTel; dashboard hiển thị cost/rate-limit realtime (bổ trợ trực tiếp màn Limits).

---

## Phase 7 — Routing thông minh (P2, giá trị cốt lõi)

Mục tiêu: tận dụng metadata provider đã có để route tối ưu.

### 7.1 Normalized Model Catalog
- Bảng/chuẩn `model_catalog`: provider, model, context_window, modalities, unit_price_in/out, latency_p50.
- Hiện `provider.data.models` là mảng string; nâng thành struct có metadata.
- Dùng cho routing + hiển thị (màn Models/Combos).

### 7.2 Cost-Aware Routing
- Cost guard hiện chỉ **block** khi vượt ngân sách.
- Mở rộng: khi có nhiều provider cùng model, ưu tiên provider rẻ nhất còn healthy.
- Kết hợp với `usageHistory` (per-connection) để ước chi phí thực tế.

### 7.3 Smart Fallback / Hedging
- Combo hiện là sequential fallback.
- Thêm mode: `hedge` (gửi N provider, lấy response nhanh nhất hợp lệ) và fallback theo loại lỗi (429 → next; 5xx → next; content-policy → dừng).
- Cấu hình per-combo (`strategy: fallback|hedge`, `hedge_min_latency_ms`).

### 7.4 Capability/SL功 Routing
- Route theo capability (vision/audio/function-calling) thay vì chỉ tên model.
- Dùng `model_catalog` để chọn provider thỏa mãn request.

**Đã implement (done):**
- **7.1** — bảng `model_catalog (provider, model, context_window, modalities, price_in/out_per_million, latency_p50_ms)` + `ModelCatalogEntry` + `upsert_model_catalog` / `load_model_catalog` / `get_model_catalog`. Backward-compatible (không đổi schema cũ).
- **7.2** — trong `proxy_single_chat_completion`, sau khi lọc cooldown/circuit, sort `available` theo `provider_estimated_cost` (dùng catalog price nếu có, else `price_per_million`) — provider rẻ nhất được thử trước (stable sort giữ thứ tự cho equal-cost).
- **7.3** — (a) **Fallback-by-error**: 429/5xx → continue fallback; 4xx client (400/401/403/404/422) → dừng ngay (áp dụng cả ở combo model-loop và provider-loop). (b) **Provider hedge** (non-streaming): gửi đồng thời 2 provider rẻ nhất qua `tokio::join!`, trả response đầu tiên hợp lệ; nếu cả 2 fail → fallback sequential. Streaming giữ sequential (SSE không race-friendly).
- **7.4** — `request_modalities` trích vision/tools từ request; `filter_by_capability` ưu tiên provider có catalog `modalities` chứa capability cần; provider thiếu metadata hoặc tất cả bị loại → giữ nguyên (không hard-drop).

Test: `retryable_errors_are_transient_only`, `request_modalities_detects_vision_and_tools` thêm vào `blackrouter-api`.

**Definition of Done (Phase 7):** chọn provider tự động theo giá/độ khỏe; combo hỗ trợ hedge; routing theo capability.

---

## Phase 8 — Conversation & Memory (P2, hướng agentic — tùy hướng sản phẩm)

> Chỉ làm nếu BlackRouter nhắm vào agent platform. Nếu là công cụ cá nhân/OSS, có thể bỏ qua.

### 8.1 Session/Conversation Store
- Tận dụng bảng `kv (scope, key, value)` sẵn có làm store, hoặc thêm `conversations` / `sessions`.
- Lưu lịch sử messages, gắn `session_id`.

### 8.2 Tích hợp Context Trimming
- Hiện `trim_context_to_fit` chạy per-request trên `messages` truyền vào.
- Nâng: khi có session, trim dựa trên lịch sử đã lưu + budget context_window (đã có sẵn logic token counting).

### 8.3 (Tùy chọn) Semantic Memory
- Nếu cần recall: thêm vector store (sqlite-vss hoặc external), index theo session/user.
- Đây là bước lớn — chỉ khi có nhu cầu thực tế.

**Đã implement (done):**
- **8.1** — bảng `kv` sẵn có tái sử dụng làm store với `scope = "conversation"`, key = `session_id` (`blackrouter-storage`: thêm `get_kv` / `set_kv` / `delete_kv` / `list_kv`). Header `x-session-id` bật conversation memory trên `/v1/chat/completions`: load lịch sử đã lưu → merge với `messages` incoming (system message incoming thay thế system block đã lưu; các message còn lại append) → gửi; sau response thành công, append assistant message và persist. Response echo `x-session-id`. Control endpoints (gated bởi control-token): `GET /api/conversations`, `GET|DELETE /api/conversations/{session_id}`.
- **8.2** — `assemble_session_body` merge lịch sử + trim toàn bộ history đã merge bằng `trim_context_to_fit`, dùng `route_context_window` (min của model_catalog / built-in table, default 128k) và `route_max_output_tokens` làm budget; `proxy_with_specific_provider` vẫn re-trim per-provider làm backstop. Tái dụng `count_chat_tokens` (cl100k_base). Streaming vẫn giữ latency: body được tee (forward + accumulate) qua `wrap_session_response`, capture assistant message sau khi stream kết thúc (cả JSON và SSE, có reconstruct `tool_calls`).
- **8.3** — chưa làm (optional, không có nhu cầu thực tế). Lưu ý: session hiện chỉ tích hợp `/v1/chat/completions`; `/v1/messages` (Anthropic) và `/v1/responses` là follow-up.

**Definition of Done (Phase 8):** duy trì hội thoại qua nhiều request; context tự động trim từ lịch sử.

---

## Phase 9 — Vận hành quy mô (P3, khi có người dùng thật)

### 9.1 Multi-Tenancy & API Key Scoping
- API key hiện chỉ bật/tắt (`require_api_key`).
- Nâng: mỗi key gắn quota (requests/tokens/cost), provider-allowlist, model-allowlist.
- Ảnh hưởng schema sớm → làm sớm nếu định thương mại hóa.

### 9.2 Proactive Provider Health Probing
- Circuit breaker hiện bị động (sau fail).
- Thêm background health-check định kỳ (ping model list / lightweight completion) để đưa provider in/out trước khi user受影响.

### 9.3 Config Hot-Reload & Versioning
- `settings` hiện 1 row JSON.
- Nâng: versioned + reload không restart (hiện `save_setup_config` ghi đè).

### 9.4 Horizontal Scaling
- RTK in-memory + SQLite local → không scale ngang.
- Khi cần: rate-limit state ra Redis/shared store; SQLite thành central DB hoặc Postgres.
- `ResponseCache` cũng cần external (Redis) nếu nhiều replica.

**Definition of Done (Phase 9):** nhiều tenant cách ly quota; deploy nhiều replica không mất rate-limit state.

---

## Phase 10 — Sản phẩm & UX (song song)

### 10.1 Setup UI hoàn thiện
- Provider test flow trực quan (đã có `showProviderModels`, `test_provider`).
- Aliases management UI.
- Cost guard config UI (hiện chỉ qua settings JSON).
- Màn Limits/Combos đã cải thiện — tiếp tục polish mobile/empty-state.

### 10.2 CLI trưởng thành (`blackrouter-cli`)
- Tool quản trị: apply config, migrate, `doctor`, export usage.
- Thay vì chỉ dev utility.

### 10.3 Docs & Runbooks
- README có sẵn; bổ sung runbook vận hành, mẫu docker-compose với Prometheus/Grafana, troubleshooting.

---

## Ma trận ưu tiên

| Ưu tiên | Hạng mục | Nỗ lực | Tác động | Phase |
|--------|----------|--------|----------|-------|
| P0 | CI + clippy + tests | Thấp | Tin cậy build | 5.1 |
| P0 | `/readyz` + durable RTK | TB | Ổn định deploy | 5.2–5.3 |
| P1 | Wire OpenTelemetry traces | TB | Debug/observe | 6.1–6.2 |
| P1 | Grafana dashboard | Thấp | Vận hành | 6.3 |
| P2 | Cost-aware / smart fallback | Cao | Giá trị cốt lõi | 7.2–7.3 |
| P2 | Conversation memory | Cao | Hướng agentic | 8.x |
| P3 | Multi-tenant + quota | Cao | Thương mại hóa | 9.1 |
| P3 | Redis/shared RTK | Cao | Scale ngang | 9.4 |

## Lộ trình đề xuất (theo tuần, ước tính)

1. **Tuần 1**: Phase 5 (CI + readiness + durable RTK).
2. **Tuần 2**: Phase 6 (OTel + dashboard).
3. **Tuần 3–4**: Phase 7 (model catalog + cost-aware + smart fallback).
4. **Tuần 5+**: Phase 8 (nếu agentic) / Phase 9 (nếu thương mại) / Phase 10 song song.

## Quyết định cần làm rõ sớm

- **Hướng sản phẩm**: công cụ cá nhân/OSS hay nền tảng thương mại? → quyết định có làm Phase 8 / 9.1 sớm.
- **Scale mục tiêu**: single-node hay cluster? → quyết định Phase 9.4 (Redis/Postgres).
- **Observability stack**: Prometheus+Grafana (có sẵn) hay vendor (Datadog/OTel collector)? → Phase 6.

---

## Checklist phát triển dần

- [x] 5.1 CI workflow (fmt/clippy/test/build)
- [x] 5.2 `/healthz` + `/readyz`
- [x] 5.3 Bảng `rtk_state` + snapshot/restore
- [x] 6.1 OTel trace spans qua proxy
- [x] 6.2 `x-request-id` + structured logs
- [x] 6.3 Grafana dashboard JSON + compose example
- [x] 7.1 Normalized model catalog
- [x] 7.2 Cost-aware routing
- [x] 7.3 Combo hedge/fallback-by-error
- [x] 7.4 Capability routing
- [x] 8.1 Session store (nếu agentic)
- [ ] 9.1 Multi-tenant API key scoping
- [ ] 9.2 Proactive health probing
- [ ] 9.3 Config hot-reload
- [ ] 9.4 Shared RTK/Cache (nếu scale ngang)
- [ ] 10.x UI/CLI/Docs polish
