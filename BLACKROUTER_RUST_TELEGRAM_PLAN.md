# Ke hoach chuyen doi 9router-custom thanh BlackRouter Rust + Telegram

_Ngay lap: 2026-07-04_

## 1. Muc tieu

Chuyen doi du an `9router-custom` thanh mot gateway moi ten `BlackRouter`, trong do phan runtime dinh tuyen chinh duoc viet bang Rust, van giu kha nang tuong thich voi 9Router hien tai va bo sung Telegram lam kenh ket noi, giam sat, dieu khien tu xa.

Muc tieu chinh:

- Chuyen core router tu Node/Next.js sang Rust de tang do on dinh, hieu nang streaming, kha nang dong goi binary va van hanh dai han.
- Giu tuong thich API voi cac endpoint dang co: `/v1/chat/completions`, `/v1/responses`, `/v1/messages`, `/v1/models`, `/v1/audio/*`, `/v1/images/*`, `/v1/embeddings`, `/v1/search`, `/v1/web/fetch`.
- Giu tuong thich du lieu voi SQLite schema hien tai trong `src/lib/db/schema.js`.
- Giu dashboard Next.js trong giai do dau, de UI tiep tuc hoat dong khi Rust backend thay the dan cac API quan trong.
- Bo sung Telegram bot de lien ket admin, xem trang thai, quan ly provider/account/combo, bat tat RTK, xem usage/log, va thuc hien cac lenh dieu khien BlackRouter co kiem soat quyen.
- Trien khai migration theo huong chay song song, co rollback, co golden tests cho request/response translation va SSE streaming.

## 2. Hien trang du an

Du an hien tai la ung dung Next.js + Node.js:

- Dashboard va management API nam trong `src/app/*`.
- OpenAI-compatible API nam trong `src/app/api/v1/*` va duoc rewrite tu `/v1/*`.
- Core routing nam trong `src/sse/*` va `open-sse/*`.
- Chat flow chinh di qua `src/sse/handlers/chat.js` va `open-sse/handlers/chatCore.js`.
- Provider executors nam trong `open-sse/executors/*`.
- Translator nam trong `open-sse/translator/*`.
- RTK compression/truncation nam trong `open-sse/rtk/*`.
- Persistence hien tai dung SQLite qua adapter chain trong `src/lib/db/*`, schema chinh trong `src/lib/db/schema.js`.
- Docker hien tai build Next standalone va chay Node tren port mac dinh `20128`.

Nhan xet:

- Nen chuyen runtime router truoc, khong nen rewrite dashboard ngay.
- Nen giu DB schema trong giai do dau de dashboard cu va Rust backend co the dung chung du lieu.
- Nen port tung provider theo muc uu tien thay vi co gang rewrite tat ca executor cung luc.

## 3. Khong nam trong pham vi giai do dau

- Khong rewrite toan bo dashboard React/Next.js sang Rust trong phase 1.
- Khong thay doi public API cua client CLI neu chua co compatibility layer.
- Khong dua secrets provider len Telegram.
- Khong xoa Node implementation cho den khi Rust core dat du coverage va co rollback path.
- Khong doi schema pha vo du lieu cu neu chua co migration va backup tu dong.

## 4. Kien truc muc tieu

```text
Clients
  Claude Code / Codex / Cursor / Cline / OpenAI SDK
        |
        | HTTP / SSE
        v
BlackRouter Rust Runtime
  axum API server
  auth + API key middleware
  model resolver
  combo/account fallback
  translator registry
  provider executors
  RTK compression/truncation
  usage/request logging
  Telegram control plane
        |
        | SQLite + files
        v
Data dir
  blackrouter.db or existing 9router db
  request logs
  audit logs

Dashboard Next.js
  Phase 1: chay song song va goi Rust management API
  Phase 2: duoc serve nhu static UI hoac tiep tuc la optional web UI
```

De xuat Rust workspace:

