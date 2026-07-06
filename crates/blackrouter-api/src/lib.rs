use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use blackrouter_common::{unix_timestamp, BuildInfo};
use blackrouter_config::AppConfig;
use blackrouter_core::{ModelRef, RouteKind};
use blackrouter_rtk::{RateLimitConfig, RequestKey, Rtk};
use blackrouter_storage::{
    ApiKeyRecord, ComboRecord, CreatedApiKey, DailyUsage, ModelListItem, NewApiKey, NewCombo,
    NewProviderConnection, ProviderConnectionRecord, RawProviderConnection, RequestDetailEntry,
    Storage, StorageError, StorageStatus, UsageEntry,
};
use blackrouter_translator::{
    chat_response_to_responses, responses_request_to_chat, stream::translate_sse_stream,
    translate_request, translate_response, WireFormat,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

const MAX_REQUEST_BYTES: usize = 50 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub storage: Storage,
    pub started_at_unix: u64,
    pub rtk: Rtk,
    pub http_client: reqwest::Client,
}

impl AppState {
    pub fn new(config: AppConfig, storage: Storage) -> Self {
        let rtk_config = RateLimitConfig {
            requests_per_minute: 60,
            tokens_per_minute: 100_000,
            max_concurrent: 10,
            enabled: true,
        };

        // Shared HTTP client with connection pooling (Phase 1.2)
        let http_client = reqwest::Client::builder()
            .pool_max_idle_per_host(100)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60))
            .tcp_nodelay(true)
            .timeout(Duration::from_secs(600))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .expect("Failed to build shared HTTP client");

        Self {
            config,
            storage,
            started_at_unix: unix_timestamp(),
            rtk: Rtk::new(rtk_config),
            http_client,
        }
    }
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(setup_redirect))
        .route("/setup", get(setup_page))
        .route("/setup.css", get(setup_css))
        .route("/setup.js", get(setup_js))
        .route("/health", get(health))
        .route("/version", get(version))
        .route("/api/runtime/status", get(runtime_status))
        .route(
            "/api/setup/config",
            get(setup_config).put(save_setup_config),
        )
        .route(
            "/api/setup/api-keys",
            get(list_api_keys).post(create_api_key),
        )
        .route(
            "/api/setup/providers",
            get(list_providers).post(create_provider),
        )
        .route(
            "/api/setup/providers/{id}",
            get(get_provider)
                .put(update_provider)
                .delete(delete_provider),
        )
        .route("/api/setup/providers/{id}/toggle", post(toggle_provider))
        .route("/api/setup/providers/{id}/test", post(test_provider))
        .route(
            "/api/setup/providers/{id}/models",
            post(fetch_provider_models),
        )
        .route("/api/setup/provider-catalog", get(provider_catalog))
        .route("/api/setup/combos", get(list_combos).post(create_combo))
        .route(
            "/api/setup/combos/{id}",
            get(get_combo).put(update_combo).delete(delete_combo),
        )
        .route("/v1/models", get(v1_models))
        .route("/v1beta/models", get(v1_models))
        .route("/v1/chat/completions", post(chat_completions_shell))
        .route("/v1/responses", post(responses_proxy))
        .route("/v1/messages", post(messages_proxy))
        .route("/api/rtk/metrics", get(rtk_metrics))
        .route("/api/rtk/status/{provider}/{model}", get(rtk_status))
        .route("/api/usage", get(usage_stats))
        .route("/api/usage/daily", get(list_daily_usage))
        .route("/api/usage/daily/{date}", get(get_daily_usage))
        .route("/api/usage/aggregate", post(aggregate_daily))
        .with_state(state)
        .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(600),
        ))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}

async fn setup_redirect() -> Redirect {
    Redirect::temporary("/setup")
}

async fn setup_page() -> Html<&'static str> {
    Html(include_str!("../static/setup.html"))
}

async fn setup_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/setup.css"),
    )
}

async fn setup_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../static/setup.js"),
    )
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    timestamp: u64,
    database: HealthDatabase,
}

#[derive(Serialize)]
struct HealthDatabase {
    path: String,
    schema_compatible: bool,
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let storage_status = state.storage.status().ok();
    let schema_compatible = storage_status
        .as_ref()
        .map(|status| status.schema_compatible)
        .unwrap_or(false);

    Json(HealthResponse {
        status: if schema_compatible { "ok" } else { "degraded" },
        service: "blackrouter",
        timestamp: unix_timestamp(),
        database: HealthDatabase {
            path: state.storage.database_path().display().to_string(),
            schema_compatible,
        },
    })
}

async fn version() -> Json<BuildInfo> {
    Json(BuildInfo::default())
}

#[derive(Serialize)]
struct RuntimeStatusResponse {
    service: &'static str,
    started_at_unix: u64,
    uptime_seconds: u64,
    config: AppConfig,
    storage: StorageStatus,
}

async fn runtime_status(
    State(state): State<AppState>,
) -> Result<Json<RuntimeStatusResponse>, ApiErrorResponse> {
    let storage = state
        .storage
        .status()
        .map_err(|error| ApiErrorResponse::internal(format!("storage status failed: {error}")))?;

    Ok(Json(RuntimeStatusResponse {
        service: "blackrouter",
        started_at_unix: state.started_at_unix,
        uptime_seconds: unix_timestamp().saturating_sub(state.started_at_unix),
        config: state.config,
        storage,
    }))
}

#[derive(Serialize)]
struct SetupConfigResponse {
    settings: Value,
}

#[derive(Debug, Deserialize)]
struct SetupConfigPayload {
    require_api_key: bool,
    telegram_enabled: bool,
    telegram_admin_ids: Vec<i64>,
    telegram_link_code_ttl_seconds: u64,
    telegram_use_webhook: bool,
    telegram_webhook_url: Option<String>,
}

async fn setup_config(
    State(state): State<AppState>,
) -> Result<Json<SetupConfigResponse>, ApiErrorResponse> {
    let settings = state
        .storage
        .settings_json()
        .map_err(|error| ApiErrorResponse::internal(format!("settings load failed: {error}")))?;
    Ok(Json(SetupConfigResponse { settings }))
}

async fn save_setup_config(
    State(state): State<AppState>,
    Json(payload): Json<SetupConfigPayload>,
) -> Result<Json<SetupConfigResponse>, ApiErrorResponse> {
    let settings = json!({
        "requireApiKey": payload.require_api_key,
        "telegram": {
            "enabled": payload.telegram_enabled,
            "adminIds": payload.telegram_admin_ids,
            "linkCodeTtlSeconds": payload.telegram_link_code_ttl_seconds,
            "useWebhook": payload.telegram_use_webhook,
            "webhookUrl": payload.telegram_webhook_url,
        }
    });

    let settings = state
        .storage
        .save_settings_json(&settings)
        .map_err(|error| ApiErrorResponse::internal(format!("settings save failed: {error}")))?;

    Ok(Json(SetupConfigResponse { settings }))
}

#[derive(Serialize)]
struct ApiKeysResponse {
    data: Vec<ApiKeyRecord>,
}

async fn list_api_keys(
    State(state): State<AppState>,
) -> Result<Json<ApiKeysResponse>, ApiErrorResponse> {
    let data = state
        .storage
        .list_api_keys()
        .map_err(|error| ApiErrorResponse::internal(format!("API key listing failed: {error}")))?;
    Ok(Json(ApiKeysResponse { data }))
}

async fn create_api_key(
    State(state): State<AppState>,
    Json(payload): Json<NewApiKey>,
) -> Result<Json<CreatedApiKey>, ApiErrorResponse> {
    let created = state
        .storage
        .create_api_key(payload)
        .map_err(|error| ApiErrorResponse::internal(format!("API key creation failed: {error}")))?;
    Ok(Json(created))
}

