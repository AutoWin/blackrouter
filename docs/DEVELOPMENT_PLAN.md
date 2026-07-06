# BlackRouter Development Plan

_Updated: 2026-07-06_

## Design Principles

> **Ưu tiên:** Hiệu năng nhanh • Tương thích cao

```
┌─────────────────────────────────────────────────────────────┐
│  PERFORMANCE FIRST                                          │
│  ─ Zero-copy khi có thể                                    │
│  ─ Connection pooling & reuse                              │
│  ─ Async streaming không buffer toàn bộ                    │
│  ─ Minimal allocation trong hot path                       │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│  MAXIMUM COMPATIBILITY                                      │
│  ─ Drop-in replacement cho 9Router                         │
│  ─ OpenAI-compatible API 100%                              │
│  ─ Hỗ trợ tất cả providers phổ biến                       │
│  ─ Graceful degradation khi provider lỗi                   │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│  AGENT-READY                                                │
│  ─ Tool call (function calling) 100% passthrough           │
│  ─ Streaming SSE cho agent tool output                     │
│  ─ Token usage tracking cho context management             │
│  ─ Long-running request support (tool exec, subagent)      │
│  ─ First-class client: Hermes Agent                        │
└─────────────────────────────────────────────────────────────┘
```

---

## Table of Contents

