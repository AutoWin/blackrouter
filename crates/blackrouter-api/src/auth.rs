use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use blackrouter_config::AppConfig;
use blackrouter_storage::StorageError;

use crate::state::AppState;

// ── ApiErrorResponse ──────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ApiErrorResponse {
    pub status: StatusCode,
    pub message: String,
    pub error_type: &'static str,
}

impl ApiErrorResponse {
    pub fn new(status: StatusCode, message: impl Into<String>, error_type: &'static str) -> Self {
        Self {
            status,
            message: message.into(),
            error_type,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message, "server_error")
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message, "invalid_request_error")
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message, "not_found")
    }
}

impl IntoResponse for ApiErrorResponse {
    fn into_response(self) -> Response {
        let body = Json(json!({
            "error": {
                "message": self.message,
                "type": self.error_type,
                "param": null,
                "code": null
            }
        }));
        (self.status, body).into_response()
    }
}

// ── v1 API key auth ──────────────────────────────────────────────────────────

pub fn authorize_v1(state: &AppState, headers: &HeaderMap) -> Result<(), ApiErrorResponse> {
    if !state.config.require_api_key {
        return Ok(());
    }

    let api_key = extract_api_key(headers).ok_or_else(|| {
        ApiErrorResponse::new(
            StatusCode::UNAUTHORIZED,
            "Missing API key",
            "authentication_error",
        )
    })?;

    let valid = state.storage.is_valid_api_key(&api_key).map_err(|error| {
        ApiErrorResponse::internal(format!("API key validation failed: {error}"))
    })?;

    if valid {
        Ok(())
    } else {
        Err(ApiErrorResponse::new(
            StatusCode::UNAUTHORIZED,
            "Invalid API key",
            "authentication_error",
        ))
    }
}

pub fn extract_api_key(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
    {
        let value = value.trim();
        if value
            .get(..7)
            .map(|prefix| prefix.eq_ignore_ascii_case("bearer "))
            .unwrap_or(false)
        {
            return Some(value[7..].trim().to_string());
        }
    }

    headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

// ── Control-plane auth middleware ─────────────────────────────────────────────

/// Axum middleware that enforces the control-token on control-plane (`/api/*`)
/// routes when `control_api_enabled` is true in the config.
///
/// Accepted header formats:
///   - `Authorization: Bearer <token>`
///   - `X-Control-Token: <token>`
///
/// When `control_api_enabled` is false the request passes through unguarded
/// (legacy / trusted-network mode).  A warning is emitted at startup for that
/// case from `main.rs`.
pub async fn enforce_control_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: axum::extract::Request,
    next: Next,
) -> Result<Response, ApiErrorResponse> {
    // If control-plane auth is not enabled, pass through (legacy mode).
    if !state.config.control_api_enabled {
        return Ok(next.run(req).await);
    }

    // control_api_enabled is true — the token MUST be present (validated at startup).
    let expected = state.config.control_token.as_deref().unwrap();

    let provided = extract_control_token(&headers);

    match provided {
        Some(token) if constant_time_eq(token.as_bytes(), expected.as_bytes()) => {
            Ok(next.run(req).await)
        }
        _ => Err(ApiErrorResponse::new(
            StatusCode::UNAUTHORIZED,
            "Invalid or missing control token",
            "authentication_error",
        )),
    }
}

/// Extract a control-token from the request headers.
///
/// Checks `X-Control-Token` first (explicit control-plane header), then
/// falls back to `Authorization: Bearer <token>`.
fn extract_control_token(headers: &HeaderMap) -> Option<String> {
    // 1. X-Control-Token header
    if let Some(value) = headers
        .get("x-control-token")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return Some(value.to_string());
    }

    // 2. Authorization: Bearer <token>
    if let Some(value) = headers.get("authorization").and_then(|v| v.to_str().ok()) {
        let value = value.trim();
        if value
            .get(..7)
            .map(|p| p.eq_ignore_ascii_case("bearer "))
            .unwrap_or(false)
        {
            let token = value[7..].trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }

    None
}

/// Constant-time byte comparison to avoid timing side-channels.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ── Storage-error mapping ────────────────────────────────────────────────────