#[derive(Serialize)]
struct ProvidersResponse {
    data: Vec<ProviderConnectionRecord>,
}

#[derive(Debug, Deserialize)]
struct ToggleProviderPayload {
    is_active: bool,
}

#[derive(Debug, Serialize)]
struct DeleteResponse {
    ok: bool,
}

#[derive(Debug, Serialize)]
struct ProviderTestResponse {
    ok: bool,
    reachable: bool,
    status: Option<u16>,
    url: Option<String>,
    message: String,
}

#[derive(Debug, Serialize)]
struct ProviderModelsResponse {
    ok: bool,
    provider: ProviderConnectionRecord,
    models: Vec<String>,
    models_url: String,
    message: String,
}

#[derive(Clone, Debug, Serialize)]
struct ProviderCatalogItem {
    id: &'static str,
    alias: &'static str,
    name: &'static str,
    category: &'static str,
    auth_type: &'static str,
    format: &'static str,
    base_url: &'static str,
    api_key_hint: &'static str,
    website: &'static str,
    required: bool,
}

async fn list_providers(
    State(state): State<AppState>,
) -> Result<Json<ProvidersResponse>, ApiErrorResponse> {
    let data = state
        .storage
        .list_provider_connections()
        .map_err(|error| ApiErrorResponse::internal(format!("provider listing failed: {error}")))?;
    Ok(Json(ProvidersResponse { data }))
}

async fn create_provider(
    State(state): State<AppState>,
    Json(payload): Json<NewProviderConnection>,
) -> Result<Json<ProviderConnectionRecord>, ApiErrorResponse> {
    let created = state
        .storage
        .create_provider_connection(payload)
        .map_err(|error| {
            ApiErrorResponse::internal(format!("provider creation failed: {error}"))
        })?;
    Ok(Json(created))
}

async fn get_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProviderConnectionRecord>, ApiErrorResponse> {
    let raw = state
        .storage
        .get_provider_connection_raw(&id)
        .map_err(|error| ApiErrorResponse::not_found(format!("{error}")))?;

    Ok(Json(ProviderConnectionRecord {
        id: raw.id,
        provider: raw.provider,
        auth_type: raw.auth_type,
        name: raw.name,
        email: raw.email,
        priority: raw.priority,
        is_active: raw.is_active,
        data: mask_for_api(raw.data),
        created_at: raw.created_at,
        updated_at: raw.updated_at,
    }))
}

async fn update_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<NewProviderConnection>,
) -> Result<Json<ProviderConnectionRecord>, ApiErrorResponse> {
    let updated = state
        .storage
        .update_provider_connection(&id, payload)
        .map_err(|error| ApiErrorResponse::internal(format!("provider update failed: {error}")))?;
    Ok(Json(updated))
}

async fn toggle_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<ToggleProviderPayload>,
) -> Result<Json<ProviderConnectionRecord>, ApiErrorResponse> {
    let updated = state
        .storage
        .set_provider_connection_active(&id, payload.is_active)
        .map_err(|error| ApiErrorResponse::internal(format!("provider toggle failed: {error}")))?;
    Ok(Json(updated))
}

async fn delete_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<DeleteResponse>, ApiErrorResponse> {
    state
        .storage
        .delete_provider_connection(&id)
        .map_err(|error| ApiErrorResponse::internal(format!("provider delete failed: {error}")))?;
    Ok(Json(DeleteResponse { ok: true }))
}

async fn test_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProviderTestResponse>, ApiErrorResponse> {
    let provider = state
        .storage
        .get_provider_connection_raw(&id)
        .map_err(|error| ApiErrorResponse::not_found(format!("{error}")))?;

    Ok(Json(check_provider_connection(provider).await))
}