- [Current Status](#current-status)
- [Hermes Agent Integration](#hermes-agent-integration)
- [Phase 1: Performance & Streaming](#phase-1-performance--streaming)
- [Phase 2: Compatibility & Providers](#phase-2-compatibility--providers)
- [Phase 3: Production Hardening](#phase-3-production-hardening)
- [Phase 4: Advanced Features](#phase-4-advanced-features)
- [Performance Targets](#performance-targets)
- [Compatibility Matrix](#compatibility-matrix)
- [Timeline](#timeline)

---

## Current Status

### ✅ Completed (Phase 0)

**Infrastructure:**
- Rust workspace with 10 crates
- SQLite storage with 9Router-compatible schema
- Config loader with env mapping (`BLACKROUTER_*`, legacy `DATA_DIR`, `PORT`)
- Axum HTTP server with graceful shutdown

**API Endpoints:**
- `GET /health`, `/version`, `/api/runtime/status`
- `GET|PUT /api/setup/config`
- `GET|POST /api/setup/api-keys`
- `CRUD /api/setup/providers` + toggle/test/models
- `GET /api/setup/provider-catalog`
- `CRUD /api/setup/combos`
- `GET /v1/models`, `/v1beta/models`
- `POST /v1/chat/completions` (non-streaming proxy)
- `POST /v1/responses`, `/v1/messages` (shells)
- `GET /api/rtk/metrics`, `/api/rtk/status/{provider}/{model}`

**Provider Translators:**
- OpenAI Chat ↔ Claude Messages
- OpenAI Chat ↔ Gemini
- OpenAI Chat → CommandCode, Cursor, Kiro, Antigravity, GeminiCli
- Response translation back to OpenAI format
- **Tool call translation (Phase 1.5):** tools, tool_choice, tool_use/tool_result, functionCall/functionResponse
- **Streaming SSE passthrough (Phase 1.1):** zero-copy byte forwarding for OpenAI→OpenAI
- **Translated streaming fallback:** strip stream flag, translate, convert to single SSE event
- **Connection pooling (Phase 1.2):** shared `reqwest::Client` in `AppState`
- **Token usage parsing (Phase 3.1 partial):** parse prompt/completion tokens from upstream response

**RTK (Real-Time Kit):**
- Rate limiting (requests/min, tokens/min, concurrent)
- Request tracking (success/fail, latency, tokens)
- Metrics with percentiles (p95, p99)
- Thread-safe with atomic counters

**Telegram Bot:**
- Full Bot API client
- Long polling runtime
- Command parser (25+ commands)
- Admin authorization
- HTML formatted responses

**Other:**
- Dockerfile & docker-compose.yml
- Setup UI (HTML/CSS/JS)
- Built-in provider catalogs (Cline, CommandCode)

---

## Hermes Agent Integration

**Strategy:** BlackRouter làm LLM gateway cho [Hermes Agent](https://github.com/nousresearch/hermes-agent)

**Approach:** Option A — Zero-code integration (config-only)

```
┌──────────────────────────────────────────────────────────┐
│  ARCHITECTURE                                             │
│                                                           │
│  Hermes Agent (Python)                                    │
│    │                                                      │
│    │  POST /v1/chat/completions                           │
│    │  Authorization: Bearer <blackrouter-key>             │
│    │  { "model": "provider/model", "stream": true,       │
│    │    "tools": [...], "tool_choice": "auto" }           │
│    ▼                                                      │
│  BlackRouter (Rust)                                       │
│    ├─ Auth (API key validation)                           │
│    ├─ Route resolution (single / combo fallback)          │
│    ├─ Format translation (OpenAI ↔ Claude ↔ Gemini)       │
│    ├─ Tool call passthrough                               │
│    ├─ Streaming SSE forward                               │
│    ├─ RTK (rate limit + metrics)                          │
│    └─ Token usage tracking                                │
│    │                                                      │
│    ▼                                                      │
│  Upstream Providers                                       │
│    OpenAI | Claude | Gemini | OpenRouter | DeepSeek ...   │
└──────────────────────────────────────────────────────────┘
```

**Hermes nhận được:**
- Combo fallback — provider lỗi tự chuyển model B
- Định dạng dịch — nói OpenAI format, BlackRouter dịch sang Claude/Gemini
- Rate limiting + metrics tập trung
- Quản lý key tập trung (single source of truth)

**BlackRouter nhận được:**
- Agent workload thực tế để validate performance targets
- Test case streaming TTFT, tool call passthrough, long-running requests
- Subagent parallelism = concurrent connection test

### Hermes Blocker Matrix

Các item trong plan được đánh dấu `🔴 Hermes Blocker` — phải hoàn thành
trước khi Hermes hoạt động ổn định.

| Blocker | Phase | Mức độ | Lý do |
|---------|-------|--------|-------|
| Streaming SSE | 1.1 | 🔴 P0 | Không stream = Hermes treo khi agent chạy tool |
| Tool call passthrough | 1.5 (mới) | 🔴 P0 | Agent không gọi tool được = không hoạt động |
| Connection pooling | 1.2 | 🟡 P1 | Mỗi request tạo client mới = latency cao |
| Token usage tracking | 3.1 | 🟡 P1 | Hermes /compress cần token count để nén context |
| `/v1/responses` proxy | 2.1 | 🟡 P1 | Một số model yêu cầu Responses API |
| `/v1/messages` proxy | 2.1 | 🟡 P1 | Claude-native endpoint cho Hermes |
| Long-running timeout | 1.2 | 🟡 P1 | Tool exec + subagent có thể chạy vài phút |

### Hermes Configuration

```yaml
# Hermes config → trỏ vào BlackRouter
provider: openai-compatible
base_url: http://blackrouter:20130/v1
api_key: <blackrouter-api-key>
model: openai/gpt-4o  # hoặc claude/claude-sonnet-4, gemini/gemini-2.0-flash
```

```bash
# Hoặc qua CLI
hermes model
# → Choose: Custom OpenAI-compatible endpoint
# → Base URL: http://blackrouter:20130/v1
# → API Key: <blackrouter-api-key>
```

---

## Phase 1: Performance & Streaming

**Goal:** Sub-100ms latency, zero-buffer streaming, connection reuse

**Priority:** 🔴 Critical — Ưu tiên cao nhất

### 1.1 Zero-Copy Streaming (SSE)

**Impact:** 🔥 High — Giảm 50-70% latency cho streaming

**🔴 Hermes Blocker** — Agent stream tool output, reasoning dài hàng chục giây.
Không stream = Hermes treo hoàn toàn cho tới khi upstream trả xong.

**Tasks:**
- [x] Đọc `body["stream"]` — nhánh streaming riêng trong `proxy_single_chat_completion`
- [x] Stream bytes trực tiếp từ upstream → client (zero-copy)
- [x] Parse chỉ header, không parse body khi passthrough
- [x] Chunked transfer encoding cho response
- [x] Combo fallback cho streaming: check upstream status trước khi stream, fallback nếu non-2xx
- [x] SSE `data: [DONE]` sentinel handling (cho translated streaming → single SSE event)
- [x] `Content-Type: text/event-stream` header passthrough

**Technical Approach:**
```rust
// Zero-copy: chỉ forward bytes, không deserialize
async fn proxy_stream_zero_copy(
    upstream: reqwest::Response,
    client_writer: &mut hyper::body::Sender,
) -> Result<()> {
    let mut stream = upstream.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        client_writer.send_data(chunk).await?;
    }
    Ok(())
}

// Chỉ parse khi cần translate format
async fn proxy_stream_with_translation(
    upstream: reqwest::Response,
    from: WireFormat,
    to: WireFormat,
    client_writer: &mut hyper::body::Sender,
) -> Result<()> {
    // Parse SSE events, translate, forward
}
```

**Performance Target:**
```
Non-streaming: p50 < 50ms, p99 < 200ms (excluding upstream)
Streaming TTFT: < 100ms
Streaming throughput: > 1GB/s
```

**Files:**
- `crates/blackrouter-api/src/proxy.rs` - New module
- `crates/blackrouter-translator/src/stream.rs` - Stream translation

---

### 1.2 Connection Pooling & Reuse

**Impact:** 🔥 High — Giảm 30-50% latency cho repeated requests

**🔴 Hermes Blocker** — Agent gọi LLM liên tục, nhiều request/giây.
~~Code hiện tại tạo `reqwest::Client` mới mỗi request~~ → Đã sửa: shared client trong `AppState`.

**Tasks:**
- [x] Shared `reqwest::Client` cho tất cả providers (thay vào `AppState`)
- [x] HTTP/2 multiplexing — reqwest feature `http2` đã enable, ALPN negotiation tự động
- [x] DNS caching (built-in reqwest/hyper)
- [x] TLS session reuse (built-in reqwest với connection pooling)
- [x] Per-provider connection pool config (via shared client config)
- [x] **Long-running timeout: 600s+** — tool exec và subagent có thể chạy vài phút

**Implementation:**
```rust
// Shared client với connection pool
lazy_static! {
    static ref HTTP_CLIENT: reqwest::Client = reqwest::Client::builder()
        .pool_max_idle_per_host(100)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(60))
        .tcp_nodelay(true)
        .http2_prior_knowledge(false)
        .http2_keep_alive_interval(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()
        .expect("Failed to build HTTP client");
}
```

**Performance Target:**
```
Connection reuse rate: > 90%
DNS lookup: 0ms (cached)
TLS handshake: < 50ms (session reuse)
```

**Files:**
- `crates/blackrouter-api/src/client.rs` - New module
- `crates/blackrouter-api/src/lib.rs` - Use shared client

---

### 1.3 Memory Optimization

**Impact:** 🟡 Medium — Giảm memory usage 30-50%

**Tasks:**
- [ ] Use `bytes::Bytes` thay vì `Vec<u8>`
- [ ] Avoid unnecessary cloning
- [ ] Use `Arc<str>` cho shared strings
- [ ] Implement request body streaming
- [ ] Bounded buffers cho streaming

**Techniques:**
```rust
// Zero-copy string sharing
let model: Arc<str> = Arc::from("gpt-4");

// Bytes cho request/response body
let body: Bytes = response.bytes().await?;

// Avoid clone với reference
fn process_body(body: &Value) -> Result<()> {
    // Process without cloning
}
```

---

### 1.4 Async Optimization

**Impact:** 🟡 Medium — Giảm context switching overhead

**Tasks:**
- [ ] Use `tokio::task::spawn_local` cho CPU-bound work
- [ ] Batch database operations
- [ ] Use `tokio::sync::Notify` thay vì channels khi có thể
- [ ] Implement request coalescing cho identical requests

---

### 1.5 Tool Call Passthrough

**Impact:** 🔥 Critical — Agent framework phụ thuộc hoàn toàn vào tool calls

**🔴 Hermes Blocker (P0)** — Translator hiện tại KHÔNG hỗ trợ `tools`, `tool_choice`,
`function_call`. Agent không gọi tool được = không hoạt động.

**Current State:** ~~`grep` toàn bộ codebase — không tìm thấy `tool_call`, `tool_choice`,
`function_call`, `tools` ở đâu. Translator `openai_to_claude`, `openai_to_gemini`
bỏ qua các field này.~~ → Đã sửa: tool call translation đã implement.

**Tasks:**
- [x] Passthrough `tools` và `tool_choice` cho OpenAI-compatible providers (không cần dịch)
- [x] Translate `tools` / `tool_choice` từ OpenAI format → Claude format (`tools` array với `input_schema`)
- [x] Translate `tools` / `tool_choice` từ OpenAI format → Gemini format (`function_declarations`)
- [x] Translate response `tool_calls` array về OpenAI format từ Claude (`tool_use` blocks) và Gemini (`functionCall`)
- [ ] Translate streaming `tool_calls` delta chunks (partial JSON accumulation) — Phase 2.3
- [x] Support `tool` role messages (tool results) trong message translation
- [x] Handle `parallel_tool_calls` parameter

**Technical Approach:**
```rust
// OpenAI tools format → Claude tools format
fn translate_tools_openai_to_claude(tools: &[Value]) -> Vec<Value> {
    tools.iter().map(|tool| {
        let function = tool.get("function").unwrap();
        json!({
            "name": function["name"],
            "description": function["description"],
            "input_schema": function["parameters"]
        })
    }).collect()
}

// Claude tool_use → OpenAI tool_calls
fn translate_tool_calls_claude_to_openai(content: &[Value]) -> Vec<Value> {
    content.iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
        .map(|b| json!({
            "id": b["id"],
            "type": "function",
            "function": {
                "name": b["name"],
                "arguments": b["input"].to_string()
            }
        }))
        .collect()
}
```

**Files:**
- `crates/blackrouter-translator/src/lib.rs` - Thêm tool call translation
- `crates/blackrouter-translator/src/tools.rs` - New module cho tool call logic
- `crates/blackrouter-api/src/lib.rs` - Passthrough `tools`/`tool_choice` cho OpenAI providers

---

## Phase 2: Compatibility & Providers

**Goal:** 100% OpenAI-compatible, support tất cả providers phổ biến

**Priority:** 🔴 Critical — Tương thích là core value

### 2.1 OpenAI API Compatibility

**Impact:** 🔥 High — Client compatibility

**🔴 Hermes Blocker** — ~~Hermes có thể gọi `/v1/responses` (OpenAI Responses API)
và `/v1/messages` (Anthropic Messages API) tuỳ model. Hiện tại cả hai đều là shell.~~
→ Đã implement: cả hai endpoint đều proxy thật.

**Tasks:**
- [ ] Verify 100% OpenAI Chat Completions API compatibility
- [ ] Support tất cả parameters:
  - `temperature`, `top_p`, `n`
  - `stream`, `stream_options`
  - `stop`, `max_tokens`
  - `presence_penalty`, `frequency_penalty`
  - `logit_bias`, `user`
  - `response_format`, `seed`
  - `tools`, `tool_choice`, `function_call` ← **critical cho Hermes**
  - `logprobs`, `top_logprobs`
  - `parallel_tool_calls` ← **Hermes subagent delegation**
- [ ] Support tất cả response fields
- [ ] Error format compatibility
- [ ] Rate limit headers compatibility
- [x] **Implement `/v1/responses` proxy** — convert Responses→Chat, proxy, convert back
- [x] **Implement `/v1/messages` proxy** — Claude passthrough hoặc Claude→OpenAI→Claude translation
- [x] Tool call response format: `choices[].message.tool_calls[]`

**Compatibility Test:**
```rust
#[test]
fn test_openai_compatibility() {
    // Test với OpenAI Python SDK
    // Test với OpenAI Node SDK
    // Test với curl commands
    // Verify response format 100% match
}
```

**Files:**
- `crates/blackrouter-api/src/openai.rs` - OpenAI compatibility layer
- `tests/openai_compat_test.rs` - Integration tests

---

### 2.2 Complete Provider Translators

**Impact:** 🔥 High — Provider coverage

**Priority Order:**

| Provider | Format | Priority | Status |
|----------|--------|----------|--------|
| OpenAI | OpenAI | 🔴 Critical | ✅ Done |
| Claude | Claude Messages | 🔴 Critical | ✅ Done |
| Gemini | Gemini | 🔴 Critical | ✅ Done |
| OpenRouter | OpenAI | 🔴 Critical | ✅ Done |
| DeepSeek | OpenAI | 🟡 High | ✅ Done |
| Groq | OpenAI | 🟡 High | ✅ Done |
| Mistral | OpenAI | 🟡 High | ✅ Done |
| CommandCode | Custom | 🟡 High | ✅ Done |
| Cline | OpenAI | 🟡 High | ✅ Done |
| Cursor | OpenAI | 🟡 High | ⏳ Partial |
| Kiro | Custom | 🟢 Medium | ⏳ Partial |
| Antigravity | OpenAI | 🟢 Medium | ⏳ Partial |
| Codex | OpenAI Responses | 🟢 Medium | ❌ TODO |
| GitHub | OpenAI | 🟢 Medium | ❌ TODO |
| Ollama | OpenAI | 🟢 Medium | ✅ Done |

**Tasks:**
- [ ] Complete Cursor provider (SSE parsing)
- [ ] Complete Kiro provider (response parsing)
- [ ] Complete Antigravity provider
- [ ] Implement Codex provider (Responses API)
- [ ] Implement GitHub Copilot provider
- [ ] Add provider-specific header injection
- [ ] Add provider-specific error mapping

**Files:**
- `crates/blackrouter-translator/src/cursor.rs`
- `crates/blackrouter-translator/src/kiro.rs`
- `crates/blackrouter-translator/src/codex.rs`

---

### 2.3 Streaming Format Translation

**Impact:** 🔥 High — Streaming compatibility

**Tasks:**
- [x] Claude SSE → OpenAI SSE translator (event-by-event, real-time)
- [x] Gemini SSE → OpenAI SSE translator (event-by-event, real-time)
- [ ] OpenAI SSE → Claude SSE (reverse) — không cần cho Hermes
- [x] Handle partial chunks correctly (tool_call delta accumulation)
- [x] Preserve usage information in stream (prompt_tokens, completion_tokens in final chunk)

**Streaming Compatibility:**

| From → To | Non-Stream | Stream |
|-----------|-----------|--------|
| OpenAI → OpenAI | ✅ | ✅ |
| OpenAI → Claude | ✅ | ✅ |
| OpenAI → Gemini | ✅ | ✅ |
| Claude → OpenAI | ✅ | ✅ |
| Gemini → OpenAI | ✅ | ✅ |

---

### 2.4 9Router Compatibility

**Impact:** 🔥 High — Drop-in replacement

**Tasks:**
- [ ] Compatible SQLite schema
- [ ] Compatible config format
- [ ] Compatible API endpoints
- [ ] Migration tool from 9Router
- [ ] Documentation for migration

**Compatibility Checklist:**
```
✅ Database schema compatible
✅ Environment variables compatible
⬜ API response format identical
⬜ Error format identical
⬜ Rate limiting compatible
⬜ Usage tracking compatible
```

---

## Phase 3: Production Hardening

**Goal:** Production-ready, monitoring, security

### 3.1 Usage Tracking

**Priority:** 🟡 High

**🔴 Hermes Blocker** — ~~Code hiện tại hardcode `0, 0, 0.0` cho token count (L1346).~~
→ Đã sửa: `parse_token_usage()` parse `usage` từ upstream response, truyền vào `record_request_end`.
Hermes dùng `/usage` để hiển thị token consumed và `/compress` để nén context
khi gần full. Token = 0 → Hermes không biết khi nào compress → tràn context silently.

**Tasks:**
- [x] Parse `usage` field từ upstream response JSON (`prompt_tokens`, `completion_tokens`, `total_tokens`)
- [x] RTK token tracking — `record_request_end` nhận `prompt_tokens`, `completion_tokens` thực tế
- [x] Record each request to `usageHistory` — `Storage::record_usage()` + `record_usage_async()`
- [x] Usage API endpoint — `GET /api/usage?since=<timestamp>`
- [x] Aggregate daily stats to `usageDaily` — `Storage::aggregate_daily_usage()` + `GET /api/usage/daily` + `POST /api/usage/aggregate`
- [x] Store request details in `requestDetails` — `Storage::record_request_details()` + `record_request_details_async()`
- [x] Calculate costs per provider/model — `calculate_cost()` với static price table (OpenAI, Claude, Gemini, DeepSeek, Groq, Mistral)
- [x] Async writes (không block request path) — `tokio::spawn` fire-and-forget
- [x] **Parse token usage từ streaming SSE** — SSE stream translator extracts usage từ final event (Claude `message_delta`, Gemini `usageMetadata`)

**Performance Note:**
```rust
// Async usage recording - không block response
tokio::spawn(async move {
    storage.record_usage(usage_data).await.ok();
});
```

---

### 3.2 Monitoring & Metrics

**Priority:** 🟡 High

**Tasks:**
- [ ] Prometheus metrics endpoint (`GET /metrics`)
- [ ] Structured logging với request IDs
- [ ] Distributed tracing (OpenTelemetry)
- [ ] Health check dashboard

**Key Metrics:**
```prometheus
# Performance
blackrouter_request_duration_seconds{provider,model,status}
blackrouter_stream_ttfb_seconds{provider,model}
blackrouter_connection_pool_size{provider}
blackrouter_connection_reuse_total{provider}

# Business
blackrouter_requests_total{provider,model,status}
blackrouter_tokens_total{provider,model,type}

# System
blackrouter_memory_usage_bytes
blackrouter_open_connections
```

---

### 3.3 Authentication & Security

**Priority:** 🟡 High

**Tasks:**
- [ ] OAuth flow cho GitHub, Codex, Cursor
- [ ] API key rotation
- [ ] Rate limiting per API key
- [ ] Input validation
- [ ] CORS configuration

---

### 3.4 Telegram Enhancements

**Priority:** 🟢 Medium

**Tasks:**
- [ ] Webhook mode (`POST /telegram/webhook`)
- [ ] Inline keyboard cho confirmations
- [ ] Callback query handling
- [ ] Usage command với real data

---

## Phase 4: Advanced Features

**Goal:** Load balancing, caching, advanced routing

### 4.1 Load Balancing

**Priority:** 🟢 Medium

**Tasks:**
- [ ] Round-robin
- [ ] Weighted round-robin
- [ ] Least connections
- [ ] Response time based
- [ ] Circuit breaker pattern

---

### 4.2 Response Caching

**Priority:** 🟢 Medium

**Tasks:**
- [ ] Cache identical requests
- [ ] Configurable TTL
- [ ] Cache key: model + messages hash
- [ ] LRU eviction

**Performance Note:**
```rust
// Cache hit: 0ms latency
if let Some(cached) = cache.get(&cache_key).await {
    return Ok(cached);
}
```

---

### 4.3 Advanced Routing

**Priority:** 🟢 Medium

**Tasks:**
- [ ] A/B testing
- [ ] Model aliases
- [ ] Model families (`gpt-4-*`)
- [ ] Request queuing

---

## Performance Targets

### Latency Targets

| Metric | Target | Current |
|--------|--------|---------|
| Non-streaming p50 | < 50ms* | N/A |
| Non-streaming p99 | < 200ms* | N/A |
| Streaming TTFT | < 100ms | N/A |
| Streaming p99 | < 500ms | N/A |

*_Excluding upstream provider latency_

### Throughput Targets

| Metric | Target | Current |
|--------|--------|---------|
| Requests/sec | > 1,000 | N/A |
| Streaming throughput | > 1GB/s | N/A |
| Concurrent connections | > 1,000 | N/A |

### Resource Targets

| Metric | Target | Current |
|--------|--------|---------|
| Memory (idle) | < 50MB | ~30MB |
| Memory (1K conn) | < 200MB | N/A |
| CPU (1K req/s) | < 50% | N/A |
| Connection reuse | > 90% | N/A |

### Optimization Techniques

```
✅ Zero-copy streaming
✅ Connection pooling
✅ HTTP/2 multiplexing
✅ DNS caching
✅ TLS session reuse
✅ Async usage recording
⬜ Response caching
⬜ Request coalescing
⬜ Memory-mapped files
```

---

## Compatibility Matrix

### Client Compatibility

| Client | Status | Notes |
|--------|--------|-------|
| **Hermes Agent** | ✅ | MVP ready — passthrough streaming + tool calls + token tracking |
| OpenAI Python SDK | ✅ | Full compatibility |
| OpenAI Node SDK | ✅ | Full compatibility |
| OpenAI Go SDK | ✅ | Full compatibility |
| curl | ✅ | Full compatibility |
| Zed Editor | ✅ | Full compatibility |
| VS Code (Continue) | ✅ | Full compatibility |
| Cursor IDE | ⚠️ | Partial (SSE) |
| Cline | ✅ | Full compatibility |

### Provider Compatibility

| Provider | Non-Stream | Stream | Models | Status |
|----------|-----------|--------|--------|--------|
| OpenAI | ✅ | ✅ | ✅ | Production |
| Claude | ✅ | ⏳ | ✅ | Production |
| Gemini | ✅ | ⏳ | ✅ | Production |
| OpenRouter | ✅ | ✅ | ✅ | Production |
| DeepSeek | ✅ | ✅ | ✅ | Production |
| Groq | ✅ | ✅ | ✅ | Production |
| Mistral | ✅ | ✅ | ✅ | Production |
| CommandCode | ✅ | ❌ | ✅ | Production |
| Cline | ✅ | ✅ | ✅ | Production |
| Cursor | ⚠️ | ⚠️ | ✅ | Beta |
| Kiro | ⚠️ | ❌ | ✅ | Beta |
| Ollama | ✅ | ✅ | ✅ | Production |

### 9Router Compatibility

| Feature | Status | Notes |
|---------|--------|-------|
| Database schema | ✅ | Compatible |
| Config format | ✅ | Compatible |
| API endpoints | ⚠️ | Partial |
| Error format | ⚠️ | Partial |
| Rate limiting | ❌ | Different impl |
| Usage tracking | ❌ | Different impl |

### Hermes Agent Compatibility

| Feature | Status | Blocker Phase | Notes |
|---------|--------|---------------|-------|
| Non-streaming chat | ✅ | — | Hoạt động ngay |
| Streaming chat (passthrough) | ✅ | — | Zero-copy SSE, OpenAI→OpenAI |
| Streaming chat (translated) | ✅ | — | SSE event-by-event translation, Claude/Gemini → OpenAI |
| Tool calls (passthrough) | ✅ | — | OpenAI→OpenAI, `tools`/`tool_choice` pass through |
| Tool calls (translated) | ✅ | — | Claude `tool_use` ↔ Gemini `functionCall` ↔ OpenAI `tool_calls` |
| Token usage tracking (RTK) | ✅ | — | Parse từ upstream response + SSE stream final event |
| Token usage tracking (storage) | ✅ | — | `usageHistory` + `requestDetails` + `usageDaily` + cost |
| `/v1/responses` | ✅ | — | Convert Responses→Chat, proxy, convert back |
| `/v1/messages` | ✅ | — | Claude passthrough hoặc Claude→OpenAI→Claude |
| Usage storage | ✅ | — | `usageHistory` table + `GET /api/usage` |
| Usage daily aggregation | ✅ | — | `usageDaily` table + `aggregate_daily_usage()` |
| Cost calculation | ✅ | — | Static price table cho OpenAI/Claude/Gemini/DeepSeek/Groq/Mistral |
| HTTP/2 multiplexing | ✅ | — | reqwest `http2` feature, ALPN negotiation |
| Combo fallback (non-stream) | ✅ | — | Hoạt động ngay |
| Combo fallback (streaming) | ✅ | — | Check status trước stream, fallback nếu non-2xx |
| Auth (Bearer + x-api-key) | ✅ | — | Hoạt động ngay |
| Connection pooling | ✅ | — | Shared `reqwest::Client` trong `AppState` |
| Tool result messages | ✅ | — | `tool` role → Claude `tool_result` / Gemini `functionResponse` |
| `parallel_tool_calls` | ✅ | — | → Claude `disable_parallel_tool_use` (inverted) |

---

## Timeline

### Phase 1: Performance & Streaming (1-2 weeks)
- Week 1: 🔴 **Streaming SSE** (Hermes P0) + **Tool call passthrough** (Hermes P0) + Connection pooling
- Week 2: Memory optimization + Performance testing + Tool call translation (Claude/Gemini)

### Phase 2: Compatibility & Providers (2-3 weeks)
- Week 1: OpenAI 100% compatibility + **`/v1/responses` + `/v1/messages` proxy** (Hermes P1)
- Week 2: Complete provider translators + Tool call streaming translation
- Week 3: Streaming translation + 9Router compat

### Phase 3: Production Hardening (2-3 weeks)
- Week 1: **Token usage tracking** (Hermes P1) + Monitoring
- Week 2: Authentication + Security
- Week 3: Telegram + Docker optimization

### Phase 4: Advanced Features (3-4 weeks)
- Week 1-2: Load balancing
- Week 3: Caching
- Week 4: Advanced routing

### Hermes Integration Milestone

```
✅ Week 1 hoàn thành = Hermes chạy được (streaming + tool calls + connection pooling)
✅ Week 2 hoàn thành = Hermes chạy ổn định (Responses/Messages API, usage storage)
✅ Week 3-4 hoàn thành = Hermes production-ready (SSE event translation, daily aggregation, cost)
```

**Total: 8-12 weeks**
**✅ Hermes MVP: DONE** — Phase 1.1 + 1.2 + 1.5 + 2.1 + 2.3 + 3.1 đã implement
**✅ Hermes Production-Ready: DONE** — tất cả blockers đã hoàn thành

---

## Success Criteria

### Performance ✅
- [ ] Streaming TTFT < 100ms
- [ ] Non-streaming p99 < 200ms
- [ ] Connection reuse > 90%
- [ ] Memory < 200MB at 1K connections

### Compatibility ✅
- [ ] OpenAI SDK 100% compatible
- [ ] All major providers working
- [ ] 9Router migration path
- [ ] Zero client changes required

### Hermes Agent Integration ✅
- [x] Hermes trỏ vào BlackRouter, chat non-streaming hoạt động
- [x] Hermes streaming tool output hoạt động (zero-copy SSE passthrough)
- [x] Hermes tool calls hoạt động qua OpenAI providers (passthrough)
- [x] Hermes tool calls hoạt động qua Claude/Gemini (translated)
- [x] Hermes `/usage` hiển thị token count (RTK metrics + storage)
- [ ] Hermes subagent parallelism = concurrent connections ổn định
- [x] Combo fallback kích hoạt khi provider chính lỗi (streaming + non-streaming)
- [x] `/v1/responses` endpoint hoạt động (OpenAI Responses API)
- [x] `/v1/messages` endpoint hoạt động (Anthropic Messages API)

### Production ✅
- [ ] 99.9% uptime
- [ ] Full observability
- [ ] Security hardened
- [ ] Documentation complete

---

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) for development setup and guidelines.

## License

MIT License - See [LICENSE](../LICENSE)
