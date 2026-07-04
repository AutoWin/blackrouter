use serde::Serialize;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum WireFormat {
    OpenAiChat,
    OpenAiResponses,
    ClaudeMessages,
    Gemini,
    GeminiCli,
    Kiro,
    Antigravity,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TranslationError {
    #[error("unsupported translation: {from:?} -> {to:?}")]
    Unsupported { from: WireFormat, to: WireFormat },
}

pub fn is_passthrough(source: WireFormat, target: WireFormat) -> bool {
    source == target
}
