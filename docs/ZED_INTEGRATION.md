# Zed IDE Integration

Huong dan nay cau hinh Zed IDE dung BlackRouter nhu mot OpenAI-compatible
provider. Vi du mac dinh dung model combo `black-mimo`.

## 1. Chay BlackRouter

Chay BlackRouter local:

```bash
BLACKROUTER_PORT=20130 cargo run -p blackrouter-bin
```

Hoac dung port test rieng:

```bash
BLACKROUTER_PORT=20131 cargo run -p blackrouter-bin
```

Kiem tra server:

```bash
curl http://localhost:20130/health
curl http://localhost:20130/v1/models
```

Neu Zed chay tren cung may, API URL se la:

```text
http://localhost:20130/v1
```

Neu dung port `20131`, doi URL thanh:

```text
http://localhost:20131/v1
```

## 2. Tao Provider trong BlackRouter

Mo setup UI:

```text
http://localhost:20130/setup
```

Vao tab `Providers`, tao hoac kiem tra upstream provider cho model Mimo:

- `Provider`: `cline`
- `Name`: `Cline Router`
- `Auth Type`: `api-key`
- `Format`: `openai`
- `Base URL`: `https://api.cline.bot/api/v1/chat/completions`
- `API Key`: token/API key cua Cline
- `Active`: bat

Sau khi save:

1. Bam `Test` de kiem tra endpoint/auth.
2. Bam `Fetch Models` neu muon cap nhat danh sach model.
3. Dam bao model can route co trong provider hoac co the go truc tiep theo id:

```text
cline/cline-pass/mimo-v2.5-pro
```

## 3. Tao Combo `black-mimo`

Trong setup UI, vao tab `Combos` va tao combo:

- `Name`: `black-mimo`
- `Kind`: `llm`
- `Models`:

```text
cline/cline-pass/mimo-v2.5-pro
```

Kiem tra combo da hien tren model list:

```bash
curl http://localhost:20130/v1/models
```

Test nhanh chat completions:

```bash
curl -sS http://localhost:20130/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "black-mimo",
    "messages": [
      { "role": "user", "content": "Reply with exactly: ok" }
    ],
    "max_tokens": 1024,
    "stream": false
  }'
```

## 4. Them Provider trong Zed UI

Theo Zed docs, `LLM Providers` dung cho cac tinh nang AI native cua Zed nhu
Zed Agent, Inline Assistant, Git commit generation, thread summaries. No khong
cau hinh model cho `External Agents` hoac `Terminal Threads`; cac agent/CLI do
thuong tu quan ly model, auth, va config rieng.

Trong Zed:

1. Mo Command Palette.
2. Chay `agent: open settings`.
3. Trong `LLM Providers`, chon `Add Provider`.
4. Chon provider type `OpenAI`.
5. Dien:
   - `Provider Name`: `blackrouter`
   - `API URL`: `http://localhost:20130/v1`
   - `Model ID`: `black-mimo`
   - `Context Window`: `1000000`
6. API key:
   - Neu `BLACKROUTER_REQUIRE_API_KEY=false`, co the dung mot gia tri dummy nhu
     `blackrouter-local` neu Zed bat buoc nhap key.
   - Neu `BLACKROUTER_REQUIRE_API_KEY=true`, tao key trong BlackRouter setup UI
     tab `API Keys`, roi paste vao Zed.

Khong nen luu API key trong `settings.json`. Zed luu key tu UI vao system
keychain; neu muon dung env var, voi provider name `blackrouter`, Zed se doc:

```bash
export BLACKROUTER_API_KEY='your-blackrouter-api-key-or-dummy'
```

Neu co ca key trong keychain va env var, Zed uu tien env var. Sau khi doi hoac
xoa env var, restart Zed de app doc lai moi truong. Voi SSH/dev container/remote
project, LLM Providers van duoc khoi tao tu Zed app local: keychain va env var
la cua may dang chay Zed, khong phai remote shell.

Sau khi them provider, chon model `black-mimo` trong model selector cua Zed
Agent.

## 5. Cau hinh `settings.json`

Mo Zed command `zed: open settings file`, them cau hinh sau. Neu file cua ban
da co cac block `language_models` hoac `agent`, merge noi dung thay vi copy de
ghi de toan bo file.

