# Provider Login Mechanisms

Tài liệu này mô tả các cơ chế xác thực (authentication) của các providers trong BlackRouter.

## Tổng quan

BlackRouter hỗ trợ 3 loại xác thực chính cho providers:

| Loại | Mô tả | Providers |
|------|--------|-----------|
| `api-key` | Xác thực bằng API key | OpenAI, Anthropic, Gemini, DeepSeek, Groq, xAI, Mistral, Perplexity, Together, Fireworks, NVIDIA, OpenRouter, Cline, Command Code |
| `oauth` | Xác thực qua OAuth 2.0 | GitHub Copilot, Codex (OpenAI), Antigravity (Google), Cursor, Kiro |
| `none` | Không cần xác thực | Ollama Local, OpenCode |

---

## 1. API Key Authentication

Đây là cơ chế đơn giản nhất - người dùng cung cấp API key từ provider.

### Cấu hình

Thêm provider với `auth_type: "api-key"` và điền `api_key`:

```json
{
  "provider": "openai",
  "auth_type": "api-key",
  "data": {
    "api_key": "sk-..."
  }
}
```

### Các providers hỗ trợ

| Provider | API Key Format | Website |
|----------|---------------|---------|
| OpenAI | `sk-...` | https://platform.openai.com |
| Anthropic | `sk-ant-...` | https://console.anthropic.com |
| Gemini | Google AI Studio key | https://ai.google.dev |
| DeepSeek | DeepSeek API key | https://platform.deepseek.com |
| Groq | Groq API key | https://console.groq.com |
| xAI | xAI API key | https://console.x.ai |
| Mistral | Mistral API key | https://console.mistral.ai |
| Perplexity | Perplexity API key | https://www.perplexity.ai/settings/api |
| Together | Together API key | https://api.together.xyz |
| Fireworks | Fireworks API key | https://fireworks.ai |
| NVIDIA NIM | NVIDIA API key | https://build.nvidia.com |
| OpenRouter | OpenRouter API key | https://openrouter.ai |
| Cline | Cline auth token | https://cline.bot |
| Command Code | `user_...` from auth.json | https://commandcode.ai |

---

## 2. OAuth 2.0 Authentication

BlackRouter hỗ trợ 4 OAuth flows khác nhau cho các providers.

### 2.1 GitHub Copilot - Device Code Flow

**Flow:** Device Code (RFC 8628)

**Cách hoạt động:**
1. Gọi `POST /api/oauth/github/start`
2. Nhận `user_code` và `verification_uri`
3. Người dùng mở `https://github.com/login/device` và nhập `user_code`
4. Frontend poll `POST /api/oauth/github/exchange` cho đến khi user xác nhận
5. Nhận GitHub access token
6. BlackRouter tự động đổi lấy Copilot token từ `https://api.github.com/copilot_internal/v2/token`

**Environment Variables:**
```bash
OAUTH_GITHUB_CLIENT_ID=your_github_client_id
```

**API Endpoints:**
```
POST /api/oauth/github/start     → Bắt đầu device flow
POST /api/oauth/github/exchange  → Poll để lấy token
GET  /api/oauth/github/status    → Kiểm tra trạng thái
```

**Response mẫu:**
```json
{
  "url": "https://github.com/login/device",
  "state": "a1b2c3d4",
  "provider": "github",
  "flow_type": "device_code",
  "user_code": "ABCD-1234",
  "verification_uri": "https://github.com/login/device",
  "expires_in": 900,
  "interval": 5
}
```

---

### 2.2 Google/Gemini - Authorization Code Flow

**Flow:** Authorization Code (RFC 6749)

**Cách hoạt động:**
1. Gọi `POST /api/oauth/google/start`
2. Nhận URL authorization
3. Người dùng mở URL và đăng nhập Google
4. Google redirect về `/api/oauth/google/callback` với authorization code
5. BlackRouter tự động đổi code lấy access token

**Environment Variables:**
```bash
OAUTH_GOOGLE_CLIENT_ID=your_google_client_id
OAUTH_GOOGLE_CLIENT_SECRET=your_google_client_secret
```