pub fn storage_error_to_api(error: StorageError, context: &'static str) -> ApiErrorResponse {
    match error {
        StorageError::Validation(message) if message.contains("not found") => {
            ApiErrorResponse::not_found(message)
        }
        StorageError::Validation(message) => ApiErrorResponse::bad_request(message),
        other => ApiErrorResponse::internal(format!("{context}: {other}")),
    }
}

// ── Config validation (called from main.rs) ──────────────────────────────────

/// Validate control-plane config at startup.  Returns an error if the
/// configuration is insecure (control_api_enabled without a token).
pub fn validate_control_config(config: &AppConfig) -> Result<(), String> {
    if config.control_api_enabled && config.control_token.is_none() {
        return Err(
            "BLACKROUTER_CONTROL_API_ENABLED=true requires BLACKROUTER_CONTROL_TOKEN to be set \
             — refusing to start with an unauthenticated control plane."
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches() {
        assert!(constant_time_eq(b"secret", b"secret"));
    }

    #[test]
    fn constant_time_eq_mismatch() {
        assert!(!constant_time_eq(b"secret", b"SECRET"));
    }

    #[test]
    fn constant_time_eq_different_lengths() {
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }

    #[test]
    fn extract_control_token_from_x_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-control-token", "mytoken".parse().unwrap());
        assert_eq!(extract_control_token(&headers), Some("mytoken".to_string()));
    }

    #[test]
    fn extract_control_token_from_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer mytoken".parse().unwrap());
        assert_eq!(extract_control_token(&headers), Some("mytoken".to_string()));
    }

    #[test]
    fn extract_control_token_prefers_x_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-control-token", "xtoken".parse().unwrap());
        headers.insert("authorization", "Bearer bearertoken".parse().unwrap());
        assert_eq!(extract_control_token(&headers), Some("xtoken".to_string()));
    }

    #[test]
    fn extract_control_token_empty_returns_none() {
        let mut headers = HeaderMap::new();
        headers.insert("x-control-token", "".parse().unwrap());
        assert_eq!(extract_control_token(&headers), None);
    }

    #[test]
    fn validate_control_config_requires_token_when_enabled() {
        let config = AppConfig {
            host: "0.0.0.0".to_string(),
            port: 20130,
            data_dir: "/tmp".into(),
            database_url: "sqlite:///tmp/test.db".to_string(),
            database_path: "/tmp/test.db".into(),
            compat_9router_db: false,
            require_api_key: false,
            control_api_enabled: true,
            control_token: None,
            log_level: "info".to_string(),
            telegram: blackrouter_config::TelegramConfig {
                enabled: false,
                bot_token: None,
                admin_ids: vec![],
                link_code_ttl_seconds: 300,
                use_webhook: false,
                webhook_url: None,
            },
        };
        assert!(validate_control_config(&config).is_err());
    }

    #[test]
    fn validate_control_config_ok_with_token() {
        let config = AppConfig {
            host: "0.0.0.0".to_string(),
            port: 20130,
            data_dir: "/tmp".into(),
            database_url: "sqlite:///tmp/test.db".to_string(),
            database_path: "/tmp/test.db".into(),
            compat_9router_db: false,
            require_api_key: false,
            control_api_enabled: true,
            control_token: Some("s3cret".to_string()),
            log_level: "info".to_string(),
            telegram: blackrouter_config::TelegramConfig {
                enabled: false,
                bot_token: None,
                admin_ids: vec![],
                link_code_ttl_seconds: 300,
                use_webhook: false,
                webhook_url: None,
            },
        };
        assert!(validate_control_config(&config).is_ok());
    }

    #[test]
    fn validate_control_config_ok_when_disabled() {
        let config = AppConfig {
            host: "0.0.0.0".to_string(),
            port: 20130,
            data_dir: "/tmp".into(),
            database_url: "sqlite:///tmp/test.db".to_string(),
            database_path: "/tmp/test.db".into(),
            compat_9router_db: false,
            require_api_key: false,
            control_api_enabled: false,
            control_token: None,
            log_level: "info".to_string(),
            telegram: blackrouter_config::TelegramConfig {
                enabled: false,
                bot_token: None,
                admin_ids: vec![],
                link_code_ttl_seconds: 300,
                use_webhook: false,
                webhook_url: None,
            },
        };
        assert!(validate_control_config(&config).is_ok());
    }
}