async fn fetch_provider_models(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ProviderModelsResponse>, ApiErrorResponse> {
    let provider = state
        .storage
        .get_provider_connection_raw(&id)
        .map_err(|error| ApiErrorResponse::not_found(format!("{error}")))?;

    let models_url = provider_models_url(&provider.data)?;
    let (models, models_url, message) = match fetch_provider_model_ids(&provider, &models_url).await
    {
        Ok(models) => {
            let message = format!("Fetched {} models", models.len());
            (models, models_url, message)
        }
        Err(error) => {
            let fallback = builtin_provider_models(&provider).ok_or(error)?;
            let models = fallback
                .models
                .iter()
                .map(|model| (*model).to_string())
                .collect::<Vec<_>>();
            let message = format!(
                "Loaded {} built-in {} models because live model fetch is not supported",
                models.len(),
                fallback.label
            );
            (models, fallback.source.to_string(), message)
        }
    };
    let updated = state
        .storage
        .set_provider_connection_models(&id, models.clone(), Some(models_url.clone()))
        .map_err(|error| storage_error_to_api(error, "provider model save failed"))?;

    Ok(Json(ProviderModelsResponse {
        ok: true,
        provider: updated,
        models: models.clone(),
        models_url,
        message,
    }))
}

async fn provider_catalog() -> Json<Vec<ProviderCatalogItem>> {
    Json(PROVIDER_CATALOG.to_vec())
}

#[derive(Serialize)]
struct CombosResponse {
    data: Vec<ComboRecord>,
}

async fn list_combos(
    State(state): State<AppState>,
) -> Result<Json<CombosResponse>, ApiErrorResponse> {
    let data = state
        .storage
        .list_combos()
        .map_err(|error| storage_error_to_api(error, "combo listing failed"))?;
    Ok(Json(CombosResponse { data }))
}

async fn create_combo(
    State(state): State<AppState>,
    Json(payload): Json<NewCombo>,
) -> Result<Json<ComboRecord>, ApiErrorResponse> {
    let created = state
        .storage
        .create_combo(payload)
        .map_err(|error| storage_error_to_api(error, "combo creation failed"))?;
    Ok(Json(created))
}

async fn get_combo(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ComboRecord>, ApiErrorResponse> {
    let combo = state
        .storage
        .get_combo(&id)
        .map_err(|error| storage_error_to_api(error, "combo load failed"))?;
    Ok(Json(combo))
}

async fn update_combo(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<NewCombo>,
) -> Result<Json<ComboRecord>, ApiErrorResponse> {
    let updated = state
        .storage
        .update_combo(&id, payload)
        .map_err(|error| storage_error_to_api(error, "combo update failed"))?;
    Ok(Json(updated))
}

async fn delete_combo(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<DeleteResponse>, ApiErrorResponse> {
    state
        .storage
        .delete_combo(&id)
        .map_err(|error| storage_error_to_api(error, "combo delete failed"))?;
    Ok(Json(DeleteResponse { ok: true }))
}

async fn check_provider_connection(provider: RawProviderConnection) -> ProviderTestResponse {
    let url = provider
        .data
        .get("baseUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let Some(url) = url else {
        return ProviderTestResponse {
            ok: false,
            reachable: false,
            status: None,
            url: None,
            message: "Missing data.baseUrl".to_string(),
        };
    };

    if let Err(message) = validate_provider_auth(&provider) {
        return ProviderTestResponse {
            ok: false,
            reachable: false,
            status: None,
            url: Some(url),
            message,
        };
    }

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return ProviderTestResponse {
                ok: false,
                reachable: false,
                status: None,
                url: Some(url),
                message: format!("Failed to create HTTP client: {error}"),
            };
        }
    };

    let mut request = client.head(&url);
    if let Some(token) = provider_token(&provider.data) {
        request = request.bearer_auth(token);
    }

    match request.send().await {
        Ok(response) => classify_provider_response(url, response.status().as_u16()),
        Err(head_error) => match client.get(&url).send().await {
            Ok(response) => classify_provider_response(url, response.status().as_u16()),
            Err(get_error) => ProviderTestResponse {
                ok: false,
                reachable: false,
                status: None,
                url: Some(url),
                message: format!("Connection failed: {get_error}; HEAD error: {head_error}"),
            },
        },
    }
}

fn classify_provider_response(url: String, status: u16) -> ProviderTestResponse {
    let (ok, message) = match status {
        200..=399 => (true, "Provider endpoint is reachable".to_string()),
        401 | 403 => (
            false,
            "Provider endpoint is reachable, but authentication failed".to_string(),
        ),
        404 => (
            false,
            "Provider host is reachable, but the configured endpoint was not found".to_string(),
        ),
        405 => (
            true,
            "Provider endpoint is reachable; this API requires a POST request".to_string(),
        ),
        400..=499 => (
            false,
            format!("Provider endpoint is reachable, but returned HTTP {status}"),
        ),
        _ => (false, format!("Provider endpoint returned HTTP {status}")),
    };

    ProviderTestResponse {
        ok,
        reachable: true,
        status: Some(status),
        url: Some(url),
        message,
    }
}

fn validate_provider_auth(provider: &RawProviderConnection) -> Result<(), String> {
    let auth_type = provider.auth_type.to_ascii_lowercase();
    if auth_type == "none" {
        return Ok(());
    }

    if (auth_type.contains("api") || auth_type.contains("oauth"))
        && provider_token(&provider.data).is_none()
    {
        return Err("Missing API key/access token in provider data".to_string());
    }

    Ok(())
}

fn provider_token(data: &Value) -> Option<&str> {
    data.get("apiKey")
        .or_else(|| data.get("api_key"))
        .or_else(|| data.get("accessToken"))
        .or_else(|| data.get("token"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn provider_models_url(data: &Value) -> Result<String, ApiErrorResponse> {
    if let Some(models_url) = data
        .get("modelsUrl")
        .or_else(|| data.get("models_url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(models_url.to_string());
    }

    let base_url = data
        .get("baseUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiErrorResponse::bad_request("Missing data.baseUrl"))?;

    derive_models_url(base_url)
        .ok_or_else(|| ApiErrorResponse::bad_request("Could not derive models URL from baseUrl"))
}

fn derive_models_url(base_url: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(base_url).ok()?;
    let path = url.path().trim_end_matches('/').to_string();
    let models_path = if path.ends_with("/chat/completions") {
        path.trim_end_matches("/chat/completions").to_string() + "/models"
    } else if path.ends_with("/messages") {
        path.trim_end_matches("/messages").to_string() + "/models"
    } else if path.ends_with("/responses") {
        path.trim_end_matches("/responses").to_string() + "/models"
    } else if path.ends_with("/models") {
        path
    } else if path.ends_with("/generate") {
        path.trim_end_matches("/generate").to_string() + "/models"
    } else if path.ends_with("/generateContent") {
        path.trim_end_matches("/generateContent").to_string()
    } else {
        return None;
    };
    url.set_path(&models_path);
    Some(url.to_string())
}

async fn fetch_provider_model_ids(
    provider: &RawProviderConnection,
    models_url: &str,
) -> Result<Vec<String>, ApiErrorResponse> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|error| {
            ApiErrorResponse::internal(format!("Failed to create HTTP client: {error}"))
        })?;

    let mut request_url = models_url.to_string();
    if provider
        .data
        .get("format")
        .and_then(Value::as_str)
        .map(|format| format.eq_ignore_ascii_case("gemini"))
        .unwrap_or(false)
    {
        if let Some(token) = provider_token(&provider.data) {
            if let Ok(mut url) = reqwest::Url::parse(models_url) {
                url.query_pairs_mut().append_pair("key", token);
                request_url = url.to_string();
            }
        }
    }

    let mut request = client.get(&request_url);
    if let Some(headers) = provider.data.get("headers").and_then(Value::as_object) {
        for (key, value) in headers {
            if let Some(value) = value.as_str() {
                request = request.header(key, value);
            }
        }
    }
    if !request_url.contains("key=") {
        if let Some(token) = provider_token(&provider.data) {
            request = request.bearer_auth(token);
        }
    }

    let response = request
        .send()
        .await
        .map_err(|error| ApiErrorResponse::bad_request(format!("Model fetch failed: {error}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(ApiErrorResponse::bad_request(format!(
            "Model fetch returned HTTP {}",
            status.as_u16()
        )));
    }

    let payload = response.json::<Value>().await.map_err(|error| {
        ApiErrorResponse::bad_request(format!("Model fetch returned invalid JSON: {error}"))
    })?;
    let models = extract_model_ids(&payload);
    if models.is_empty() {
        return Err(ApiErrorResponse::bad_request(
            "Model fetch succeeded but no model ids were found",
        ));
    }
    Ok(models)
}

fn extract_model_ids(payload: &Value) -> Vec<String> {
    let mut models = Vec::new();
    for key in ["data", "models", "results"] {
        if let Some(items) = payload.get(key).and_then(Value::as_array) {
            collect_model_ids(items, &mut models);
        }
    }
    if models.is_empty() {
        if let Some(items) = payload.as_array() {
            collect_model_ids(items, &mut models);
        }
    }
    models.sort();
    models.dedup();
    models
}

fn collect_model_ids(items: &[Value], out: &mut Vec<String>) {
    for item in items {
        let id = match item {
            Value::String(value) => Some(value.as_str()),
            Value::Object(map) => map
                .get("id")
                .or_else(|| map.get("name"))
                .or_else(|| map.get("model"))
                .and_then(Value::as_str),
            _ => None,
        };
        if let Some(id) = id.map(str::trim).filter(|value| !value.is_empty()) {
            out.push(id.trim_start_matches("models/").to_string());
        }
    }
}

struct BuiltinProviderModels {
    label: &'static str,
    source: &'static str,
    models: &'static [&'static str],
}

fn builtin_provider_models(provider: &RawProviderConnection) -> Option<BuiltinProviderModels> {
    let provider_id = provider.provider.to_ascii_lowercase();
    let alias = provider
        .data
        .get("alias")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();

    match (provider_id.as_str(), alias.as_str()) {
        ("cline", _) | (_, "cl") => Some(BuiltinProviderModels {
            label: "Cline Router",
            source: "builtin://cline",
            models: &[
                "cline-pass/qwen3.7-max",
                "cline-pass/qwen3.7-plus",
                "cline-pass/minimax-m3",
                "cline-pass/mimo-v2.5-pro",
                "cline-pass/glm-5.2",
                "cline-pass/mimo-v2.5",
                "cline-pass/kimi-k2.7-code",
                "cline-pass/deepseek-v4-flash",
                "cline-pass/deepseek-v4-pro",
                "cline-pass/kimi-k2.6",
                "stepfun/step-3.7-flash",
                "deepseek/deepseek-v4-flash",
                "zai/glm-5.2",
                "moonshotai/kimi-k2.7-code",
                "anthropic/claude-opus-4.8",
                "anthropic/claude-sonnet-4.6",
                "openai/gpt-5.5",
            ],
        }),
        ("commandcode", _) | (_, "cmc") => Some(BuiltinProviderModels {
            label: "Command Code",
            source: "builtin://commandcode",
            models: &[
                "claude-sonnet-4-6",
                "claude-fable-5",
                "claude-opus-4-8",
                "claude-opus-4-7",
                "claude-haiku-4-5-20251001",
                "gpt-5.5",
                "gpt-5.4",
                "gpt-5.3-codex",
                "gpt-5.4-mini",
                "moonshotai/Kimi-K2.6",
                "moonshotai/Kimi-K2.5",
                "zai-org/GLM-5.2",
                "zai-org/GLM-5.1",
                "zai-org/GLM-5",
                "MiniMaxAI/MiniMax-M3",
                "MiniMaxAI/MiniMax-M2.7",
                "MiniMaxAI/MiniMax-M2.5",
                "deepseek/deepseek-v4-pro",
                "deepseek/deepseek-v4-flash",
                "Qwen/Qwen3.6-Max-Preview",
                "Qwen/Qwen3.6-Plus",
                "Qwen/Qwen3.7-Max",
            ],
        }),
        _ => None,
    }
}

fn mask_for_api(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let masked = if is_sensitive_key(&key) {
                        value
                            .as_str()
                            .map(blackrouter_common::mask_secret)
                            .map(Value::String)
                            .unwrap_or(Value::String("***".to_string()))
                    } else {
                        mask_for_api(value)
                    };
                    (key, masked)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(mask_for_api).collect()),
        other => other,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("apikey")
        || key.contains("api_key")
        || key.contains("token")
        || key.contains("secret")
        || key.contains("password")
        || key == "authorization"
}

const PROVIDER_CATALOG: &[ProviderCatalogItem] = &[
    ProviderCatalogItem {
        id: "commandcode",
        alias: "cmc",
        name: "Command Code",
        category: "coding",
        auth_type: "api-key",
        format: "commandcode",
        base_url: "https://api.commandcode.ai/alpha/generate",
        api_key_hint: "user_... from ~/.commandcode/auth.json or commandcode.ai/studio",
        website: "https://commandcode.ai",
        required: true,
    },
    ProviderCatalogItem {
        id: "cline",
        alias: "cl",
        name: "Cline Router",
        category: "coding",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.cline.bot/api/v1/chat/completions",
        api_key_hint: "Cline auth token or API key",
        website: "https://cline.bot",
        required: true,
    },
    ProviderCatalogItem {
        id: "openrouter",
        alias: "openrouter",
        name: "OpenRouter",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://openrouter.ai/api/v1/chat/completions",
        api_key_hint: "OpenRouter API key",
        website: "https://openrouter.ai",
        required: false,
    },
    ProviderCatalogItem {
        id: "openai",
        alias: "openai",
        name: "OpenAI",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.openai.com/v1/chat/completions",
        api_key_hint: "sk-...",
        website: "https://platform.openai.com",
        required: false,
    },
    ProviderCatalogItem {
        id: "anthropic",
        alias: "anthropic",
        name: "Anthropic",
        category: "api-key",
        auth_type: "api-key",
        format: "claude",
        base_url: "https://api.anthropic.com/v1/messages",
        api_key_hint: "sk-ant-...",
        website: "https://console.anthropic.com",
        required: false,
    },
    ProviderCatalogItem {
        id: "gemini",
        alias: "gemini",
        name: "Gemini",
        category: "free-tier",
        auth_type: "api-key",
        format: "gemini",
        base_url: "https://generativelanguage.googleapis.com/v1beta/models",
        api_key_hint: "Google AI Studio API key",
        website: "https://ai.google.dev",
        required: false,
    },
    ProviderCatalogItem {
        id: "deepseek",
        alias: "ds",
        name: "DeepSeek",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.deepseek.com/chat/completions",
        api_key_hint: "DeepSeek API key",
        website: "https://platform.deepseek.com",
        required: false,
    },
    ProviderCatalogItem {
        id: "groq",
        alias: "groq",
        name: "Groq",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.groq.com/openai/v1/chat/completions",
        api_key_hint: "Groq API key",
        website: "https://console.groq.com",
        required: false,
    },
    ProviderCatalogItem {
        id: "xai",
        alias: "xai",
        name: "xAI",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.x.ai/v1/chat/completions",
        api_key_hint: "xAI API key",
        website: "https://console.x.ai",
        required: false,
    },
    ProviderCatalogItem {
        id: "mistral",
        alias: "mistral",
        name: "Mistral",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.mistral.ai/v1/chat/completions",
        api_key_hint: "Mistral API key",
        website: "https://console.mistral.ai",
        required: false,
    },
    ProviderCatalogItem {
        id: "perplexity",
        alias: "pplx",
        name: "Perplexity",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.perplexity.ai/chat/completions",
        api_key_hint: "Perplexity API key",
        website: "https://www.perplexity.ai/settings/api",
        required: false,
    },
    ProviderCatalogItem {
        id: "together",
        alias: "together",
        name: "Together AI",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.together.xyz/v1/chat/completions",
        api_key_hint: "Together API key",
        website: "https://api.together.xyz",
        required: false,
    },
    ProviderCatalogItem {
        id: "fireworks",
        alias: "fireworks",
        name: "Fireworks",
        category: "api-key",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://api.fireworks.ai/inference/v1/chat/completions",
        api_key_hint: "Fireworks API key",
        website: "https://fireworks.ai",
        required: false,
    },
    ProviderCatalogItem {
        id: "nvidia",
        alias: "nvidia",
        name: "NVIDIA NIM",
        category: "free-tier",
        auth_type: "api-key",
        format: "openai",
        base_url: "https://integrate.api.nvidia.com/v1/chat/completions",
        api_key_hint: "NVIDIA API key",
        website: "https://build.nvidia.com",
        required: false,
    },
    ProviderCatalogItem {
        id: "github",
        alias: "gh",
        name: "GitHub Copilot",
        category: "subscription",
        auth_type: "oauth",
        format: "openai",
        base_url: "https://api.githubcopilot.com/chat/completions",
        api_key_hint: "OAuth access token",
        website: "https://github.com/features/copilot",
        required: false,
    },
    ProviderCatalogItem {
        id: "codex",
        alias: "cx",
        name: "Codex",
        category: "subscription",
        auth_type: "oauth",
        format: "openai-responses",
        base_url: "https://chatgpt.com/backend-api/codex/responses",
        api_key_hint: "OAuth access token",
        website: "https://chatgpt.com",
        required: false,
    },
    ProviderCatalogItem {
        id: "cursor",
        alias: "cu",
        name: "Cursor",
        category: "subscription",
        auth_type: "oauth",
        format: "cursor",
        base_url: "https://api2.cursor.sh",
        api_key_hint: "Cursor session token",
        website: "https://cursor.com",
        required: false,
    },
    ProviderCatalogItem {
        id: "kiro",
        alias: "kr",
        name: "Kiro",
        category: "subscription",
        auth_type: "oauth",
        format: "kiro",
        base_url: "https://codewhisperer.us-east-1.amazonaws.com/generateAssistantResponse",
        api_key_hint: "Kiro OAuth token",
        website: "https://kiro.dev",
        required: false,
    },
    ProviderCatalogItem {
        id: "opencode",
        alias: "oc",
        name: "OpenCode Free",
        category: "local",
        auth_type: "none",
        format: "openai",
        base_url: "http://localhost:4096/v1/chat/completions",
        api_key_hint: "No auth",
        website: "https://opencode.ai",
        required: false,
    },
    ProviderCatalogItem {
        id: "ollama-local",
        alias: "ollama-local",
        name: "Ollama Local",
        category: "local",
        auth_type: "none",
        format: "openai",
        base_url: "http://localhost:11434/v1/chat/completions",
        api_key_hint: "No auth",
        website: "https://ollama.com",
        required: false,
    },
];

#[derive(Serialize)]
struct ModelListResponse {
    object: &'static str,
    data: Vec<ModelListItem>,
}

async fn v1_models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ModelListResponse>, ApiErrorResponse> {
    authorize_v1(&state, &headers)?;

    let data = state
        .storage
        .list_model_shell_items()
        .map_err(|error| ApiErrorResponse::internal(format!("model listing failed: {error}")))?;

    Ok(Json(ModelListResponse {
        object: "list",
        data,
    }))
}

async fn chat_completions_shell(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiErrorResponse> {
    authorize_v1(&state, &headers)?;
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiErrorResponse::bad_request("missing model"))?;

    let route = state
        .storage
        .resolve_model_route(model)
        .map_err(|error| storage_error_to_api(error, "model route resolution failed"))?;

    proxy_chat_completions(&state, body, route).await
}

fn format_model_ref(model: &ModelRef) -> String {
    format!("{}/{}", model.provider, model.model)
}

/// Parse token usage from upstream response bytes (before translation)
fn parse_token_usage(bytes: &[u8], upstream_format: WireFormat) -> (u64, u64) {
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(_) => return (0, 0),
    };

    match upstream_format {
        WireFormat::ClaudeMessages => {
            let prompt = value
                .get("usage")
                .and_then(|u| u.get("input_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let completion = value
                .get("usage")
                .and_then(|u| u.get("output_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            (prompt, completion)
        }
        WireFormat::Gemini => {
            let prompt = value
                .get("usageMetadata")
                .and_then(|m| m.get("promptTokenCount"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let completion = value
                .get("usageMetadata")
                .and_then(|m| m.get("candidatesTokenCount"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            (prompt, completion)
        }
        // OpenAI and OpenAI-compatible formats
        _ => {
            let prompt = value
                .get("usage")
                .and_then(|u| u.get("prompt_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let completion = value
                .get("usage")
                .and_then(|u| u.get("completion_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            (prompt, completion)
        }
    }
}

/// Calculate cost based on provider, model, and token usage (Phase 3.1)
/// Prices are per 1M tokens (USD). Returns cost in USD.
fn calculate_cost(provider: &str, model: &str, prompt_tokens: u64, completion_tokens: u64) -> f64 {
    let (prompt_per_m, completion_per_m) = price_per_million(provider, model);
    (prompt_tokens as f64 / 1_000_000.0) * prompt_per_m
        + (completion_tokens as f64 / 1_000_000.0) * completion_per_m
}

/// Price per 1M tokens (prompt, completion) for known providers/models
/// Unknown models default to $0 (cost tracking disabled)
fn price_per_million(provider: &str, model: &str) -> (f64, f64) {
    let p = provider.to_ascii_lowercase();
    let m = model.to_ascii_lowercase();

    // OpenAI models
    if p == "openai" || p == "openrouter" {
        if m.contains("gpt-4o-mini") {
            return (0.15, 0.60);
        }
        if m.contains("gpt-4o") {
            return (2.50, 10.00);
        }
        if m.contains("gpt-4.1-mini") {
            return (0.40, 1.60);
        }
        if m.contains("gpt-4.1") {
            return (2.00, 8.00);
        }
        if m.contains("gpt-4-turbo") {
            return (10.00, 30.00);
        }
        if m.contains("gpt-4") {
            return (30.00, 60.00);
        }
        if m.contains("gpt-3.5") {
            return (0.50, 1.50);
        }
        if m.contains("o1-mini") {
            return (3.00, 12.00);
        }
        if m.contains("o1") {
            return (15.00, 60.00);
        }
        if m.contains("o3-mini") {
            return (3.00, 12.00);
        }
        if m.contains("o3") {
            return (15.00, 60.00);
        }
        if m.contains("o4-mini") {
            return (1.10, 4.40);
        }
    }

    // Claude models
    if p == "claude" || p == "anthropic" {
        if m.contains("claude-3-5-haiku") || m.contains("claude-3.5-haiku") {
            return (0.80, 4.00);
        }
        if m.contains("claude-3-5-sonnet") || m.contains("claude-3.5-sonnet") {
            return (3.00, 15.00);
        }
        if m.contains("claude-3-opus") || m.contains("claude-3.3-opus") {
            return (15.00, 75.00);
        }
        if m.contains("claude-3-haiku") {
            return (0.25, 1.25);
        }
        if m.contains("claude-3-sonnet") {
            return (3.00, 15.00);
        }
        if m.contains("claude-sonnet-4") {
            return (3.00, 15.00);
        }
        if m.contains("claude-opus-4") {
            return (15.00, 75.00);
        }
    }

    // Gemini models
    if p == "gemini" || p == "google" {
        if m.contains("gemini-2.0-flash-lite") {
            return (0.075, 0.30);
        }
        if m.contains("gemini-2.0-flash") || m.contains("gemini-2.5-flash") {
            return (0.10, 0.40);
        }
        if m.contains("gemini-1.5-flash") {
            return (0.075, 0.30);
        }
        if m.contains("gemini-1.5-pro") || m.contains("gemini-2.5-pro") {
            return (1.25, 5.00);
        }
    }

    // DeepSeek
    if p == "deepseek" {
        if m.contains("deepseek-r1") {
            return (0.55, 2.19);
        }
        if m.contains("deepseek-v3") || m.contains("deepseek-chat") {
            return (0.14, 0.28);
        }
    }

    // Groq — very cheap, mostly free tier
    if p == "groq" {
        return (0.05, 0.08);
    }

    // Mistral
    if p == "mistral" {
        if m.contains("mistral-large") {
            return (2.00, 6.00);
        }
        if m.contains("mistral-small") {
            return (0.20, 0.60);
        }
        return (0.25, 0.25);
    }

    // Default: no cost tracking for unknown providers/models
    (0.0, 0.0)
}

/// Record usage to storage asynchronously (non-blocking, fire-and-forget)
fn record_usage_async(
    storage: &Storage,
    provider: &str,
    model: &str,
    endpoint: &str,
    prompt_tokens: u64,
    completion_tokens: u64,
    status: &str,
) {
    let cost = calculate_cost(provider, model, prompt_tokens, completion_tokens);
    let entry = UsageEntry {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: blackrouter_common::unix_timestamp().to_string(),
        provider: provider.to_string(),
        model: model.to_string(),
        connection_id: None,
        api_key: None,
        endpoint: endpoint.to_string(),
        prompt_tokens,
        completion_tokens,
        cost,
        status: status.to_string(),
        tokens: None,
        meta: None,
    };

    let storage = storage.clone();
    tokio::spawn(async move {
        if let Err(e) = storage.record_usage(&entry) {
            tracing::warn!("failed to record usage: {e}");
        }
    });
}

/// Record request details asynchronously
fn record_request_details_async(
    storage: &Storage,
    provider: &str,
    model: &str,
    status: &str,
    data: Value,
) {
    let entry = RequestDetailEntry {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: blackrouter_common::unix_timestamp().to_string(),
        provider: provider.to_string(),
        model: model.to_string(),
        connection_id: None,
        status: status.to_string(),
        data: serde_json::to_string(&data).unwrap_or_default(),
    };

    let storage = storage.clone();
    tokio::spawn(async move {
        if let Err(e) = storage.record_request_details(&entry) {
            tracing::warn!("failed to record request details: {e}");
        }
    });
}

async fn proxy_chat_completions(
    state: &AppState,
    body: Value,
    route: RouteKind,
) -> Result<Response, ApiErrorResponse> {
    let is_stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);

    let models = match route {
        RouteKind::Single(model) => vec![model],
        RouteKind::Combo { models, .. } => models,
    };

    let mut last_error = None;
    for model in models {
        match proxy_single_chat_completion(state, &body, &model, is_stream).await {
            Ok(response) => return Ok(response),
            Err(error) => {
                last_error = Some(format!(
                    "{} failed: {}",
                    format_model_ref(&model),
                    error.message
                ));
            }
        }
    }

    Err(ApiErrorResponse::new(
        StatusCode::BAD_GATEWAY,
        last_error.unwrap_or_else(|| "All routed models failed".to_string()),
        "provider_error",
    ))
}

async fn proxy_single_chat_completion(
    state: &AppState,
    body: &Value,
    model: &ModelRef,
    is_stream: bool,
) -> Result<Response, ApiErrorResponse> {
    let provider = state
        .storage
        .get_active_provider_connection_raw(&model.provider)
        .map_err(|error| storage_error_to_api(error, "provider lookup failed"))?;

    let format = provider
        .data
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("openai")
        .to_ascii_lowercase();

    // Determine target wire format
    let target_format = match format.as_str() {
        "openai" | "openai-chat" => WireFormat::OpenAiChat,
        "openai-responses" => WireFormat::OpenAiResponses,
        "claude" | "claude-messages" => WireFormat::ClaudeMessages,
        "gemini" => WireFormat::Gemini,
        "gemini-cli" => WireFormat::GeminiCli,
        "kiro" => WireFormat::Kiro,
        "antigravity" => WireFormat::Antigravity,
        "commandcode" => WireFormat::CommandCode,
        "cursor" => WireFormat::Cursor,
        _ => {
            return Err(ApiErrorResponse::new(
                StatusCode::BAD_GATEWAY,
                format!("unsupported provider format: {format}"),
                "provider_error",
            ));
        }
    };

    // Translate request from OpenAI format to target format
    let translated_body = if target_format != WireFormat::OpenAiChat {
        translate_request(body, WireFormat::OpenAiChat, target_format).map_err(|error| {
            ApiErrorResponse::new(
                StatusCode::BAD_REQUEST,
                format!("request translation failed: {error}"),
                "translation_error",
            )
        })?
    } else {
        body.clone()
    };

    let base_url = provider
        .data
        .get("baseUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiErrorResponse::bad_request("Missing provider data.baseUrl"))?;

    let mut upstream_body = translated_body;
    if let Some(object) = upstream_body.as_object_mut() {
        object.insert("model".to_string(), Value::String(model.model.clone()));
    }

    // For translated streaming (OpenAI→Claude/Gemini with stream=true),
    // keep stream flag and use SSE event-by-event translation (Phase 2.3).
    let passthrough_stream = is_stream && target_format == WireFormat::OpenAiChat;
    // No longer strip stream flag — upstream gets stream=true and we translate SSE events

    // Use shared HTTP client (Phase 1.2 — connection pooling)
    let mut request = state.http_client.post(base_url).json(&upstream_body);
    if let Some(headers) = provider.data.get("headers").and_then(Value::as_object) {
        for (key, value) in headers {
            if let Some(value) = value.as_str() {
                request = request.header(key, value);
            }
        }
    }
    if let Some(token) = provider_token(&provider.data) {
        request = request.bearer_auth(token);
    }

    let request_key = RequestKey {
        provider: model.provider.clone(),
        model: model.model.clone(),
        api_key: None,
    };

    // Check rate limit
    if !state.rtk.check_rate_limit(&request_key).await {
        return Err(ApiErrorResponse::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded",
            "rate_limit_error",
        ));
    }

    // Record request start
    state.rtk.record_request_start(&request_key).await;
    let start_time = std::time::Instant::now();

    // Send request
    let response = match request.send().await {
        Ok(r) => r,
        Err(error) => {
            state
                .rtk
                .record_request_end(&request_key, false, start_time.elapsed(), 0, 0, 0.0)
                .await;
            return Err(ApiErrorResponse::new(
                StatusCode::BAD_GATEWAY,
                format!("provider request failed: {error}"),
                "provider_error",
            ));
        }
    };

    let status = response.status();

    // Non-success: record failure and return error (allows combo fallback)
    if !status.is_success() {
        state
            .rtk
            .record_request_end(&request_key, false, start_time.elapsed(), 0, 0, 0.0)
            .await;
        return Err(ApiErrorResponse::new(
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            format!(
                "{} returned HTTP {}",
                format_model_ref(model),
                status.as_u16()
            ),
            "provider_error",
        ));
    }

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);

    // ============================================================
    // Streaming path: passthrough (OpenAI → OpenAI) — Phase 1.1
    // ============================================================
    if passthrough_stream {
        // Record request end (optimistic — upstream returned 200)
        state
            .rtk
            .record_request_end(&request_key, true, start_time.elapsed(), 0, 0, 0.0)
            .await;

        // Zero-copy: forward bytes_stream directly to client
        let stream = response.bytes_stream();
        let body = Body::from_stream(stream);

        return Response::builder()
            .status(status)
            .header(
                header::CONTENT_TYPE,
                content_type.unwrap_or_else(|| "text/event-stream".to_string()),
            )
            .header("Cache-Control", "no-cache")
            .header("Connection", "keep-alive")
            .body(body)
            .map_err(|error| {
                ApiErrorResponse::internal(format!("failed to build stream response: {error}"))
            });
    }

    // ============================================================
    // Translated streaming path (OpenAI → Claude/Gemini) — Phase 2.3
    // SSE event-by-event translation in real-time
    // ============================================================
    if is_stream && !passthrough_stream {
        // Record request end (optimistic — tokens tracked via SSE events)
        state
            .rtk
            .record_request_end(&request_key, true, start_time.elapsed(), 0, 0, 0.0)
            .await;

        let upstream_stream = response.bytes_stream();
        let translated = translate_sse_stream(upstream_stream, target_format, model.model.clone());
        let body = Body::from_stream(translated);

        return Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header("Cache-Control", "no-cache")
            .header("Connection", "keep-alive")
            .body(body)
            .map_err(|error| {
                ApiErrorResponse::internal(format!(
                    "failed to build translated stream response: {error}"
                ))
            });
    }

    // ============================================================
    // Non-streaming path — read full response, translate, return
    // ============================================================
    let bytes = match response.bytes().await {
        Ok(b) => b,
        Err(error) => {
            state
                .rtk
                .record_request_end(&request_key, false, start_time.elapsed(), 0, 0, 0.0)
                .await;
            return Err(ApiErrorResponse::new(
                StatusCode::BAD_GATEWAY,
                format!("provider response read failed: {error}"),
                "provider_error",
            ));
        }
    };

    // Parse token usage from upstream response (Phase 3.1)
    let (prompt_tokens, completion_tokens) = parse_token_usage(&bytes, target_format);

    // Cost calculation (Phase 3.1)
    let cost = calculate_cost(
        &model.provider,
        &model.model,
        prompt_tokens,
        completion_tokens,
    );

    state
        .rtk
        .record_request_end(
            &request_key,
            true,
            start_time.elapsed(),
            prompt_tokens,
            completion_tokens,
            cost,
        )
        .await;

    // Record to usage storage (async, non-blocking)
    record_usage_async(
        &state.storage,
        &model.provider,
        &model.model,
        "/v1/chat/completions",
        prompt_tokens,
        completion_tokens,
        "success",
    );

    // Record request details (async, non-blocking)
    record_request_details_async(
        &state.storage,
        &model.provider,
        &model.model,
        "success",
        serde_json::json!({
            "endpoint": "/v1/chat/completions",
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "cost": cost,
            "latency_ms": start_time.elapsed().as_millis(),
        }),
    );

    // Translate response back to OpenAI format if needed
    let response_body = if target_format != WireFormat::OpenAiChat {
        let response_value: Value = serde_json::from_slice(&bytes).unwrap_or_else(
            |_| serde_json::json!({"raw": String::from_utf8_lossy(&bytes).to_string()}),
        );

        match translate_response(&response_value, target_format, WireFormat::OpenAiChat) {
            Ok(translated) => serde_json::to_vec(&translated).unwrap_or_else(|_| bytes.to_vec()),
            Err(_) => bytes.to_vec(),
        }
    } else {
        bytes.to_vec()
    };

    let mut builder = Response::builder().status(status);
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    builder.body(Body::from(response_body)).map_err(|error| {
        ApiErrorResponse::internal(format!("failed to build provider response: {error}"))
    })
}

#[derive(Serialize)]
struct RtkMetricsResponse {
    metrics: blackrouter_rtk::RequestMetrics,
    uptime_seconds: u64,
}

async fn rtk_metrics(State(state): State<AppState>) -> Json<RtkMetricsResponse> {
    let metrics = state.rtk.metrics().await;
    let uptime = state.rtk.uptime().as_secs();

    Json(RtkMetricsResponse {
        metrics,
        uptime_seconds: uptime,
    })
}

#[derive(Serialize)]
struct RtkStatusResponse {
    provider: String,
    model: String,
    rate_limit: blackrouter_rtk::RateLimitStatus,
}

async fn rtk_status(
    State(state): State<AppState>,
    Path((provider, model)): Path<(String, String)>,
) -> Json<RtkStatusResponse> {
    let key = RequestKey {
        provider,
        model,
        api_key: None,
    };

    let status = state.rtk.rate_limit_status(&key).await;

    Json(RtkStatusResponse {
        provider: key.provider,
        model: key.model,
        rate_limit: status,
    })
}

#[derive(Serialize)]
struct UsageStatsResponse {
    rows: Vec<blackrouter_storage::UsageRow>,
}

async fn usage_stats(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<UsageStatsResponse>, ApiErrorResponse> {
    let since = params.get("since").map(|s| s.as_str());
    let rows = state
        .storage
        .usage_stats(since)
        .map_err(|e| ApiErrorResponse::internal(format!("usage stats failed: {e}")))?;
    Ok(Json(UsageStatsResponse { rows }))
}

#[derive(Serialize)]
struct DailyUsageListResponse {
    days: Vec<DailyUsage>,
}

async fn list_daily_usage(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<DailyUsageListResponse>, ApiErrorResponse> {
    let limit: u32 = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let days = state
        .storage
        .list_daily_usage(limit)
        .map_err(|e| ApiErrorResponse::internal(format!("list daily usage failed: {e}")))?;
    Ok(Json(DailyUsageListResponse { days }))
}

async fn get_daily_usage(
    State(state): State<AppState>,
    Path(date): Path<String>,
) -> Result<Json<Option<DailyUsage>>, ApiErrorResponse> {
    let daily = state
        .storage
        .get_daily_usage(&date)
        .map_err(|e| ApiErrorResponse::internal(format!("get daily usage failed: {e}")))?;
    Ok(Json(daily))
}

#[derive(Deserialize)]
struct AggregateRequest {
    date: Option<String>,
}

async fn aggregate_daily(
    State(state): State<AppState>,
    Json(req): Json<AggregateRequest>,
) -> Result<Json<DailyUsage>, ApiErrorResponse> {
    let date = req.date.unwrap_or_else(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Simple date format from unix timestamp
        let dt = std::time::UNIX_EPOCH + std::time::Duration::from_secs(now);
        format_date(dt)
    });
    let daily = state
        .storage
        .aggregate_daily_usage(&date)
        .map_err(|e| ApiErrorResponse::internal(format!("daily aggregation failed: {e}")))?;
    Ok(Json(daily))
}

/// Format a SystemTime as YYYY-MM-DD (UTC)
fn format_date(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Simple conversion: days since epoch → date
    let days = secs / 86400;
    let remaining = secs % 86400;
    let _ = remaining;
    // Use a simple algorithm for date calculation
    let (year, month, day) = days_to_date(days as i64);
    format!("{:04}-{:02}-{:02}", year, month, day)
}

/// Convert days since epoch (1970-01-01) to (year, month, day)
fn days_to_date(days: i64) -> (i64, u32, u32) {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

async fn responses_proxy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiErrorResponse> {
    authorize_v1(&state, &headers)?;

    let model = body
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiErrorResponse::bad_request("missing model"))?;

    // Convert Responses API request → Chat Completions request
    let chat_body = responses_request_to_chat(&body).map_err(|error| {
        ApiErrorResponse::bad_request(format!("responses conversion failed: {error}"))
    })?;

    let route = state
        .storage
        .resolve_model_route(model)
        .map_err(|error| storage_error_to_api(error, "model route resolution failed"))?;

    // Proxy through chat completions (reuse all streaming/translation/pooling logic)
    let response = proxy_chat_completions(&state, chat_body, route).await?;

    // For non-streaming responses, convert chat completion → Responses API format
    let is_stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    if is_stream {
        // Streaming: return SSE as-is (chat completion SSE is compatible enough)
        return Ok(response);
    }

    // Non-streaming: read response body, convert to Responses format
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(ToOwned::to_owned);
    let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .map_err(|e| ApiErrorResponse::internal(format!("failed to read response body: {e}")))?;

    let chat_response: Value = serde_json::from_slice(&bytes)
        .map_err(|e| ApiErrorResponse::internal(format!("failed to parse chat response: {e}")))?;

    let responses_body = chat_response_to_responses(&chat_response, model).map_err(|error| {
        ApiErrorResponse::internal(format!("responses conversion failed: {error}"))
    })?;

    let body_bytes = serde_json::to_vec(&responses_body).unwrap_or_else(|_| bytes.to_vec());

    let mut builder = Response::builder().status(status);
    builder = builder.header(header::CONTENT_TYPE, "application/json");
    let _ = content_type;
    builder.body(Body::from(body_bytes)).map_err(|error| {
        ApiErrorResponse::internal(format!("failed to build responses response: {error}"))
    })
}

async fn messages_proxy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiErrorResponse> {
    authorize_v1(&state, &headers)?;

    let model = body
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiErrorResponse::bad_request("missing model"))?;

    let route = state
        .storage
        .resolve_model_route(model)
        .map_err(|error| storage_error_to_api(error, "model route resolution failed"))?;

    // Check if target provider is Claude — if so, passthrough directly
    let models = match &route {
        RouteKind::Single(m) => vec![m.clone()],
        RouteKind::Combo { models, .. } => models.clone(),
    };

    let first_model = models
        .first()
        .ok_or_else(|| ApiErrorResponse::bad_request("no models in route"))?;

    let provider = state
        .storage
        .get_active_provider_connection_raw(&first_model.provider)
        .map_err(|error| storage_error_to_api(error, "provider lookup failed"))?;

    let format = provider
        .data
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("openai")
        .to_ascii_lowercase();

    let is_claude = format == "claude" || format == "claude-messages";

    if is_claude {
        // Claude → Claude: passthrough using existing proxy with Claude input format
        let is_stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);

        let mut upstream_body = body.clone();
        if let Some(object) = upstream_body.as_object_mut() {
            object.insert(
                "model".to_string(),
                Value::String(first_model.model.clone()),
            );
        }

        let base_url = provider
            .data
            .get("baseUrl")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| ApiErrorResponse::bad_request("Missing provider data.baseUrl"))?;

        let mut request = state.http_client.post(base_url).json(&upstream_body);
        if let Some(hdrs) = provider.data.get("headers").and_then(Value::as_object) {
            for (key, value) in hdrs {
                if let Some(value) = value.as_str() {
                    request = request.header(key, value);
                }
            }
        }
        if let Some(token) = provider_token(&provider.data) {
            request = request.bearer_auth(token);
        }

        let request_key = RequestKey {
            provider: first_model.provider.clone(),
            model: first_model.model.clone(),
            api_key: None,
        };

        if !state.rtk.check_rate_limit(&request_key).await {
            return Err(ApiErrorResponse::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded",
                "rate_limit_error",
            ));
        }

        state.rtk.record_request_start(&request_key).await;
        let start_time = std::time::Instant::now();

        let upstream_response = match request.send().await {
            Ok(r) => r,
            Err(error) => {
                state
                    .rtk
                    .record_request_end(&request_key, false, start_time.elapsed(), 0, 0, 0.0)
                    .await;
                return Err(ApiErrorResponse::new(
                    StatusCode::BAD_GATEWAY,
                    format!("provider request failed: {error}"),
                    "provider_error",
                ));
            }
        };

        let status = upstream_response.status();
        if !status.is_success() {
            state
                .rtk
                .record_request_end(&request_key, false, start_time.elapsed(), 0, 0, 0.0)
                .await;
            return Err(ApiErrorResponse::new(
                StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                format!("provider returned HTTP {}", status.as_u16()),
                "provider_error",
            ));
        }

        let content_type = upstream_response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);

        // Streaming passthrough
        if is_stream {
            state
                .rtk
                .record_request_end(&request_key, true, start_time.elapsed(), 0, 0, 0.0)
                .await;
            let stream = upstream_response.bytes_stream();
            let body = Body::from_stream(stream);
            return Response::builder()
                .status(status)
                .header(
                    header::CONTENT_TYPE,
                    content_type.unwrap_or_else(|| "text/event-stream".to_string()),
                )
                .header("Cache-Control", "no-cache")
                .header("Connection", "keep-alive")
                .body(body)
                .map_err(|error| {
                    ApiErrorResponse::internal(format!("failed to build stream response: {error}"))
                });
        }

        // Non-streaming passthrough
        let bytes = match upstream_response.bytes().await {
            Ok(b) => b,
            Err(error) => {
                state
                    .rtk
                    .record_request_end(&request_key, false, start_time.elapsed(), 0, 0, 0.0)
                    .await;
                return Err(ApiErrorResponse::new(
                    StatusCode::BAD_GATEWAY,
                    format!("provider response read failed: {error}"),
                    "provider_error",
                ));
            }
        };

        let (prompt_tokens, completion_tokens) =
            parse_token_usage(&bytes, WireFormat::ClaudeMessages);
        state
            .rtk
            .record_request_end(
                &request_key,
                true,
                start_time.elapsed(),
                prompt_tokens,
                completion_tokens,
                0.0,
            )
            .await;

        // Record to usage storage (async, non-blocking)
        record_usage_async(
            &state.storage,
            &first_model.provider,
            &first_model.model,
            "/v1/messages",
            prompt_tokens,
            completion_tokens,
            "success",
        );

        return Response::builder()
            .status(status)
            .header(
                header::CONTENT_TYPE,
                content_type.unwrap_or_else(|| "application/json".to_string()),
            )
            .body(Body::from(bytes))
            .map_err(|error| {
                ApiErrorResponse::internal(format!("failed to build response: {error}"))
            });
    }

    // Non-Claude provider: translate Claude request → OpenAI Chat, proxy, translate back
    let is_stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);

    let chat_body = translate_request(&body, WireFormat::ClaudeMessages, WireFormat::OpenAiChat)
        .map_err(|error| {
            ApiErrorResponse::bad_request(format!("claude→openai translation failed: {error}"))
        })?;

    let chat_response = proxy_chat_completions(&state, chat_body, route).await?;

    // For non-streaming, convert OpenAI response → Claude Messages format
    if is_stream {
        return Ok(chat_response);
    }

    let status = chat_response.status();
    let bytes = axum::body::to_bytes(chat_response.into_body(), 10 * 1024 * 1024)
        .await
        .map_err(|e| ApiErrorResponse::internal(format!("failed to read response: {e}")))?;

    let openai_response: Value = serde_json::from_slice(&bytes)
        .map_err(|e| ApiErrorResponse::internal(format!("failed to parse response: {e}")))?;

    let claude_response = translate_response(
        &openai_response,
        WireFormat::OpenAiChat,
        WireFormat::ClaudeMessages,
    )
    .map_err(|error| {
        ApiErrorResponse::internal(format!(
            "openai→claude response translation failed: {error}"
        ))
    })?;

    let body_bytes = serde_json::to_vec(&claude_response).unwrap_or_else(|_| bytes.to_vec());

    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body_bytes))
        .map_err(|error| {
            ApiErrorResponse::internal(format!("failed to build messages response: {error}"))
        })
}

fn authorize_v1(state: &AppState, headers: &HeaderMap) -> Result<(), ApiErrorResponse> {
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

fn extract_api_key(headers: &HeaderMap) -> Option<String> {
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

#[derive(Debug)]
pub struct ApiErrorResponse {
    status: StatusCode,
    message: String,
    error_type: &'static str,
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

fn storage_error_to_api(error: StorageError, context: &'static str) -> ApiErrorResponse {
    match error {
        StorageError::Validation(message) if message.contains("not found") => {
            ApiErrorResponse::not_found(message)
        }
        StorageError::Validation(message) => ApiErrorResponse::bad_request(message),
        other => ApiErrorResponse::internal(format!("{context}: {other}")),
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

#[cfg(test)]
mod tests {
    use super::{builtin_provider_models, derive_models_url, extract_api_key, extract_model_ids};
    use axum::http::HeaderMap;
    use blackrouter_storage::RawProviderConnection;
    use serde_json::json;

    #[test]
    fn extracts_bearer_api_key() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer sk-test".parse().unwrap());
        assert_eq!(extract_api_key(&headers).as_deref(), Some("sk-test"));
    }

    #[test]
    fn extracts_x_api_key() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "sk-test".parse().unwrap());
        assert_eq!(extract_api_key(&headers).as_deref(), Some("sk-test"));
    }

    #[test]
    fn derives_models_url_from_common_provider_urls() {
        assert_eq!(
            derive_models_url("https://api.openai.com/v1/chat/completions").as_deref(),
            Some("https://api.openai.com/v1/models")
        );
        assert_eq!(
            derive_models_url("https://api.cline.bot/api/v1/chat/completions").as_deref(),
            Some("https://api.cline.bot/api/v1/models")
        );
        assert_eq!(
            derive_models_url("https://api.commandcode.ai/alpha/generate").as_deref(),
            Some("https://api.commandcode.ai/alpha/models")
        );
        assert_eq!(
            derive_models_url("https://generativelanguage.googleapis.com/v1beta/models").as_deref(),
            Some("https://generativelanguage.googleapis.com/v1beta/models")
        );
    }

    #[test]
    fn extracts_model_ids_from_common_payload_shapes() {
        assert_eq!(
            extract_model_ids(&json!({
                "data": [{"id": "gpt-4.1"}, {"id": "gpt-4.1"}, {"name": "fallback-name"}]
            })),
            vec!["fallback-name".to_string(), "gpt-4.1".to_string()]
        );
        assert_eq!(
            extract_model_ids(&json!({
                "models": [{"name": "models/gemini-3-pro-preview"}, "plain-model"]
            })),
            vec![
                "gemini-3-pro-preview".to_string(),
                "plain-model".to_string()
            ]
        );
    }

    #[test]
    fn finds_builtin_provider_model_catalogs() {
        let cline = RawProviderConnection {
            id: "1".to_string(),
            provider: "cline".to_string(),
            auth_type: "api-key".to_string(),
            name: None,
            email: None,
            priority: None,
            is_active: true,
            data: json!({ "alias": "cl" }),
            created_at: "1".to_string(),
            updated_at: "1".to_string(),
        };
        assert!(builtin_provider_models(&cline)
            .unwrap()
            .models
            .contains(&"anthropic/claude-sonnet-4.6"));

        let commandcode = RawProviderConnection {
            provider: "custom".to_string(),
            data: json!({ "alias": "cmc" }),
            ..cline
        };
        assert!(builtin_provider_models(&commandcode)
            .unwrap()
            .models
            .contains(&"claude-sonnet-4-6"));
    }
}