**API Endpoints:**
```
POST /api/oauth/google/start      → Tạo authorization URL
GET  /api/oauth/google/callback   → OAuth callback (tự động)
POST /api/oauth/google/exchange   → Manual code exchange
GET  /api/oauth/google/status     → Kiểm tra trạng thái
```

**Scopes:** `https://www.googleapis.com/auth/userinfo.email`

---

### 2.3 Antigravity (Google) - Authorization Code Flow với Onboarding

**Flow:** Authorization Code + Cloud Code Assist Onboarding

**Cách hoạt động:**
1. Gọi `POST /api/oauth/antigravity/start`
2. Nhận URL authorization với scopes mở rộng
3. Người dùng mở URL và đăng nhập Google
4. Google redirect về `/api/oauth/antigravity/callback`
5. BlackRouter đổi code lấy access token
6. Tự động gọi `loadCodeAssist` để lấy project ID
7. Tự động gọi `onboardUser` để kích hoạt Gemini Code Assist

**Environment Variables:**
```bash
# Optional - có sẵn credentials mặc định
OAUTH_ANTIGRAVITY_CLIENT_ID=...     # (optional)
OAUTH_ANTIGRAVITY_CLIENT_SECRET=... # (optional)
```

**API Endpoints:**
```
POST /api/oauth/antigravity/start      → Tạo authorization URL
GET  /api/oauth/antigravity/callback   → OAuth callback (tự động)
POST /api/oauth/antigravity/exchange   → Manual code exchange
GET  /api/oauth/antigravity/status     → Kiểm tra trạng thái
```

**Scopes:**
- `https://www.googleapis.com/auth/cloud-platform`
- `https://www.googleapis.com/auth/userinfo.email`
- `https://www.googleapis.com/auth/userinfo.profile`
- `https://www.googleapis.com/auth/cclog`
- `https://www.googleapis.com/auth/experimentsandconfigs`

**Đặc biệt:** Antigravity sử dụng credentials mặc định (hardcoded) giống Antigravity IDE. Người dùng có thể ghi đè bằng environment variables.

---

### 2.4 Codex/OpenAI - Authorization Code Flow với PKCE

**Flow:** Authorization Code + PKCE (RFC 7636)

**Cách hoạt động:**
1. Gọi `POST /api/oauth/codex/start`
2. BlackRouter tạo `code_verifier` và `code_challenge` (PKCE)
3. Nhận URL authorization với PKCE parameters
4. Người dùng mở URL và đăng nhập OpenAI
5. OpenAI redirect về `/api/oauth/codex/callback`
6. BlackRouter đổi code lấy access token (kèm code_verifier)
7. Trích xuất email từ `id_token` (JWT)

**Environment Variables:**
```bash
OAUTH_CODEX_CLIENT_ID=your_codex_client_id
```

**API Endpoints:**
```
POST /api/oauth/codex/start      → Tạo authorization URL (với PKCE)
GET  /api/oauth/codex/callback   → OAuth callback (tự động)
POST /api/oauth/codex/exchange   → Manual code exchange
GET  /api/oauth/codex/status     → Kiểm tra trạng thái
```

**Scopes:** `openid profile email offline_access`

**Đặc biệt:** 
- Sử dụng PKCE để bảo mật mà không cần client secret
- Tự động trích xuất email từ JWT id_token
- Hỗ trợ cả tên `codex` và `openai`

---

## 3. Không xác thực (No Auth)

Một số providers chạy local không cần xác thực.

### Providers

| Provider | Default URL | Mô tả |
|----------|-------------|--------|
| Ollama Local | `http://localhost:11434/v1/chat/completions` | Ollama chạy local |
| OpenCode | `http://localhost:4096/v1/chat/completions` | OpenCode free local |

### Cấu hình

```json
{
  "provider": "ollama-local",
  "auth_type": "none",
  "data": {
    "base_url": "http://localhost:11434/v1/chat/completions"
  }
}
```

---

