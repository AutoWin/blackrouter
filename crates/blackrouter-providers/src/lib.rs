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