```text
blackrouter/
  Cargo.toml
  crates/
    blackrouter-bin/       # binary chinh
    blackrouter-api/       # axum routes, middleware, SSE response
    blackrouter-core/      # routing orchestration, fallback, model resolver
    blackrouter-translator/# OpenAI/Claude/Gemini/Responses translators
    blackrouter-providers/ # provider executors
    blackrouter-rtk/       # RTK compression, truncation, token filters
    blackrouter-storage/   # SQLite repos, migrations, backup
    blackrouter-telegram/  # Telegram bot, command dispatcher
    blackrouter-config/    # env/config loader
    blackrouter-common/    # shared errors, types, telemetry
```

Thu vien Rust de xuat:

- HTTP server: `axum`, `tower`, `tower-http`, `hyper`.
- Async runtime: `tokio`.
- HTTP client: `reqwest` voi `rustls-tls`, proxy support.
- SSE/streaming: `tokio-stream`, `async-stream`, `eventsource-stream` neu can parser.
- JSON/schema: `serde`, `serde_json`, `schemars`.
- SQLite: `sqlx` hoac `rusqlite`. Uu tien `sqlx` neu muon async pool, uu tien `rusqlite` neu muon binary gon va hanh vi SQLite truc tiep.
- Config: `figment` hoac `config`, ket hop `dotenvy`.
- Error/log: `thiserror`, `anyhow` o binary boundary, `tracing`, `tracing-subscriber`.
- Telegram: `teloxide`.
- Secret masking: module rieng trong `blackrouter-common`, khong log raw token.

## 5. Cau hinh moi

Giu cac bien moi ro rang, van map duoc voi bien cu:

```env
BLACKROUTER_HOST=0.0.0.0
BLACKROUTER_PORT=20128
BLACKROUTER_DATA_DIR=~/.9router
BLACKROUTER_DATABASE_URL=sqlite://~/.9router/blackrouter.db
BLACKROUTER_COMPAT_9ROUTER_DB=true
BLACKROUTER_REQUIRE_API_KEY=true

TELEGRAM_BOT_TOKEN=
TELEGRAM_ADMIN_IDS=
TELEGRAM_LINK_CODE_TTL_SECONDS=300
TELEGRAM_USE_WEBHOOK=false
TELEGRAM_WEBHOOK_URL=

BLACKROUTER_CONTROL_TOKEN=
BLACKROUTER_LOG_LEVEL=info
```

Compatibility mapping:

- Neu `DATA_DIR` ton tai thi Rust doc nhu `BLACKROUTER_DATA_DIR`.
- Neu `PORT` ton tai thi Rust doc nhu `BLACKROUTER_PORT`.
- Neu `NEXT_PUBLIC_BASE_URL` ton tai thi chi dashboard tiep tuc su dung.

## 6. Telegram control plane

### 6.1. Muc tieu Telegram

Telegram la kenh admin tu xa, khong phai public API cho user cuoi. Bot can ho tro:

- Ket noi admin voi BlackRouter bang flow lien ket co ma mot lan.
- Xem trang thai runtime, provider, model, combo, tunnel, usage.
- Dieu khien an toan: enable/disable provider connection, reload config, bat tat RTK, doi combo active, test provider.
- Gui canh bao: provider loi, account rate limited, token refresh fail, quota gan het, request error spike.
- Audit moi lenh nhay cam.

### 6.2. Link admin

Flow de xuat:

1. Dashboard hoac CLI sinh one-time code:

   ```bash
   blackrouter admin link-code
   ```

2. Admin nhan ma, gui vao bot:

   ```text
   /link BR-123456
   ```

3. Rust backend luu `telegram_chat_id` vao bang setting/kv sau khi hash va verify code.
4. Cac lenh sau chi chap nhan neu `chat_id` nam trong allowlist hoac `TELEGRAM_ADMIN_IDS`.

Quy tac:

- Link code het han sau `TELEGRAM_LINK_CODE_TTL_SECONDS`.
- Moi code chi dung mot lan.
- Bot khong bao gio gui lai raw API key, OAuth token, cookie, refresh token.
- Lenh thay doi cau hinh can confirm inline keyboard neu anh huong den traffic.

