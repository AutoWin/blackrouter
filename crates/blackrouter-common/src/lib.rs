use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, BlackRouterError>;

#[derive(Debug, Error)]
pub enum BlackRouterError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct BuildInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub rust_runtime: &'static str,
}

impl Default for BuildInfo {
    fn default() -> Self {
        Self {
            name: "blackrouter",
            version: env!("CARGO_PKG_VERSION"),
            rust_runtime: "tokio",
        }
    }
}

pub fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

pub fn mask_secret(value: &str) -> String {
    let trimmed = value.trim();
    let len = trimmed.chars().count();

    if len == 0 {
        return String::new();
    }

    if len <= 8 {
        return "*".repeat(len);
    }

    let prefix: String = trimmed.chars().take(4).collect();
    let suffix: String = trimmed
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    format!("{prefix}...{suffix}")
}

#[cfg(test)]
mod tests {
    use super::mask_secret;

    #[test]
    fn masks_short_values() {
        assert_eq!(mask_secret("abc"), "***");
        assert_eq!(mask_secret("12345678"), "********");
    }

    #[test]
    fn masks_long_values() {
        assert_eq!(mask_secret("sk-1234567890abcdef"), "sk-1...cdef");
    }
}