```json
{
  "language_models": {
    "openai_compatible": {
      "blackrouter": {
        "api_url": "http://localhost:20130/v1",
        "available_models": [
          {
            "name": "black-mimo",
            "display_name": "BlackRouter Mimo",
            "max_tokens": 1000000,
            "max_output_tokens": 250000,
            "max_completion_tokens": 300000,
            "capabilities": {
              "tools": true,
              "images": true,
              "parallel_tool_calls": true,
              "prompt_cache_key": true,
              "chat_completions": true,
              "interleaved_reasoning": true
            }
          }
        ]
      }
    }
  }
}
```

Neu BlackRouter dang chay port `20131`, doi:

```json
"api_url": "http://localhost:20131/v1"
```

Neu can Zed gui tham so max token theo OpenAI-compatible request, co the them
vao `capabilities`:

```json
"max_tokens_parameter": true
```

Chi them field nay khi provider sau BlackRouter chap nhan token-limit parameter.
BlackRouter hien co xu ly `max_tokens`, `max_output_tokens`, va
`max_completion_tokens` cho cac translator lien quan.

Ghi nho cac capability mac dinh cua Zed cho `openai_compatible`:

- `tools`: `true`
- `images`: `false`
- `parallel_tool_calls`: `false`
- `prompt_cache_key`: `false`
- `chat_completions`: `true`
- `interleaved_reasoning`: `false`
- `max_tokens_parameter`: `false`

Neu mot model/route chi hoat dong qua Responses API, dat
`capabilities.chat_completions` thanh `false`. Voi BlackRouter, nen giu
`chat_completions: true` vi BlackRouter expose `/v1/chat/completions` va tu
translate/fallback sang provider phu hop.

## 6. Cau hinh nhieu combo

Co the khai bao them combo BlackRouter khac trong cung provider:

```json
{
  "language_models": {
    "openai_compatible": {
      "blackrouter": {
        "api_url": "http://localhost:20130/v1",
        "available_models": [
          {
            "name": "black-mimo",
            "display_name": "BlackRouter Mimo",
            "max_tokens": 1000000,
            "max_output_tokens": 250000,
            "max_completion_tokens": 300000,
            "capabilities": {
              "tools": true,
              "images": true,
              "parallel_tool_calls": true,
              "prompt_cache_key": true,
              "chat_completions": true,
              "interleaved_reasoning": true
            }
          },
          {
            "name": "black-gpt",
            "display_name": "BlackRouter GPT",
            "max_tokens": 1000000,
            "max_output_tokens": 250000,
            "max_completion_tokens": 300000,
            "capabilities": {
              "tools": true,
              "images": true,
              "parallel_tool_calls": true,
              "prompt_cache_key": true,
              "chat_completions": true,
              "interleaved_reasoning": true
            }
          }
        ]
      }
    }
  }
}
```

Moi `name` phai trung voi model id ma BlackRouter resolve duoc:

- combo name, vi du `black-mimo`
- alias name, neu da tao trong BlackRouter
- direct provider/model, vi du `cline/cline-pass/mimo-v2.5-pro`

## 7. Troubleshooting

### Zed khong thay model

Kiem tra:

```bash
curl http://localhost:20130/v1/models
```

Neu `black-mimo` khong co trong response, tao lai combo trong BlackRouter setup
UI hoac kiem tra database dang dung dung port/data dir.

### `401 Unauthorized`

Neu `BLACKROUTER_REQUIRE_API_KEY=true`, tao API key trong BlackRouter setup UI
va nhap vao Zed Agent Settings. Khong dat key vao `settings.json`.

### `Connection refused`

Kiem tra BlackRouter dang chay va dung port:

```bash
curl http://localhost:20130/health
```

Neu ban dang test bang port `20131`, phai doi `api_url` trong Zed thanh
`http://localhost:20131/v1`.

### Provider upstream loi

Neu Zed goi duoc BlackRouter nhung upstream loi, test truc tiep:

```bash
curl -sS http://localhost:20130/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "black-mimo",
    "messages": [
      { "role": "user", "content": "Say ok" }
    ],
    "max_tokens": 1024
  }'
```

Sau do kiem tra provider trong setup UI: active status, API key, base URL,
format, cooldown, va combo route.

## References

- Zed Use API Access: https://zed.dev/docs/ai/use-api-access
- Zed Agent Settings: https://zed.dev/docs/ai/agent-settings
- Zed AI Quick Start: https://zed.dev/docs/ai/quick-start