### 6.3. Lenh Telegram giai do dau

Lenh thong tin:

```text
/start
/help
/status
/health
/version
/providers
/provider <provider_id>
/models <provider_id>
/combos
/combo <combo_name>
/usage today
/usage 7d
/logs <n>
```

Lenh dieu khien:

```text
/enable provider <provider_id>
/disable provider <provider_id>
/enable connection <connection_id>
/disable connection <connection_id>
/test provider <provider_id>
/test connection <connection_id>
/rtk on
/rtk off
/reload
/shutdown
```

Lenh can confirm:

- `/disable provider <provider_id>`
- `/disable connection <connection_id>`
- `/rtk off`
- `/shutdown`
- Bat ky lenh nao xoa/sua secret.

Lenh nen tri hoan den phase sau:

- Them API key provider qua Telegram.
- Them OAuth token qua Telegram.
- Sua raw JSON config qua Telegram.

Ly do: cac lenh nay co nguy co lo secret va kho validate.

### 6.4. Telegram alert

Alert nen co rate limit va gom nhom:

- Provider bi 401/403 lien tiep.
- Account bi mark unavailable/rate limited.
- Token refresh fail.
- Usage vuot nguong ngay/thang.
- Latency p95 tang dot bien.
- Rust runtime restart hoac crash recovery.

Bang audit de xuat:

```sql
CREATE TABLE IF NOT EXISTS adminAuditLog (
  id TEXT PRIMARY KEY,
  timestamp TEXT NOT NULL,
  actorType TEXT NOT NULL,
  actorId TEXT NOT NULL,
  action TEXT NOT NULL,
  target TEXT,
  status TEXT NOT NULL,
  meta TEXT
);
```

## 7. Lo trinh migration

### Phase 0 - Dong bang hanh vi va tao baseline

Muc tieu: biet chinh xac Node runtime dang lam gi truoc khi port.

Cong viec:

- Lap inventory endpoint trong `src/app/api/v1/*` va management endpoint nao dashboard dang goi.
- Tao golden fixtures cho request/response cac format:
  - OpenAI chat completions.
  - OpenAI Responses API.
  - Claude Messages.
  - Gemini.
  - Kiro/Antigravity neu dang dung format dac thu.
- Tao mock upstream server de test streaming, non-streaming, 401 refresh, 429 fallback, malformed SSE.
- Ghi lai matrix provider executor tu `open-sse/executors/index.js`.
- Ghi lai DB schema hien tai va tao backup script.

Deliverables:

- `docs/blackrouter-endpoint-inventory.md`
- `tests/fixtures/router-golden/*`
- CI job chay golden tests Node implementation.

Dieu kien xong:

- Co bo test bat duoc regression cho translation, fallback, SSE terminal events va usage logging.

### Phase 1 - Scaffold Rust runtime

Muc tieu: co binary Rust chay duoc, expose health/version/config va doc duoc DB hien tai.

Cong viec:

- Tao Rust workspace `blackrouter/`.
- Tao binary `blackrouter-bin`.
- Implement `GET /health`, `GET /version`, `GET /api/runtime/status`.
- Implement config loader doc env moi va env compatibility.
- Implement SQLite storage layer doc bang:
  - `settings`
  - `providerConnections`
  - `providerNodes`
  - `apiKeys`
  - `combos`
  - `kv`
  - `usageHistory`
  - `requestDetails`
- Implement secret masking va structured logging.
- Them Docker build stage Rust nhung chua thay Docker default.

Deliverables:

- Binary Rust chay tren port rieng, vi du `20129`.
- Storage read-only compatibility voi DB 9Router.
- Health/status API duoc dashboard hoac curl kiem tra.

Dieu kien xong:

- `cargo test` pass.
- Rust runtime doc duoc DB hien tai ma khong migration pha vo.

### Phase 2 - API compatibility shell

Muc tieu: Rust nhan request OpenAI-compatible va tra response dung shape, truoc khi port het provider.

Cong viec:

