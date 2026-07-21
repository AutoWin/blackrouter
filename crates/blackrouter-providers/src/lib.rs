use serde::Serialize;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum ProviderCapability {
    ChatCompletions,
    Responses,
    Messages,
    Embeddings,
    Images,
    Audio,
    Search,
    WebFetch,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ProviderError {
    #[error("provider not implemented: {0}")]
    NotImplemented(String),
    #[error("provider authentication failed: {0}")]
    Authentication(String),
    #[error("provider unavailable: {0}")]
    Unavailable(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderProfile {
    pub id: &'static str,
    pub alias: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub auth_type: &'static str,
    pub format: &'static str,
    pub base_url: &'static str,
    pub api_key_hint: &'static str,
    pub website: &'static str,
    #[serde(skip_serializing)]
    pub models_url: &'static str,
    #[serde(skip_serializing)]
    pub default_headers: &'static [(&'static str, &'static str)],
    #[serde(skip_serializing)]
    pub fallback_models: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinProviderModels {
    pub label: &'static str,
    pub source: &'static str,
    pub models: &'static [&'static str],
}

pub const GEMINI_CLI_MODELS: &[&str] = &[
    "gemini-2.0-flash-lite",
    "gemini-2.0-flash",
    "gemini-2.5-flash",
    "gemini-1.5-flash",
    "gemini-1.5-pro",
    "gemini-2.5-pro",
    "gemini-3-flash-preview",
    "gemini-3-pro-preview",
];

pub const ANTIGRAVITY_MODELS: &[&str] = &[
    // Antigravity agent models
    "gemini-3-flash-agent",
    "gemini-3.5-flash-low",
    "gemini-3.5-flash-extra-low",
    "gemini-pro-agent",
    "gemini-3.1-pro-low",
    "claude-sonnet-4-6",
    "claude-opus-4-6-thinking",
    "gpt-oss-120b-medium",
    "gemini-3-flash",
    // Gemini CLI models, backed by the same Google OAuth connection.
    "gemini-2.0-flash-lite",
    "gemini-2.0-flash",
    "gemini-2.5-flash",
    "gemini-1.5-flash",
    "gemini-1.5-pro",
    "gemini-2.5-pro",
    "gemini-3-flash-preview",
    "gemini-3-pro-preview",
];

const CLINE_MODELS: &[&str] = &[
    "cline-pass/qwen3.7-max",
    "cline-pass/qwen3.7-plus",
    "cline-pass/minimax-m3",
    "cline-pass/mimo-v2.5-pro",
    "cline-pass/glm-5.2",
    "cline-pass/mimo-v2.5",
    "cline-pass/kimi-k2.7-code",
    "cline-pass/kimi-k3",
    "cline-pass/deepseek-v4-flash",
    "cline-pass/deepseek-v4-pro",
    "cline-pass/kimi-k2.6",
    "stepfun/step-3.7-flash",
    "deepseek/deepseek-v4-flash",
    "zai/glm-5.2",
    "moonshotai/kimi-k2.7-code",
    "moonshotai/kimi-k3",
    "anthropic/claude-opus-4.8",
    "anthropic/claude-sonnet-4.6",
    "openai/gpt-5.5",
];

const COMMANDCODE_MODELS: &[&str] = &[
    // Anthropic
    "claude-sonnet-5",
    "claude-sonnet-4-6",
    "claude-fable-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-haiku-4-5",
    // OpenAI
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.3-codex",
    "gpt-5.4-mini",
    // Moonshot
    "moonshotai/Kimi-K2.7-Code",
    "moonshotai/Kimi-K2.7-Code-Highspeed",
    "moonshotai/Kimi-K3",
    "moonshotai/Kimi-K2.6",
    "moonshotai/Kimi-K2.5",
    // Zhipu
    "zai-org/GLM-5.2",
    "zai-org/GLM-5.2-Fast",
    "zai-org/GLM-5.1",
    "zai-org/GLM-5",
    // MiniMax
    "MiniMaxAI/MiniMax-M3",
    "MiniMaxAI/MiniMax-M2.7",
    "MiniMaxAI/MiniMax-M2.5",
    // Xiaomi
    "xiaomi/mimo-v2.5-pro",
    "xiaomi/mimo-v2.5",
    // DeepSeek
    "deepseek/deepseek-v4-pro",
    "deepseek/deepseek-v4-flash",
    // Alibaba
    "Qwen/Qwen3.7-Max",
    "Qwen/Qwen3.7-Plus",
    "Qwen/Qwen3.6-Max-Preview",
    "Qwen/Qwen3.6-Plus",
    // StepFun
    "stepfun/Step-3.7-Flash",
    "stepfun/Step-3.5-Flash",
    // Tencent
    "tencent/Hy3",
    // NVIDIA
    "nvidia/nemotron-3-ultra-550b-a55b",
    // Google
    "google/gemini-3.5-flash",
    "google/gemini-3.1-flash-lite",
    // Sakana
    "sakana/fugu-ultra",
];

const CODEX_MODELS: &[&str] = &[
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.3-codex",
    "gpt-5.3-codex-review",
    "gpt-5.3-codex-xhigh",
    "gpt-5.3-codex-high",
    "gpt-5.3-codex-low",
    "gpt-5.3-codex-none",
    "gpt-5.3-codex-spark",
    "gpt-5.2-codex",
    "gpt-5.2-codex-review",
    "gpt-5.1-codex-max",
    "gpt-5.1-codex",
    "gpt-5.1-codex-mini",
    "gpt-5-codex",
    "gpt-5-codex-mini",
];

// ── Model Context Window Catalog ──────────────────────────────────────

/// Token limit metadata for a single model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    /// Maximum total tokens (input + output) the model supports.
    pub context_window: u32,
    /// Maximum output tokens the model can generate in one response.
    pub max_output_tokens: u32,
}

/// Static catalog mapping lowercase model names to their token limits.
/// Both hyphenated and dotted version separators are listed where the
/// model appears in multiple formats across the codebase.
const MODEL_CATALOG: &[(&str, ModelInfo)] = &[
    // ── Anthropic Claude ──────────────────────────────────────────────
    (
        "claude-sonnet-5",
        ModelInfo {
            context_window: 200_000,
            max_output_tokens: 16_384,
        },
    ),
    (
        "claude-sonnet-4-6",
        ModelInfo {
            context_window: 200_000,
            max_output_tokens: 8_192,
        },
    ),
    (
        "claude-sonnet-4.6",
        ModelInfo {
            context_window: 200_000,
            max_output_tokens: 8_192,
        },
    ),
    (
        "claude-opus-4-8",
        ModelInfo {
            context_window: 200_000,
            max_output_tokens: 32_768,
        },
    ),
    (
        "claude-opus-4.8",
        ModelInfo {
            context_window: 200_000,
            max_output_tokens: 32_768,
        },
    ),
    (
        "claude-opus-4-7",
        ModelInfo {
            context_window: 200_000,
            max_output_tokens: 32_768,
        },
    ),
    (
        "claude-opus-4.7",
        ModelInfo {
            context_window: 200_000,
            max_output_tokens: 32_768,
        },
    ),
    (
        "claude-opus-4-6-thinking",
        ModelInfo {
            context_window: 200_000,
            max_output_tokens: 64_000,
        },
    ),
    (
        "claude-fable-5",
        ModelInfo {
            context_window: 200_000,
            max_output_tokens: 16_384,
        },
    ),
    (
        "claude-haiku-4-5",
        ModelInfo {
            context_window: 200_000,
            max_output_tokens: 8_192,
        },
    ),
    (
        "claude-haiku-4.5",
        ModelInfo {
            context_window: 200_000,
            max_output_tokens: 8_192,
        },
    ),
    // ── OpenAI GPT ────────────────────────────────────────────────────
    (
        "gpt-5.5",
        ModelInfo {
            context_window: 1_048_576,
            max_output_tokens: 32_768,
        },
    ),
    (
        "gpt-5.4",
        ModelInfo {
            context_window: 1_048_576,
            max_output_tokens: 32_768,
        },
    ),
    (
        "gpt-5.4-mini",
        ModelInfo {
            context_window: 1_048_576,
            max_output_tokens: 16_384,
        },
    ),
    // ── OpenAI Codex ──────────────────────────────────────────────────
    (
        "gpt-5.3-codex",
        ModelInfo {
            context_window: 1_048_576,
            max_output_tokens: 65_536,
        },
    ),
    (
        "gpt-5.3-codex-review",
        ModelInfo {
            context_window: 1_048_576,
            max_output_tokens: 32_768,
        },
    ),
    (
        "gpt-5.3-codex-xhigh",
        ModelInfo {
            context_window: 1_048_576,
            max_output_tokens: 65_536,
        },
    ),
    (
        "gpt-5.3-codex-high",
        ModelInfo {
            context_window: 1_048_576,
            max_output_tokens: 65_536,
        },
    ),
    (
        "gpt-5.3-codex-low",
        ModelInfo {
            context_window: 524_288,
            max_output_tokens: 32_768,
        },
    ),
    (
        "gpt-5.3-codex-none",
        ModelInfo {
            context_window: 262_144,
            max_output_tokens: 16_384,
        },
    ),
    (
        "gpt-5.3-codex-spark",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    (
        "gpt-5.2-codex",
        ModelInfo {
            context_window: 1_048_576,
            max_output_tokens: 65_536,
        },
    ),
    (
        "gpt-5.2-codex-review",
        ModelInfo {
            context_window: 1_048_576,
            max_output_tokens: 32_768,
        },
    ),
    (
        "gpt-5.1-codex-max",
        ModelInfo {
            context_window: 524_288,
            max_output_tokens: 32_768,
        },
    ),
    (
        "gpt-5.1-codex",
        ModelInfo {
            context_window: 524_288,
            max_output_tokens: 32_768,
        },
    ),
    (
        "gpt-5.1-codex-mini",
        ModelInfo {
            context_window: 262_144,
            max_output_tokens: 16_384,
        },
    ),
    (
        "gpt-5-codex",
        ModelInfo {
            context_window: 262_144,
            max_output_tokens: 32_768,
        },
    ),
    (
        "gpt-5-codex-mini",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 16_384,
        },
    ),
    (
        "gpt-oss-120b-medium",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 16_384,
        },
    ),
    // ── Google Gemini ─────────────────────────────────────────────────
    (
        "gemini-1.5-flash",
        ModelInfo {
            context_window: 1_048_576,
            max_output_tokens: 8_192,
        },
    ),
    (
        "gemini-1.5-pro",
        ModelInfo {
            context_window: 2_097_152,
            max_output_tokens: 8_192,
        },
    ),
    (
        "gemini-2.0-flash",
        ModelInfo {
            context_window: 1_048_576,
            max_output_tokens: 8_192,
        },
    ),
    (
        "gemini-2.0-flash-lite",
        ModelInfo {
            context_window: 1_048_576,
            max_output_tokens: 8_192,
        },
    ),
    (
        "gemini-2.5-flash",
        ModelInfo {
            context_window: 1_048_576,
            max_output_tokens: 65_536,
        },
    ),
    (
        "gemini-2.5-pro",
        ModelInfo {
            context_window: 1_048_576,
            max_output_tokens: 65_536,
        },
    ),
    (
        "gemini-3-flash",
        ModelInfo {
            context_window: 2_097_152,
            max_output_tokens: 65_536,
        },
    ),
    (
        "gemini-3-flash-preview",
        ModelInfo {
            context_window: 2_097_152,
            max_output_tokens: 65_536,
        },
    ),
    (
        "gemini-3-flash-agent",
        ModelInfo {
            context_window: 2_097_152,
            max_output_tokens: 65_536,
        },
    ),
    (
        "gemini-3-pro-preview",
        ModelInfo {
            context_window: 2_097_152,
            max_output_tokens: 65_536,
        },
    ),
    (
        "gemini-3.5-flash",
        ModelInfo {
            context_window: 2_097_152,
            max_output_tokens: 65_536,
        },
    ),
    (
        "gemini-3.5-flash-low",
        ModelInfo {
            context_window: 2_097_152,
            max_output_tokens: 32_768,
        },
    ),
    (
        "gemini-3.5-flash-extra-low",
        ModelInfo {
            context_window: 2_097_152,
            max_output_tokens: 16_384,
        },
    ),
    (
        "gemini-3.1-pro-low",
        ModelInfo {
            context_window: 2_097_152,
            max_output_tokens: 32_768,
        },
    ),
    (
        "gemini-3.1-flash-lite",
        ModelInfo {
            context_window: 2_097_152,
            max_output_tokens: 16_384,
        },
    ),
    (
        "gemini-pro-agent",
        ModelInfo {
            context_window: 2_097_152,
            max_output_tokens: 65_536,
        },
    ),
    // ── DeepSeek ──────────────────────────────────────────────────────
    (
        "deepseek-v4-pro",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 16_384,
        },
    ),
    (
        "deepseek-v4-flash",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    // ── Alibaba Qwen ─────────────────────────────────────────────────
    (
        "qwen3.7-max",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 16_384,
        },
    ),
    (
        "qwen3.7-plus",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    (
        "qwen3.6-max-preview",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 16_384,
        },
    ),
    (
        "qwen3.6-plus",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    // ── Moonshot Kimi ─────────────────────────────────────────────────
    (
        "kimi-k2.7-code",
        ModelInfo {
            context_window: 262_144,
            max_output_tokens: 16_384,
        },
    ),
    (
        "kimi-k2.7-code-highspeed",
        ModelInfo {
            context_window: 262_144,
            max_output_tokens: 16_384,
        },
    ),
    (
        "kimi-k3",
        ModelInfo {
            context_window: 262_144,
            max_output_tokens: 16_384,
        },
    ),
    (
        "kimi-k2.6",
        ModelInfo {
            context_window: 262_144,
            max_output_tokens: 16_384,
        },
    ),
    (
        "kimi-k2.5",
        ModelInfo {
            context_window: 262_144,
            max_output_tokens: 16_384,
        },
    ),
    // ── Zhipu GLM ─────────────────────────────────────────────────────
    (
        "glm-5.2",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    (
        "glm-5.2-fast",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    (
        "glm-5.1",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    (
        "glm-5",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    // ── MiniMax ───────────────────────────────────────────────────────
    (
        "minimax-m3",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 16_384,
        },
    ),
    (
        "minimax-m2.7",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    (
        "minimax-m2.5",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    // ── Xiaomi mimo ───────────────────────────────────────────────────
    (
        "mimo-v2.5-pro",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    (
        "mimo-v2.5",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    // ── StepFun ───────────────────────────────────────────────────────
    (
        "step-3.7-flash",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    (
        "step-3.5-flash",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    // ── Tencent ───────────────────────────────────────────────────────
    (
        "hy3",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
    // ── NVIDIA ────────────────────────────────────────────────────────
    (
        "nemotron-3-ultra-550b-a55b",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 16_384,
        },
    ),
    // ── Sakana ────────────────────────────────────────────────────────
    (
        "fugu-ultra",
        ModelInfo {
            context_window: 131_072,
            max_output_tokens: 8_192,
        },
    ),
];

/// Look up model info by model name. Handles `provider/model` format by
/// stripping the prefix. Comparison is case-insensitive.
pub fn lookup_model_info(model: &str) -> Option<&'static ModelInfo> {
    let name = match model.rsplit_once('/') {
        Some((_, m)) => m,
        None => model,
    };
    MODEL_CATALOG
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, info)| info)
}

pub const PROVIDER_PROFILES: &[ProviderProfile] = &[
    ProviderProfile {
        id: "commandcode",
        alias: "cmc",
        name: "Command Code",
        category: "coding",
        auth_type: "api-key",
        format: "commandcode",
        base_url: "https://api.commandcode.ai/alpha/generate",
        api_key_hint: "user_... from ~/.commandcode/auth.json or commandcode.ai/studio",
        website: "https://commandcode.ai",
        models_url: "",
        default_headers: &[
            ("x-command-code-version", "0.25.7"),
            ("x-cli-environment", "cli"),
        ],
        fallback_models: COMMANDCODE_MODELS,
    },
    ProviderProfile {
        id: "tencent",
        alias: "tencent",
        name: "Tencent",
        category: "coding",
        auth_type: "api-key",
        format: "commandcode",
        base_url: "https://api.commandcode.ai/alpha/generate",
        api_key_hint: "user_... from ~/.commandcode/auth.json or commandcode.ai/studio",
        website: "https://commandcode.ai",
        models_url: "",
        default_headers: &[
            ("x-command-code-version", "0.25.7"),
            ("x-cli-environment", "cli"),
        ],
        fallback_models: &["tencent/Hy3"],
    },
    ProviderProfile {
        id: "cline",
        alias: "cl",
        name: "Cline Router",
        category: "coding",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.cline.bot/api/v1/chat/completions",
        api_key_hint: "Cline auth token or API key",
        website: "https://cline.bot",
        models_url: "",
        default_headers: &[],
        fallback_models: CLINE_MODELS,
    },
    ProviderProfile {
        id: "antigravity",
        alias: "ag",
        name: "Google Antigravity",
        category: "coding",
        auth_type: "oauth",
        format: "antigravity",
        base_url: "https://cloudcode-pa.googleapis.com",
        api_key_hint:
            "Google OAuth token (set OAUTH_ANTIGRAVITY_CLIENT_ID / OAUTH_ANTIGRAVITY_CLIENT_SECRET)",
        website: "https://antigravity.google",
        models_url: "",
        default_headers: &[],
        fallback_models: ANTIGRAVITY_MODELS,
    },
    ProviderProfile {
        id: "openrouter",
        alias: "openrouter",
        name: "OpenRouter",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://openrouter.ai/api/v1/chat/completions",
        api_key_hint: "OpenRouter API key",
        website: "https://openrouter.ai",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "openai",
        alias: "openai",
        name: "OpenAI",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.openai.com/v1/chat/completions",
        api_key_hint: "sk-...",
        website: "https://platform.openai.com",
        models_url: "",
        default_headers: &[],
        fallback_models: CODEX_MODELS,
    },
    ProviderProfile {
        id: "anthropic",
        alias: "anthropic",
        name: "Anthropic",
        category: "api-key",
        auth_type: "api-key",
        format: "claude",
        base_url: "https://api.anthropic.com/v1/messages",
        api_key_hint: "sk-ant-...",
        website: "https://console.anthropic.com",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "gemini",
        alias: "gemini",
        name: "Gemini",
        category: "free-tier",
        auth_type: "api-key",
        format: "gemini",
        base_url: "https://generativelanguage.googleapis.com/v1beta/models",
        api_key_hint: "Google AI Studio API key",
        website: "https://ai.google.dev",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "deepseek",
        alias: "ds",
        name: "DeepSeek",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.deepseek.com/chat/completions",
        api_key_hint: "DeepSeek API key",
        website: "https://platform.deepseek.com",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "groq",
        alias: "groq",
        name: "Groq",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.groq.com/openai/v1/chat/completions",
        api_key_hint: "Groq API key",
        website: "https://console.groq.com",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "xai",
        alias: "xai",
        name: "xAI",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.x.ai/v1/chat/completions",
        api_key_hint: "xAI API key",
        website: "https://console.x.ai",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "mistral",
        alias: "mistral",
        name: "Mistral",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.mistral.ai/v1/chat/completions",
        api_key_hint: "Mistral API key",
        website: "https://console.mistral.ai",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "perplexity",
        alias: "pplx",
        name: "Perplexity",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.perplexity.ai/chat/completions",
        api_key_hint: "Perplexity API key",
        website: "https://www.perplexity.ai/settings/api",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "together",
        alias: "together",
        name: "Together AI",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.together.xyz/v1/chat/completions",
        api_key_hint: "Together API key",
        website: "https://api.together.xyz",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "fireworks",
        alias: "fireworks",
        name: "Fireworks",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.fireworks.ai/inference/v1/chat/completions",
        api_key_hint: "Fireworks API key",
        website: "https://fireworks.ai",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "nvidia",
        alias: "nvidia",
        name: "NVIDIA NIM",
        category: "free-tier",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://integrate.api.nvidia.com/v1/chat/completions",
        api_key_hint: "NVIDIA API key",
        website: "https://build.nvidia.com",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "github",
        alias: "gh",
        name: "GitHub Copilot",
        category: "subscription",
        auth_type: "oauth",
        format: "openai",
        base_url: "https://api.githubcopilot.com/chat/completions",
        api_key_hint: "OAuth access token",
        website: "https://github.com/features/copilot",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "codex",
        alias: "cx",
        name: "Codex",
        category: "subscription",
        auth_type: "oauth",
        format: "openai-responses",
        base_url: "https://chatgpt.com/backend-api/codex/responses",
        api_key_hint: "OAuth access token",
        website: "https://chatgpt.com",
        models_url: "",
        default_headers: &[],
        fallback_models: CODEX_MODELS,
    },
    ProviderProfile {
        id: "cursor",
        alias: "cu",
        name: "Cursor",
        category: "subscription",
        auth_type: "oauth",
        format: "cursor",
        base_url: "https://api2.cursor.sh",
        api_key_hint: "Cursor session token",
        website: "https://cursor.com",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "kiro",
        alias: "kr",
        name: "Kiro",
        category: "subscription",
        auth_type: "oauth",
        format: "kiro",
        base_url: "https://codewhisperer.us-east-1.amazonaws.com/generateAssistantResponse",
        api_key_hint: "Kiro OAuth token",
        website: "https://kiro.dev",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "opencode",
        alias: "oc",
        name: "OpenCode Free",
        category: "local",
        auth_type: "none",
        format: "openai",
        base_url: "http://localhost:4096/v1/chat/completions",
        api_key_hint: "No auth",
        website: "https://opencode.ai",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
    ProviderProfile {
        id: "ollama-local",
        alias: "ollama-local",
        name: "Ollama Local",
        category: "local",
        auth_type: "none",
        format: "openai",
        base_url: "http://localhost:11434/v1/chat/completions",
        api_key_hint: "No auth",
        website: "https://ollama.com",
        models_url: "",
        default_headers: &[],
        fallback_models: &[],
    },
];

pub fn provider_profiles() -> &'static [ProviderProfile] {
    PROVIDER_PROFILES
}

pub fn provider_profile(id_or_alias: &str) -> Option<&'static ProviderProfile> {
    let id_or_alias = id_or_alias.to_ascii_lowercase();
    PROVIDER_PROFILES
        .iter()
        .find(|profile| profile.id == id_or_alias || profile.alias == id_or_alias)
}

pub fn builtin_provider_models(
    provider_id: &str,
    alias: Option<&str>,
) -> Option<BuiltinProviderModels> {
    let provider_id = provider_id.to_ascii_lowercase();
    let alias = alias.unwrap_or_default().to_ascii_lowercase();

    match (provider_id.as_str(), alias.as_str()) {
        ("cline", _) | (_, "cl") => Some(BuiltinProviderModels {
            label: "Cline Router",
            source: "builtin://cline",
            models: CLINE_MODELS,
        }),
        ("commandcode", _) | (_, "cmc") => Some(BuiltinProviderModels {
            label: "Command Code",
            source: "builtin://commandcode",
            models: COMMANDCODE_MODELS,
        }),
        ("antigravity", _) | (_, "ag") => Some(BuiltinProviderModels {
            label: "Google Antigravity",
            source: "builtin://antigravity",
            models: ANTIGRAVITY_MODELS,
        }),
        ("gemini-cli", _) => Some(BuiltinProviderModels {
            label: "Gemini CLI",
            source: "builtin://gemini-cli",
            models: GEMINI_CLI_MODELS,
        }),
        ("codex", _) | (_, "cx") => Some(BuiltinProviderModels {
            label: "Codex",
            source: "builtin://codex",
            models: CODEX_MODELS,
        }),
        ("openai", _) => Some(BuiltinProviderModels {
            label: "OpenAI/Codex",
            source: "builtin://openai-codex",
            models: CODEX_MODELS,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_lookup_accepts_id_and_alias() {
        assert_eq!(
            provider_profile("antigravity").map(|profile| profile.alias),
            Some("ag")
        );
        assert_eq!(
            provider_profile("AG").map(|profile| profile.id),
            Some("antigravity")
        );
    }

    #[test]
    fn provider_profile_serializes_public_catalog_fields_only() {
        let value = serde_json::to_value(provider_profile("antigravity").unwrap()).unwrap();

        assert_eq!(value["id"], "antigravity");
        assert_eq!(value["alias"], "ag");
        assert_eq!(value["format"], "antigravity");
        assert!(value.get("models_url").is_none());
        assert!(value.get("default_headers").is_none());
        assert!(value.get("fallback_models").is_none());
    }

    #[test]
    fn builtin_models_cover_antigravity_and_gemini_cli() {
        let antigravity = builtin_provider_models("antigravity", None).unwrap();
        assert!(antigravity.models.contains(&"gemini-3.5-flash-extra-low"));
        assert!(antigravity.models.contains(&"gemini-2.5-flash"));

        let gemini_cli = builtin_provider_models("gemini-cli", None).unwrap();
        assert_eq!(gemini_cli.source, "builtin://gemini-cli");
        assert!(gemini_cli.models.contains(&"gemini-2.5-pro"));

        let codex = builtin_provider_models("codex", None).unwrap();
        assert_eq!(codex.source, "builtin://codex");
        assert!(codex.models.contains(&"gpt-5.3-codex"));

        let openai = builtin_provider_models("openai", None).unwrap();
        assert_eq!(openai.source, "builtin://openai-codex");
        assert!(openai.models.contains(&"gpt-5.3-codex"));
    }
}