## 4. OAuth Session Management

BlackRouter lưu trữ OAuth sessions trong memory (HashMap) với các thông tin:

```rust
struct OAuthSession {
    provider: String,
    code_verifier: Option<String>,    // PKCE code verifier
    access_token: Option<String>,     // Access token hoặc device code
    refresh_token: Option<String>,    // Refresh token (nếu có)
    email: Option<String>,            // Email người dùng
    expires_at: Option<String>,       // Thời hạn token
    status: String,                   // "pending", "done", "error"
    error: Option<String>,            // Thông báo lỗi
}
```

### Session Lifecycle

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   pending   │ ──▶ │    done     │     │   error     │
└─────────────┘     └─────────────┘     └─────────────┘
       │                                       ▲
       │                                       │
       └───────────────────────────────────────┘
                    (nếu có lỗi)
```

---

## 5. API Reference

### OAuth Start

```http
POST /api/oauth/{provider}/start
```

**Path Parameters:**
- `provider`: `github`, `google`, `gemini`, `antigravity`, `codex`, `openai`

**Response (200 OK):**
```json
{
  "url": "https://...",
  "state": "session_id",
  "provider": "github",
  "flow_type": "device_code|authorization_code",
  "user_code": "ABCD-1234",           // chỉ device_code
  "verification_uri": "https://...",   // chỉ device_code
  "expires_in": 900,                   // chỉ device_code
  "interval": 5                        // chỉ device_code
}
```

### OAuth Callback

```http
GET /api/oauth/{provider}/callback?code=...&state=...
```

Tự động đổi code lấy token và hiển thị trang thành công.

### OAuth Exchange

```http
POST /api/oauth/{provider}/exchange
Content-Type: application/json

{
  "code": "authorization_code",
  "state": "session_id"
}
```

**Response (200 OK):**
```json
{
  "status": "done",
  "access_token": "ghu_...",
  "refresh_token": "ghr_...",
  "email": "user@example.com",
  "project_id": "my-gcp-project",  // chỉ antigravity
  "error": null
}
```

### OAuth Status

```http
GET /api/oauth/{provider}/status?state=session_id
```

**Response (200 OK):**
```json
{
  "status": "pending|done|error",
  "access_token": "...",
  "refresh_token": "...",
  "email": "user@example.com",
  "error": null
}
```

---

## 6. Cấu hình Environment Variables

Tạo file `.env` từ `.env.example`:

```bash
# GitHub OAuth (Device Code Flow)
OAUTH_GITHUB_CLIENT_ID=

# Google OAuth (Authorization Code Flow)
OAUTH_GOOGLE_CLIENT_ID=
OAUTH_GOOGLE_CLIENT_SECRET=

# Antigravity OAuth (có sẵn credentials mặc định)
OAUTH_ANTIGRAVITY_CLIENT_ID=
OAUTH_ANTIGRAVITY_CLIENT_SECRET=

# Codex/OpenAI OAuth (Authorization Code + PKCE)
OAUTH_CODEX_CLIENT_ID=

# Base URL cho OAuth callbacks
BLACKROUTER_BASE_URL=http://localhost:20130
```

---

## 7. Provider Catalog

Gọi `GET /api/setup/provider-catalog` để xem danh sách tất cả providers:

```json
[
  {
    "id": "openai",
    "alias": "openai",
    "name": "OpenAI",
    "category": "api-key",
    "auth_type": "api-key",
    "format": "openai",
    "base_url": "https://api.openai.com/v1/chat/completions",
    "api_key_hint": "sk-...",
    "website": "https://platform.openai.com"
  }
]
```

### Categories

| Category | Mô tả |
|----------|--------|
| `api-key` | Providers sử dụng API key |
| `coding` | Coding assistants (Command Code, Cline, Antigravity) |
| `free-tier` | Providers có free tier (Gemini, NVIDIA) |
| `subscription` | Providers yêu cầu subscription (GitHub Copilot, Codex, Cursor, Kiro) |
| `local` | Providers chạy local (Ollama, OpenCode) |