- Implement middleware:
  - CORS.
  - request id.
  - API key auth tu bang `apiKeys`.
  - timeout va body size limit.
- Implement endpoint shell:
  - `POST /v1/chat/completions`
  - `POST /v1/responses`
  - `POST /v1/messages`
  - `GET /v1/models`
  - `GET /v1beta/models`
- Implement error response shape tuong thich Node runtime.
- Implement SSE response writer va cancellation khi client disconnect.
- Implement request detail logging vao SQLite.

Deliverables:

- Rust endpoint nhan request va co the route den mock executor.
- Golden tests xac nhan status code, header, SSE framing.

Dieu kien xong:

- Cac client OpenAI-compatible co the ket noi den Rust runtime voi mock provider.

### Phase 3 - Port core routing

Muc tieu: dua logic dinh tuyen quan trong sang Rust.

Cong viec:

- Port model parser/resolver:
  - `provider/model`
  - aliases
  - combo names
  - compatible node models
- Port account selection va fallback:
  - active account.
  - priority.
  - cooldown.
  - retry-after.
  - fallback theo status/error.
- Port combo fallback:
  - fallback sequence.
  - round-robin/sticky neu dang dung trong settings.
- Port usage tracking:
  - prompt/completion tokens.
  - cost.
  - status.
  - connection id.
- Port bypass logic:
  - warmup/naming requests.
  - client detector.
- Port token refresh interface, truoc tien dat abstraction de provider tu implement.

Deliverables:

- `blackrouter-core` co unit tests cho routing/fallback.
- Integration test voi mock upstream cho success, 401 refresh, 429 fallback, all unavailable.

Dieu kien xong:

- Rust core cho ket qua routing giong Node tren fixture.

### Phase 4 - Port translators va RTK

Muc tieu: giu chat quality va token saving khi chuyen sang Rust.

Cong viec:

- Port translator registry:
  - OpenAI -> OpenAI.
  - OpenAI -> Claude.
  - Claude -> OpenAI.
  - OpenAI Responses -> provider target.
  - Gemini/Gemini CLI cac case dang co traffic.
- Port stream transformer:
  - provider SSE -> client SSE.
  - provider JSON -> client JSON.
  - forced SSE-to-JSON.
  - terminal events va abort handling.
- Port RTK:
  - compression tool_result.
  - truncation sliding/smart neu dang bat.
  - caveman prompt neu can giu compatibility.
  - log stats token saving.
- Port tool dedupe cho Claude clients neu can.

Deliverables:

- `blackrouter-translator` va `blackrouter-rtk`.
- Golden tests so sanh translated request va stream output voi Node.

Dieu kien xong:

- Request tu Claude Code/Codex/OpenAI SDK qua Rust tao output tuong thich tren mock provider.

### Phase 5 - Port provider executors theo muc uu tien

Muc tieu: dua traffic thuc te qua Rust theo tung provider.

Thu tu uu tien de xuat:

1. Default OpenAI-compatible executor.
2. OpenAI, OpenRouter, Anthropic-compatible.
3. Gemini/Gemini CLI.
4. Codex.
5. GitHub Copilot.
6. Kiro.
7. Cursor/Antigravity/Qoder/iFlow/Qwen.
8. Local/Ollama va provider dac thu con lai.

Moi executor can co:

- Request builder.
- Auth header builder.
- Proxy support.
- Streaming parser.
- Non-streaming parser.
- Error parser.
- Token refresh hook neu provider can.
- Provider-specific tests voi mock server.

Deliverables:

- Provider executor Rust theo tung PR nho.
- Feature flag route traffic provider sang Rust hoac Node.

Dieu kien xong:

- It nhat 3 provider quan trong chay production-like qua Rust truoc khi cutover.

### Phase 6 - Chay song song Node va Rust

Muc tieu: cutover an toan, co rollback nhanh.

Phuong an A: Next.js dung lam front proxy tam thoi.

- Dashboard va management API van chay Node.
- `/v1/*` co the forward sang Rust neu `BLACKROUTER_RUST_API=true`.
- Neu Rust loi, co flag rollback ve Node.

