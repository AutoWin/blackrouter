use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use blackrouter_common::{unix_timestamp, BuildInfo};
use blackrouter_config::AppConfig;
use blackrouter_core::{ModelRef, RouteKind};
use blackrouter_storage::{
    ApiKeyRecord, ComboRecord, CreatedApiKey, ModelListItem, NewApiKey, NewCombo,
    NewProviderConnection, ProviderConnectionRecord, RawProviderConnection, Storage, StorageError,
    StorageStatus,
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
}

impl AppState {
    pub fn new(config: AppConfig, storage: Storage) -> Self {
        Self {
            config,
            storage,
            started_at_unix: unix_timestamp(),
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
        .route("/v1/responses", post(responses_shell))
        .route("/v1/messages", post(messages_shell))
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

async fn proxy_chat_completions(
    state: &AppState,
    body: Value,
    route: RouteKind,
) -> Result<Response, ApiErrorResponse> {
    let models = match route {
        RouteKind::Single(model) => vec![model],
        RouteKind::Combo { models, .. } => models,
    };

    let mut last_error = None;
    for model in models {
        match proxy_single_chat_completion(state, &body, &model).await {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => {
                let status = response.status();
                last_error = Some(format!(
                    "{} returned HTTP {}",
                    format_model_ref(&model),
                    status.as_u16()
                ));
            }
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
    if !format.contains("openai") {
        return Err(ApiErrorResponse::new(
            StatusCode::BAD_GATEWAY,
            format!("provider format is not implemented yet: {format}"),
            "provider_error",
        ));
    }

    let base_url = provider
        .data
        .get("baseUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiErrorResponse::bad_request("Missing provider data.baseUrl"))?;

    let mut upstream_body = body.clone();
    if let Some(object) = upstream_body.as_object_mut() {
        object.insert("model".to_string(), Value::String(model.model.clone()));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|error| {
            ApiErrorResponse::internal(format!("Failed to create HTTP client: {error}"))
        })?;

    let mut request = client.post(base_url).json(&upstream_body);
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

    let response = request.send().await.map_err(|error| {
        ApiErrorResponse::new(
            StatusCode::BAD_GATEWAY,
            format!("provider request failed: {error}"),
            "provider_error",
        )
    })?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let bytes = response.bytes().await.map_err(|error| {
        ApiErrorResponse::new(
            StatusCode::BAD_GATEWAY,
            format!("provider response read failed: {error}"),
            "provider_error",
        )
    })?;

    let mut builder = Response::builder().status(status);
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    builder.body(Body::from(bytes)).map_err(|error| {
        ApiErrorResponse::internal(format!("failed to build provider response: {error}"))
    })
}

async fn responses_shell(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> Result<Response, ApiErrorResponse> {
    authorize_v1(&state, &headers)?;
    Err(ApiErrorResponse::new(
        StatusCode::NOT_IMPLEMENTED,
        "BlackRouter Rust /v1/responses shell is ready, but response routing is not implemented yet",
        "not_implemented",
    ))
}

async fn messages_shell(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(_body): Json<Value>,
) -> Result<Response, ApiErrorResponse> {
    authorize_v1(&state, &headers)?;
    Err(ApiErrorResponse::new(
        StatusCode::NOT_IMPLEMENTED,
        "BlackRouter Rust /v1/messages shell is ready, but message routing is not implemented yet",
        "not_implemented",
    ))
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