Phuong an B: Rust dung lam main gateway.

- Rust listen `20128`.
- Rust serve `/v1/*` va `/api/runtime/*`.
- Rust reverse proxy cac route dashboard/API chua port sang Next.js tren internal port.

Khuyen nghi:

- Bat dau voi phuong an A de giam rui ro.
- Chuyen sang phuong an B khi Rust da giu du API quan trong.

Deliverables:

- Proxy integration.
- Runtime flag.
- Smoke test script:

  ```bash
  npm run build
  cargo test --workspace
  blackrouter smoke --base-url http://localhost:20128
  ```

Dieu kien xong:

- Co the bat/tat Rust routing ma khong doi config client.

### Phase 7 - Telegram bot

Muc tieu: dieu khien BlackRouter tu Telegram mot cach an toan.

Cong viec:

- Tao crate `blackrouter-telegram`.
- Implement long polling truoc, webhook sau.
- Implement auth:
  - `TELEGRAM_ADMIN_IDS`.
  - one-time link code.
  - audit log.
- Implement command dispatcher va inline confirmation.
- Implement command read-only truoc:
  - `/status`
  - `/health`
  - `/providers`
  - `/usage today`
  - `/logs 20`
- Implement command dieu khien sau:
  - enable/disable provider/connection.
  - test provider/connection.
  - rtk on/off.
  - reload.
- Implement alert worker.
- Them dashboard setting de bat/tat Telegram va tao link code.

Deliverables:

- Telegram bot chay trong cung binary Rust.
- Admin co the link bot va xem/dieu khien runtime.
- Audit log luu moi command thay doi state.

Dieu kien xong:

- Bot khong chap nhan lenh tu chat id chua duoc authorize.
- Bot khong lam lo secrets trong message/log.

### Phase 8 - Docker, release va packaging

Muc tieu: dong goi BlackRouter de deploy local/Docker de hon ban Node-only.

Cong viec:

- Multi-stage Docker:
  - build Rust binary.
  - build Next dashboard neu van can.
  - runtime image nho, copy binary + static/dashboard.
- Support volume `/app/data`.
- Expose port `20128`.
- Them healthcheck.
- Them command:

  ```bash
  blackrouter serve
  blackrouter migrate
  blackrouter backup
  blackrouter admin link-code
  blackrouter smoke
  ```

- Neu can npm wrapper, package chi cai binary va dashboard asset.

Deliverables:

- `Dockerfile.blackrouter`.
- Release artifact cho macOS/Linux/Windows neu can.
- Migration guide tu 9Router sang BlackRouter.

Dieu kien xong:

- Nguoi dung hien tai co the backup data, start BlackRouter, giu endpoint cu `http://localhost:20128/v1`.

### Phase 9 - Cutover va don dep

Muc tieu: Rust la runtime chinh.

Cong viec:

- Doi default Docker/start script sang Rust runtime.
- Giu Node dashboard nhu optional UI neu chua rewrite.
- Dong bang Node router core, chi sua bug critical.
- Cap nhat docs:
  - architecture.
  - env vars.
  - Telegram setup.
  - migration.
  - troubleshooting.
- Lap ke hoach xoa dan code Node core sau 2-3 release on dinh.

Dieu kien xong:

- BlackRouter Rust xu ly traffic mac dinh.
- Node core khong con nam tren request path chinh.

## 8. Chien luoc DB va migration du lieu

Giai do dau nen dung chung schema SQLite hien tai:

- `settings.data` giu JSON settings.
- `providerConnections.data` giu credentials/provider config.
- `providerNodes.data` giu compatible nodes.
- `apiKeys` dung cho auth.
- `combos.models` dung cho combo fallback.
- `usageHistory` va `requestDetails` tiep tuc ghi usage/log.

Nguyen tac:

- Truoc moi migration destructive phai tao backup.
- Rust chi auto-add bang/cot moi, khong drop/rename trong startup.
- Bang moi cho BlackRouter nen co prefix ro rang hoac nam trong migration version moi:
  - `adminAuditLog`
  - `telegramLinks`
  - `runtimeEvents`
- Neu doi tu `db.json` cu sang SQLite da xong trong repo hien tai, BlackRouter chi can support SQLite; neu van con user dung JSON cu, can migration CLI rieng.

## 9. Bao mat

Yeu cau bat buoc:

- Mask secrets trong log, Telegram, error response.
- API key auth khong duoc bi bypass khi `requireApiKey=true`.
- Telegram command chi chay voi allowlisted chat id.
- Lenh nhay cam can confirm va ghi audit.
- Rate limit Telegram commands.
- Khong gui raw provider token/API key qua Telegram.
- Backup DB truoc migration.
- Phan biet control token noi bo voi API key client.
- Co kill switch:

  ```env
  TELEGRAM_ENABLED=false
  BLACKROUTER_CONTROL_API_ENABLED=false
  ```

## 10. Testing

Can co cac lop test:

- Unit tests:
  - model resolver.
  - combo fallback.
  - account selection.
  - error classifier.
  - translator.
  - RTK filters.
  - Telegram command parser.
- Golden tests:
  - input request -> translated upstream request.
  - upstream stream -> client stream.
  - error response compatibility.
- Integration tests:
  - mock upstream SSE.
  - mock 401 refresh.
  - mock 429 fallback.
  - SQLite read/write.
  - Telegram unauthorized/authorized command flow.
- Smoke tests:
  - start server.
  - `GET /health`.
  - `GET /v1/models`.
  - `POST /v1/chat/completions` streaming.
  - dashboard can load.

Acceptance criteria truoc cutover:

- Golden tests cua Node va Rust khop voi cac provider/format uu tien.
- Khong mat usage logging.
- Client hien tai khong can doi endpoint/base URL.
- Rollback Node path van ton tai trong it nhat mot release.

## 11. Rui ro va cach giam rui ro

Rui ro: SSE streaming bi sai format.

- Giam rui ro bang golden tests byte-level cho stream events va terminal events.

Rui ro: Provider dac thu co header/token refresh phuc tap.

- Giam rui ro bang cach port default-compatible providers truoc, provider dac thu sau.

Rui ro: Dashboard Next.js phu thuoc management API Node.

- Giam rui ro bang chay song song, chi forward `/v1/*` sang Rust luc dau.

Rui ro: Lo secret qua Telegram.

- Giam rui ro bang secret masking, command allowlist, khong support nhap raw token qua Telegram trong phase dau.

Rui ro: DB migration pha du lieu.

- Giam rui ro bang read-only compatibility truoc, backup bat buoc, migration chi add bang/cot.

Rui ro: Rust rewrite qua lon.

- Giam rui ro bang chia crate/module va cutover theo provider/endpoint.

## 12. Thu tu uu tien thuc thi ngan han

1. Tao endpoint inventory va golden fixtures.
2. Scaffold Rust workspace + health/status + config.
3. Doc SQLite schema hien tai tu Rust.
4. Implement `/v1/models` va mock `/v1/chat/completions`.
5. Port model resolver + API key auth.
6. Port default OpenAI-compatible executor.
7. Port SSE transformer va RTK toi thieu.
8. Them flag forward `/v1/*` tu Next sang Rust.
9. Them Telegram read-only commands.
10. Them Telegram control commands co confirm.

## 13. Dinh nghia "done"

BlackRouter duoc xem la hoan thanh giai do migration khi:

- Rust binary xu ly mac dinh `/v1/chat/completions`, `/v1/responses`, `/v1/messages`, `/v1/models`.
- It nhat cac provider uu tien cua du an chay qua Rust voi tests.
- Dashboard van quan ly duoc providers, keys, combos, usage.
- Telegram bot link duoc admin va dieu khien duoc provider/connection/RTK.
- Docker image chay duoc tren port `20128` voi data volume.
- Co migration/rollback guide.
- Secrets khong xuat hien trong logs, Telegram messages, request details.
- Co CI cho Rust tests, Node compatibility tests va smoke tests.
