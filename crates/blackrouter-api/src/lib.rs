use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use base64::Engine;
use blackrouter_common::{unix_timestamp, BuildInfo};
use blackrouter_config::AppConfig;
use blackrouter_core::{ModelRef, RouteKind};
use blackrouter_providers::{
    builtin_provider_models as registry_builtin_provider_models, provider_profiles,
    BuiltinProviderModels, ProviderProfile,
};
use blackrouter_rtk::{LoadBalanceStrategy, RateLimitConfig, RequestKey, ResponseCache, Rtk};
use blackrouter_storage::{
    ApiKeyRecord, CachedProviderModels, ComboRecord, CreatedApiKey, DailyUsage, ModelAliasRecord,
    ModelListItem, NewApiKey, NewCombo, NewModelAlias, NewProviderConnection,
    ProviderConnectionRecord, RawProviderConnection, RequestDetailEntry, Storage, StorageError,
    StorageStatus, UsageEntry, UsageRow,
};
use blackrouter_telegram::{
    TelegramBot, TelegramBotConfig, TelegramRuntime, Update as TelegramUpdate,
};
use blackrouter_translator::{
    chat_response_to_responses, commandcode_stream_text_to_openai, commandcode_stream_token_usage,
    responses_request_to_chat, stream::translate_sse_stream, translate_request, translate_response,
    WireFormat,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use prometheus::{
    register_histogram_vec, register_int_counter_vec, register_int_gauge, Encoder, HistogramVec,
    IntCounterVec, IntGauge, Registry, TextEncoder,
};

mod oauth;

const MAX_REQUEST_BYTES: usize = 50 * 1024 * 1024;
const COMMANDCODE_VERSION: &str = "0.25.7";
const CODEX_MODELS_URL: &str = "https://chatgpt.com/backend-api/codex/models?client_version=1.0.0";
const CODEX_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const CODEX_USER_AGENT: &str = "codex_cli_rs/0.136.0";

#[derive(Clone)]
pub struct Metrics {
    pub registry: Arc<Registry>,
    pub requests_total: IntCounterVec,
    pub request_duration: HistogramVec,
    pub stream_ttfb: HistogramVec,
    pub tokens_total: IntCounterVec,
    pub open_connections: IntGauge,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Arc::new(Registry::new());

        let requests_total = register_int_counter_vec!(
            "blackrouter_requests_total",
            "Total number of requests",
            &["provider", "model", "status"]
        )
        .unwrap();

        let request_duration = register_histogram_vec!(
            "blackrouter_request_duration_seconds",
            "Request duration in seconds",
            &["provider", "model"],
            vec![0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0]
        )
        .unwrap();

        let stream_ttfb = register_histogram_vec!(
            "blackrouter_stream_ttfb_seconds",
            "Time to first byte for streaming requests",
            &["provider", "model"],
            vec![0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0]
        )
        .unwrap();

        let tokens_total = register_int_counter_vec!(
            "blackrouter_tokens_total",
            "Total tokens processed",
            &["provider", "model", "type"]
        )
        .unwrap();

        let open_connections =
            register_int_gauge!("blackrouter_open_connections", "Current open connections")
                .unwrap();

        // Register process metrics
        let process_collector = prometheus::process_collector::ProcessCollector::for_self();
        let _ = prometheus::register(Box::new(process_collector));

        registry.register(Box::new(requests_total.clone())).ok();
        registry.register(Box::new(request_duration.clone())).ok();
        registry.register(Box::new(stream_ttfb.clone())).ok();
        registry.register(Box::new(tokens_total.clone())).ok();
        registry.register(Box::new(open_connections.clone())).ok();

        Self {
            registry,
            requests_total,
            request_duration,
            stream_ttfb,
            tokens_total,
            open_connections,
        }
    }

    pub fn encode(&self) -> String {
        let mut buffer = vec![];
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        encoder.encode(&metric_families, &mut buffer).ok();
        String::from_utf8(buffer).unwrap_or_default()
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub storage: Storage,
    pub started_at_unix: u64,
    pub rtk: Rtk,
    pub http_client: reqwest::Client,
    pub usage_tx: tokio::sync::mpsc::UnboundedSender<UsageEntry>,
    pub metrics: Metrics,
    pub response_cache: ResponseCache,
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

        // Batch usage writer (Phase 1.4): buffer entries, flush every 5s or at 100 entries
        let (usage_tx, mut usage_rx) = tokio::sync::mpsc::unbounded_channel::<UsageEntry>();
        let batch_storage = storage.clone();
        tokio::spawn(async move {
            let mut buffer: Vec<UsageEntry> = Vec::with_capacity(128);
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                tokio::select! {
                    Some(entry) = usage_rx.recv() => {
                        buffer.push(entry);
                        if buffer.len() >= 100 {
                            let batch = std::mem::take(&mut buffer);
                            if let Err(e) = batch_storage.record_usages_batch(&batch) {
                                tracing::warn!("batch usage write failed: {e}");
                            }
                        }
                    }
                    _ = interval.tick() => {
                        if !buffer.is_empty() {
                            let batch = std::mem::take(&mut buffer);
                            if let Err(e) = batch_storage.record_usages_batch(&batch) {
                                tracing::warn!("batch usage flush failed: {e}");
                            }
                        }
                    }
                }
            }
        });

        Self {
            config,
            storage,
            started_at_unix: unix_timestamp(),
            rtk: Rtk::new(rtk_config),
            http_client,
            usage_tx,
            metrics: Metrics::new(),
            response_cache: ResponseCache::new(Duration::from_secs(300), 1000),
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
        .route("/api/setup/api-keys/{id}/rotate", post(rotate_api_key))
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
        .route("/api/doctor", get(doctor))
        .route("/api/setup/combos", get(list_combos).post(create_combo))
        .route(
            "/api/setup/combos/{id}",
            get(get_combo).put(update_combo).delete(delete_combo),
        )
        .route(
            "/api/setup/aliases",
            get(list_model_aliases).post(create_model_alias),
        )
        .route(
            "/api/setup/aliases/{id}",
            put(update_model_alias).delete(delete_model_alias),
        )
        .route(
            "/api/setup/lb-strategy",
            get(get_lb_strategy).put(set_lb_strategy),
        )
        .route("/api/provider-limits", get(provider_limits))
        .route("/v1/models", get(v1_models))
        .route("/v1beta/models", get(v1_models))
        .route("/v1/chat/completions", post(chat_completions_shell))
        .route("/v1/responses", post(responses_proxy))
        .route("/v1/messages", post(messages_proxy))
        .route("/api/rtk/metrics", get(rtk_metrics))
        .route("/api/rtk/status/{provider}/{model}", get(rtk_status))
        .route("/metrics", get(metrics))
        .route("/api/usage", get(usage_stats))
        .route("/api/usage/daily", get(list_daily_usage))
        .route("/api/usage/daily/{date}", get(get_daily_usage))
        .route("/api/usage/aggregate", post(aggregate_daily))
        .route("/telegram/webhook", post(telegram_webhook))
        .route("/api/oauth/{provider}/start", post(oauth::oauth_start))
        .route("/api/oauth/{provider}/callback", get(oauth::oauth_callback))
        .route(
            "/api/oauth/{provider}/exchange",
            post(oauth::oauth_exchange),
        )
        .route("/api/oauth/{provider}/status", get(oauth::oauth_status))
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

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let body = state.metrics.encode();
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body)
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

async fn rotate_api_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<CreatedApiKey>, ApiErrorResponse> {
    let rotated = state
        .storage
        .rotate_api_key(&id)
        .map_err(|error| ApiErrorResponse::not_found(format!("API key not found: {error}")))?;
    Ok(Json(rotated))
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

    Ok(Json(provider_record_from_raw(raw)))
}

fn provider_record_from_raw(raw: RawProviderConnection) -> ProviderConnectionRecord {
    ProviderConnectionRecord {
        id: raw.id,
        provider: raw.provider,
        auth_type: raw.auth_type,
        name: raw.name,
        email: raw.email,
        priority: raw.priority,
        is_active: raw.is_active,
        status: raw.status,
        cooldown_until: raw.cooldown_until,
        expires_at: raw.expires_at,
        data: mask_for_api(raw.data),
        created_at: raw.created_at,
        updated_at: raw.updated_at,
    }
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
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<ProviderModelsResponse>, ApiErrorResponse> {
    let provider = state
        .storage
        .get_provider_connection_raw(&id)
        .map_err(|error| ApiErrorResponse::not_found(format!("{error}")))?;

    let refresh = params
        .get("refresh")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let cache_ttl = model_catalog_cache_ttl_seconds(&state.storage);
    if !refresh {
        if let Some(cached) = state
            .storage
            .cached_provider_models(&id, cache_ttl)
            .map_err(|error| storage_error_to_api(error, "provider model cache lookup failed"))?
        {
            return Ok(Json(provider_models_cache_response(provider, cached)));
        }
    }

    let models_url = match provider_models_url(&provider.data) {
        Ok(url) => Some(url),
        Err(_) => None,
    };
    let (models, resolved_url, message) = if let Some(ref url) = models_url {
        match fetch_provider_model_ids(&provider, url).await {
            Ok(models) => {
                let message = format!("Fetched {} models", models.len());
                let resolved_url = if provider_uses_codex_model_catalog(&provider) {
                    CODEX_MODELS_URL.to_string()
                } else {
                    url.clone()
                };
                (models, resolved_url, message)
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
        }
    } else {
        // Can't derive models URL — fall back to builtin immediately
        let fallback = builtin_provider_models(&provider).ok_or_else(|| {
            ApiErrorResponse::bad_request(
                "Could not derive models URL and no built-in models available",
            )
        })?;
        let models = fallback
            .models
            .iter()
            .map(|model| (*model).to_string())
            .collect::<Vec<_>>();
        let message = format!(
            "Loaded {} built-in {} models (live fetch not available)",
            models.len(),
            fallback.label
        );
        (models, fallback.source.to_string(), message)
    };
    let updated = state
        .storage
        .set_provider_connection_models(&id, models.clone(), Some(resolved_url.clone()))
        .map_err(|error| storage_error_to_api(error, "provider model save failed"))?;

    Ok(Json(ProviderModelsResponse {
        ok: true,
        provider: updated,
        models: models.clone(),
        models_url: resolved_url,
        message,
    }))
}

fn provider_models_cache_response(
    provider: RawProviderConnection,
    cached: CachedProviderModels,
) -> ProviderModelsResponse {
    let models_url = cached
        .models_url
        .unwrap_or_else(|| "cache://provider-models".to_string());
    ProviderModelsResponse {
        ok: true,
        provider: provider_record_from_raw(provider),
        models: cached.models,
        models_url,
        message: format!("Loaded cached model catalog (age {}s)", cached.age_seconds),
    }
}

fn model_catalog_cache_ttl_seconds(storage: &Storage) -> u64 {
    storage
        .settings_json()
        .ok()
        .and_then(|settings| {
            settings
                .get("modelCatalogCacheTtlSeconds")
                .or_else(|| settings.get("model_catalog_cache_ttl_seconds"))
                .and_then(Value::as_u64)
        })
        .unwrap_or(6 * 60 * 60)
}

async fn provider_catalog() -> Json<Vec<ProviderProfile>> {
    Json(provider_profiles().to_vec())
}

#[derive(Serialize)]
struct DoctorResponse {
    ok: bool,
    storage: StorageStatus,
    providers: DoctorProviders,
    model_catalog: DoctorModelCatalog,
    cost_guard: CostGuardStatus,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct DoctorProviders {
    total: usize,
    active: usize,
    available: usize,
    cooldown: usize,
    expired: usize,
}

#[derive(Serialize)]
struct DoctorModelCatalog {
    provider_profile_count: usize,
    remote_cache_ttl_seconds: u64,
    cached_provider_count: usize,
}

async fn doctor(State(state): State<AppState>) -> Result<Json<DoctorResponse>, ApiErrorResponse> {
    let storage = state
        .storage
        .status()
        .map_err(|error| ApiErrorResponse::internal(format!("storage status failed: {error}")))?;
    let providers = state
        .storage
        .list_provider_connections()
        .map_err(|error| storage_error_to_api(error, "provider listing failed"))?;
    let now = blackrouter_common::unix_timestamp();
    let mut provider_status = DoctorProviders {
        total: providers.len(),
        active: 0,
        available: 0,
        cooldown: 0,
        expired: 0,
    };
    let mut cached_provider_count = 0usize;
    let cache_ttl = model_catalog_cache_ttl_seconds(&state.storage);

    for provider in &providers {
        if provider.is_active {
            provider_status.active += 1;
        }
        let cooldown_until = provider
            .cooldown_until
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok());
        let expires_at = provider
            .expires_at
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok());
        let is_cooldown = cooldown_until.map(|value| value > now).unwrap_or(false);
        let is_expired =
            expires_at.map(|value| value <= now).unwrap_or(false) || provider.status == "expired";

        if is_cooldown {
            provider_status.cooldown += 1;
        }
        if is_expired {
            provider_status.expired += 1;
        }
        if provider.is_active
            && !is_cooldown
            && !is_expired
            && provider.status != "disabled"
            && provider.status != "expired"
        {
            provider_status.available += 1;
        }
        if provider
            .data
            .get("modelsFetchedAt")
            .or_else(|| provider.data.get("models_fetched_at"))
            .is_some()
        {
            cached_provider_count += 1;
        }
    }

    let cost_guard = cost_guard_status(&state.storage).map_err(|error| {
        ApiErrorResponse::internal(format!("cost guard status failed: {error}"))
    })?;
    let mut warnings = Vec::new();
    if !storage.schema_compatible {
        warnings.push("storage schema is missing compatibility tables".to_string());
    }
    if provider_status.available == 0 {
        warnings.push("no available provider connections".to_string());
    }
    if cost_guard.enabled && (cost_guard.daily_exceeded || cost_guard.monthly_exceeded) {
        warnings.push("cost guard budget exceeded".to_string());
    }

    Ok(Json(DoctorResponse {
        ok: warnings.is_empty(),
        storage,
        providers: provider_status,
        model_catalog: DoctorModelCatalog {
            provider_profile_count: provider_profiles().len(),
            remote_cache_ttl_seconds: cache_ttl,
            cached_provider_count,
        },
        cost_guard,
        warnings,
    }))
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

// ── Model Aliases (Phase 4.3) ─────────────────────────────────────────────

#[derive(Serialize)]
struct ModelAliasesResponse {
    data: Vec<ModelAliasRecord>,
}

async fn list_model_aliases(
    State(state): State<AppState>,
) -> Result<Json<ModelAliasesResponse>, ApiErrorResponse> {
    let data = state
        .storage
        .list_model_aliases()
        .map_err(|error| storage_error_to_api(error, "alias listing failed"))?;
    Ok(Json(ModelAliasesResponse { data }))
}

async fn create_model_alias(
    State(state): State<AppState>,
    Json(payload): Json<NewModelAlias>,
) -> Result<Json<ModelAliasRecord>, ApiErrorResponse> {
    let created = state
        .storage
        .create_model_alias(payload)
        .map_err(|error| storage_error_to_api(error, "alias creation failed"))?;
    Ok(Json(created))
}

async fn update_model_alias(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<NewModelAlias>,
) -> Result<Json<ModelAliasRecord>, ApiErrorResponse> {
    let updated = state
        .storage
        .update_model_alias(&id, payload)
        .map_err(|error| storage_error_to_api(error, "alias update failed"))?;
    Ok(Json(updated))
}

async fn delete_model_alias(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<DeleteResponse>, ApiErrorResponse> {
    state
        .storage
        .delete_model_alias(&id)
        .map_err(|error| storage_error_to_api(error, "alias delete failed"))?;
    Ok(Json(DeleteResponse { ok: true }))
}

// ── Load Balancing Strategy (Phase 4.1) ───────────────────────────────────

#[derive(Serialize, Deserialize)]
struct LbStrategyResponse {
    strategy: LoadBalanceStrategy,
}

#[derive(Deserialize)]
struct LbStrategyUpdate {
    strategy: LoadBalanceStrategy,
}

async fn get_lb_strategy(State(state): State<AppState>) -> Json<LbStrategyResponse> {
    Json(LbStrategyResponse {
        strategy: state.rtk.lb_strategy().await,
    })
}

async fn set_lb_strategy(
    State(state): State<AppState>,
    Json(payload): Json<LbStrategyUpdate>,
) -> Json<LbStrategyResponse> {
    state.rtk.update_lb_strategy(payload.strategy).await;
    let strategy = state.rtk.lb_strategy().await;
    Json(LbStrategyResponse { strategy })
}

#[derive(Serialize)]
struct ProviderLimitsResponse {
    data: Vec<ProviderLimitRow>,
    usage: Vec<UsageRow>,
    metrics: blackrouter_rtk::RequestMetrics,
    cost_guard: CostGuardStatus,
}

#[derive(Serialize)]
struct ProviderLimitRow {
    id: String,
    provider: String,
    name: Option<String>,
    email: Option<String>,
    is_active: bool,
    status: String,
    model: Option<String>,
    upstream_rate_limit: Option<Value>,
    rtk: Option<ProviderRtkLimit>,
    usage: ProviderLimitUsage,
}

#[derive(Serialize)]
struct ProviderRtkLimit {
    limited: bool,
    requests_remaining: u32,
    tokens_remaining: u64,
    concurrent_remaining: u32,
    retry_after_seconds: Option<u64>,
}

#[derive(Default, Serialize)]
struct ProviderLimitUsage {
    requests: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    cost: f64,
}

async fn provider_limits(
    State(state): State<AppState>,
) -> Result<Json<ProviderLimitsResponse>, ApiErrorResponse> {
    let providers = state
        .storage
        .list_provider_connections()
        .map_err(|error| storage_error_to_api(error, "provider listing failed"))?;
    let usage = state
        .storage
        .usage_stats(None)
        .map_err(|error| storage_error_to_api(error, "usage listing failed"))?;
    let metrics = state.rtk.metrics().await;
    let cost_guard = cost_guard_status(&state.storage)
        .map_err(|error| ApiErrorResponse::internal(format!("cost guard failed: {error}")))?;

    let mut rows = Vec::with_capacity(providers.len());
    for provider in providers {
        let upstream_rate_limit = provider
            .data
            .get("rateLimit")
            .or_else(|| provider.data.get("rate_limit"))
            .cloned();
        let model = upstream_rate_limit
            .as_ref()
            .and_then(|value| value.get("model"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| first_provider_model(&provider.data));

        let rtk = if let Some(model) = &model {
            let status = state
                .rtk
                .rate_limit_status(&RequestKey {
                    provider: provider.provider.clone(),
                    model: model.clone(),
                    api_key: None,
                })
                .await;
            Some(ProviderRtkLimit {
                limited: status.limited,
                requests_remaining: status.requests_remaining,
                tokens_remaining: status.tokens_remaining,
                concurrent_remaining: status.concurrent_remaining,
                retry_after_seconds: status.retry_after.map(|duration| duration.as_secs()),
            })
        } else {
            None
        };

        let mut provider_usage = ProviderLimitUsage::default();
        for row in usage.iter().filter(|row| row.provider == provider.provider) {
            provider_usage.requests += row.count;
            provider_usage.prompt_tokens += row.prompt_tokens;
            provider_usage.completion_tokens += row.completion_tokens;
            provider_usage.cost += row.cost;
        }

        rows.push(ProviderLimitRow {
            id: provider.id,
            provider: provider.provider,
            name: provider.name,
            email: provider.email,
            is_active: provider.is_active,
            status: provider.status,
            model,
            upstream_rate_limit,
            rtk,
            usage: provider_usage,
        });
    }

    Ok(Json(ProviderLimitsResponse {
        data: rows,
        usage,
        metrics,
        cost_guard,
    }))
}

fn first_provider_model(data: &Value) -> Option<String> {
    let models = data.get("models")?.as_array()?;
    models.iter().find_map(|model| {
        if let Some(value) = model
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }
        if let Some(object) = model.as_object() {
            return object
                .get("id")
                .or_else(|| object.get("name"))
                .or_else(|| object.get("model"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
        }
        None
    })
}

async fn check_provider_connection(provider: RawProviderConnection) -> ProviderTestResponse {
    let base_url = provider
        .data
        .get("baseUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let format = provider
        .data
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("openai")
        .to_ascii_lowercase();

    let Some(base_url) = base_url else {
        return ProviderTestResponse {
            ok: false,
            reachable: false,
            status: None,
            url: None,
            message: "Missing data.baseUrl".to_string(),
        };
    };

    // Some providers expose POST-only endpoints; test the actual POST path.
    let test_url = if format == "antigravity" {
        format!(
            "{}/v1internal:generateContent",
            base_url.trim_end_matches('/')
        )
    } else {
        base_url.clone()
    };

    if let Err(message) = validate_provider_auth(&provider) {
        return ProviderTestResponse {
            ok: false,
            reachable: false,
            status: None,
            url: Some(test_url),
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
                url: Some(test_url),
                message: format!("Failed to create HTTP client: {error}"),
            };
        }
    };

    let auth_type = provider.auth_type.clone();

    let build_request = |client: &reqwest::Client, method: &str| {
        let request = if method == "HEAD" {
            client.head(&test_url)
        } else if method == "POST" {
            client
                .post(&test_url)
                .header("Content-Type", "application/json")
                .json(&provider_check_post_body(&format))
        } else {
            client.get(&test_url)
        };
        let request = apply_auth(request, &auth_type, &provider.data);
        if format == "commandcode" {
            apply_commandcode_headers(request)
        } else {
            request
        }
    };

    if provider_check_uses_post(&format) {
        match build_request(&client, "POST").send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                if format == "antigravity" && matches!(status, 400 | 401 | 403) {
                    classify_antigravity_check_response(test_url, status)
                } else {
                    classify_provider_post_response(test_url, status)
                }
            }
            Err(post_error) => ProviderTestResponse {
                ok: false,
                reachable: false,
                status: None,
                url: Some(test_url),
                message: format!("Connection failed: {post_error}"),
            },
        }
    } else {
        match build_request(&client, "HEAD").send().await {
            Ok(response) => classify_provider_response(test_url, response.status().as_u16()),
            Err(head_error) => match build_request(&client, "GET").send().await {
                Ok(response) => classify_provider_response(test_url, response.status().as_u16()),
                Err(get_error) => ProviderTestResponse {
                    ok: false,
                    reachable: false,
                    status: None,
                    url: Some(test_url),
                    message: format!("Connection failed: {get_error}; HEAD error: {head_error}"),
                },
            },
        }
    }
}

fn provider_check_uses_post(format: &str) -> bool {
    matches!(
        format,
        "antigravity"
            | "commandcode"
            | "kiro"
            | "cursor"
            | "openai"
            | "openai-chat"
            | "openai-responses"
    )
}

fn provider_check_post_body(format: &str) -> Value {
    match format {
        "antigravity" => serde_json::json!({"requestType": "agent"}),
        // Intentionally incomplete: this verifies the POST endpoint without asking
        // the provider to generate a completion.
        "commandcode" => serde_json::json!({}),
        "kiro" => serde_json::json!({}),
        "cursor" => serde_json::json!({}),
        "openai" | "openai-chat" | "openai-responses" => serde_json::json!({}),
        _ => serde_json::json!({}),
    }
}

fn classify_antigravity_check_response(url: String, status: u16) -> ProviderTestResponse {
    ProviderTestResponse {
        ok: true,
        reachable: true,
        status: Some(status),
        url: Some(url),
        message: match status {
            401 | 403 => "Endpoint reachable — login required".to_string(),
            _ => format!("Endpoint active (HTTP {}). Login to authenticate.", status),
        },
    }
}

fn classify_provider_post_response(url: String, status: u16) -> ProviderTestResponse {
    if matches!(status, 400 | 422) {
        return ProviderTestResponse {
            ok: true,
            reachable: true,
            status: Some(status),
            url: Some(url),
            message: format!(
                "Provider POST endpoint is reachable; it returned HTTP {status} for the minimal check body"
            ),
        };
    }

    classify_provider_response(url, status)
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
    match auth_type.as_str() {
        "none" => Ok(()),
        "basic" => {
            let has_user = provider
                .data
                .get("username")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .is_some();
            let has_pass = provider
                .data
                .get("password")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .is_some();
            if !has_user || !has_pass {
                Err("Basic auth requires username and password in provider data".to_string())
            } else {
                Ok(())
            }
        }
        "header" => {
            let has_header_name = provider
                .data
                .get("headerName")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .is_some();
            let has_header_value = provider
                .data
                .get("headerValue")
                .and_then(Value::as_str)
                .or_else(|| provider.data.get("apiKey").and_then(Value::as_str))
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .is_some();
            if !has_header_name || !has_header_value {
                Err(
                    "Header auth requires headerName and headerValue (or apiKey) in provider data"
                        .to_string(),
                )
            } else {
                Ok(())
            }
        }
        _ => {
            // api-key, bearer, oauth, and any custom type — require a token
            if provider_token(&provider.data).is_none() {
                Err("Missing API key/access token in provider data".to_string())
            } else {
                Ok(())
            }
        }
    }
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

/// Apply authentication to a request builder based on auth_type and provider data.
fn apply_auth(
    mut request: reqwest::RequestBuilder,
    auth_type: &str,
    data: &Value,
) -> reqwest::RequestBuilder {
    // Always apply custom headers from data.headers (e.g. x-command-code-version)
    if let Some(headers) = data.get("headers").and_then(Value::as_object) {
        for (key, value) in headers {
            if let Some(value) = value.as_str() {
                request = request.header(key, value);
            }
        }
    }

    match auth_type.to_ascii_lowercase().as_str() {
        "none" => {}
        "basic" => {
            if let (Some(username), Some(password)) = (
                data.get("username")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty()),
                data.get("password")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty()),
            ) {
                request = request.basic_auth(username, Some(password));
            }
        }
        "header" => {
            if let (Some(header_name), Some(header_value)) = (
                data.get("headerName")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty()),
                data.get("headerValue")
                    .and_then(Value::as_str)
                    .or_else(|| data.get("apiKey").and_then(Value::as_str))
                    .map(str::trim)
                    .filter(|v| !v.is_empty()),
            ) {
                request = request.header(header_name, header_value);
            }
        }
        _ => {
            // api-key, bearer, oauth
            if let Some(token) = provider_token(data) {
                request = request.bearer_auth(token);
            }
        }
    }

    request
}

fn apply_commandcode_headers(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    request
        .header("x-command-code-version", COMMANDCODE_VERSION)
        .header("x-cli-environment", "cli")
        .header("x-session-id", uuid::Uuid::new_v4().to_string())
        .header(header::ACCEPT, "text/event-stream")
}

fn apply_codex_headers(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    request
        .header("originator", "codex_cli_rs")
        .header("User-Agent", CODEX_USER_AGENT)
        .header(header::ACCEPT, "application/json, text/event-stream")
}

fn sanitize_codex_responses_body(body: &mut Value) {
    if let Some(object) = body.as_object_mut() {
        for key in CODEX_UNSUPPORTED_REQUEST_PARAMS {
            object.remove(*key);
        }
        object.insert("stream".to_string(), Value::Bool(true));
    }
}

const CODEX_UNSUPPORTED_REQUEST_PARAMS: &[&str] = &[
    "max_output_tokens",
    "temperature",
    "top_p",
    "frequency_penalty",
    "presence_penalty",
    "logprobs",
    "top_logprobs",
    "seed",
];

fn codex_stream_to_chat_response(
    bytes: &[u8],
    model: &str,
) -> Result<(Value, u64, u64), ApiErrorResponse> {
    let raw = String::from_utf8_lossy(bytes);
    let mut content = String::new();
    let mut reasoning_content = String::new();
    let mut prompt_tokens = 0;
    let mut completion_tokens = 0;

    for event in raw.split("\n\n") {
        let Some(data) = sse_data(event) else {
            continue;
        };
        if data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "response.output_text.delta" => {
                if let Some(delta) = value
                    .get("delta")
                    .and_then(Value::as_str)
                    .or_else(|| value.get("text").and_then(Value::as_str))
                {
                    content.push_str(delta);
                }
            }
            "response.reasoning_text.delta" => {
                if let Some(delta) = value
                    .get("delta")
                    .and_then(Value::as_str)
                    .or_else(|| value.get("text").and_then(Value::as_str))
                {
                    reasoning_content.push_str(delta);
                }
            }
            "response.output_text.done" => {
                if content.is_empty() {
                    if let Some(text) = value.get("text").and_then(Value::as_str) {
                        content.push_str(text);
                    }
                }
            }
            "response.completed" => {
                if let Some(usage) = value
                    .get("response")
                    .and_then(|response| response.get("usage"))
                {
                    prompt_tokens = usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(prompt_tokens);
                    completion_tokens = usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(completion_tokens);
                }
            }
            "response.failed" | "error" => {
                let message = value
                    .get("error")
                    .and_then(|error| error.get("message").or_else(|| error.get("error")))
                    .and_then(Value::as_str)
                    .or_else(|| value.get("message").and_then(Value::as_str))
                    .unwrap_or("Codex stream error");
                return Err(ApiErrorResponse::new(
                    StatusCode::BAD_GATEWAY,
                    format!("codex stream failed: {message}"),
                    "provider_error",
                ));
            }
            _ => {}
        }
    }

    let mut message = json!({
        "role": "assistant",
        "content": content,
    });
    if !reasoning_content.is_empty() {
        message.as_object_mut().unwrap().insert(
            "reasoning_content".to_string(),
            Value::String(reasoning_content),
        );
    }

    Ok((
        json!({
            "id": format!("chatcmpl-codex-{}", unix_timestamp()),
            "object": "chat.completion",
            "created": unix_timestamp(),
            "model": model,
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens
            }
        }),
        prompt_tokens,
        completion_tokens,
    ))
}

fn sse_data(event: &str) -> Option<String> {
    let lines = event
        .lines()
        .filter_map(|line| line.trim().strip_prefix("data:").map(str::trim))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
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

    if provider_uses_codex_model_catalog(provider) {
        if let Ok(models) = fetch_codex_model_ids(&client, provider).await {
            return Ok(models);
        }
    }

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

async fn fetch_codex_model_ids(
    client: &reqwest::Client,
    provider: &RawProviderConnection,
) -> Result<Vec<String>, ApiErrorResponse> {
    let token = provider_token(&provider.data).ok_or_else(|| {
        ApiErrorResponse::bad_request("No ChatGPT/Codex access token found for model fetch")
    })?;
    let response = client
        .get(CODEX_MODELS_URL)
        .header(header::ACCEPT, "application/json")
        .header(header::CONTENT_TYPE, "application/json")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| {
            ApiErrorResponse::bad_request(format!("Codex model fetch failed: {error}"))
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(ApiErrorResponse::bad_request(format!(
            "Codex model fetch returned HTTP {}",
            status.as_u16()
        )));
    }
    let payload = response.json::<Value>().await.map_err(|error| {
        ApiErrorResponse::bad_request(format!("Codex model fetch returned invalid JSON: {error}"))
    })?;
    let models = extract_codex_model_ids(&payload);
    if models.is_empty() {
        return Err(ApiErrorResponse::bad_request(
            "Codex model fetch succeeded but no model ids were found",
        ));
    }
    Ok(models)
}

fn provider_uses_codex_model_catalog(provider: &RawProviderConnection) -> bool {
    if provider.provider.eq_ignore_ascii_case("codex") {
        return true;
    }

    let Some(token) = provider_token(&provider.data) else {
        return false;
    };
    let Some(payload) = decode_jwt_payload(token) else {
        return false;
    };

    payload
        .get("https://api.openai.com/auth")
        .and_then(Value::as_object)
        .map(|auth| {
            auth.contains_key("chatgpt_account_id")
                || auth.contains_key("chatgpt_plan_type")
                || auth.contains_key("chatgpt_user_id")
        })
        .unwrap_or(false)
}

fn decode_jwt_payload(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn extract_codex_model_ids(payload: &Value) -> Vec<String> {
    let mut models = Vec::new();
    for item in codex_model_items(payload) {
        let Some(id) = item
            .get("id")
            .or_else(|| item.get("slug"))
            .or_else(|| item.get("model"))
            .or_else(|| item.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        models.push(id.to_string());
    }
    models.sort();
    models.dedup();
    models
}

fn codex_model_items(payload: &Value) -> Vec<&serde_json::Map<String, Value>> {
    let mut items = Vec::new();
    for key in ["data", "models", "results"] {
        if let Some(array) = payload.get(key).and_then(Value::as_array) {
            items.extend(array.iter().filter_map(Value::as_object));
        }
    }
    if items.is_empty() {
        if let Some(array) = payload.as_array() {
            items.extend(array.iter().filter_map(Value::as_object));
        }
    }
    items
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

fn builtin_provider_models(provider: &RawProviderConnection) -> Option<BuiltinProviderModels> {
    let alias = provider
        .data
        .get("alias")
        .and_then(Value::as_str)
        .or_else(|| provider.data.get("provider_alias").and_then(Value::as_str));

    registry_builtin_provider_models(&provider.provider, alias)
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
    Json(mut body): Json<Value>,
) -> Result<Response, ApiErrorResponse> {
    authorize_v1(&state, &headers)?;
    let api_key = extract_api_key(&headers);
    let route = normalize_model_request_body(&state.storage, &mut body)?;

    proxy_chat_completions(&state, body, route, api_key).await
}

fn format_model_ref(model: &ModelRef) -> String {
    format!("{}/{}", model.provider, model.model)
}

fn normalize_model_request_body(
    storage: &Storage,
    body: &mut Value,
) -> Result<RouteKind, ApiErrorResponse> {
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiErrorResponse::bad_request("missing model"))?;

    let route = storage
        .resolve_model_route(model)
        .map_err(|error| storage_error_to_api(error, "model route resolution failed"))?;

    let normalized = normalized_route_model(&route);
    if let Some(object) = body.as_object_mut() {
        object.insert("model".to_string(), Value::String(normalized));
    }

    Ok(route)
}

fn normalized_route_model(route: &RouteKind) -> String {
    match route {
        RouteKind::Single(model) => format_model_ref(model),
        RouteKind::Combo { name, .. } => name.clone(),
    }
}

/// Generate rate limit headers compatible with OpenAI format
async fn rate_limit_headers(rtk: &Rtk, provider: &str, model: &str) -> Vec<(&'static str, String)> {
    let key = RequestKey {
        provider: provider.to_string(),
        model: model.to_string(),
        api_key: None,
    };
    let status = rtk.rate_limit_status(&key).await;
    vec![
        ("x-ratelimit-limit-requests", "60".to_string()),
        (
            "x-ratelimit-remaining-requests",
            status.requests_remaining.to_string(),
        ),
        (
            "x-ratelimit-reset-requests",
            status
                .retry_after
                .map_or("0".to_string(), |r| r.as_secs().to_string()),
        ),
        ("x-ratelimit-limit-tokens", "100000".to_string()),
        (
            "x-ratelimit-remaining-tokens",
            status.tokens_remaining.to_string(),
        ),
    ]
}

/// Map upstream provider error to a meaningful OpenAI-compatible error message
fn map_provider_error(status: u16, bytes: &[u8], provider: &str) -> String {
    // Try to parse common error formats from providers
    if let Ok(body) = serde_json::from_slice::<Value>(bytes) {
        // Claude error format: {"type":"error","error":{"type":"...","message":"..."}}
        if let Some(msg) = body
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
        {
            return format!("{} error: {}", provider, msg);
        }
        // OpenAI error format: {"error":{"message":"..."}}
        if let Some(msg) = body.get("error").and_then(Value::as_str) {
            return format!("{} error: {}", provider, msg);
        }
        // Gemini error format: [{"error":{"message":"..."}}]
        if let Some(arr) = body.as_array() {
            if let Some(msg) = arr
                .first()
                .and_then(|e| e.get("error"))
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
            {
                return format!("{} error: {}", provider, msg);
            }
        }
        // Generic: try "message" field
        if let Some(msg) = body.get("message").and_then(Value::as_str) {
            return format!("{} error: {}", provider, msg);
        }
    }
    // Fallback: show truncated body
    let preview = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]);
    format!("{} returned HTTP {}: {}", provider, status, preview.trim())
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
        WireFormat::Gemini | WireFormat::Antigravity => {
            let value = if upstream_format == WireFormat::Antigravity {
                value.get("response").unwrap_or(&value)
            } else {
                &value
            };
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
        WireFormat::OpenAiResponses => {
            let prompt = value
                .get("usage")
                .and_then(|u| u.get("input_tokens").or_else(|| u.get("prompt_tokens")))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let completion = value
                .get("usage")
                .and_then(|u| {
                    u.get("output_tokens")
                        .or_else(|| u.get("completion_tokens"))
                })
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
    if p == "gemini" || p == "google" || p == "antigravity" {
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

#[derive(Clone, Debug, Serialize)]
struct CostGuardStatus {
    enabled: bool,
    deny_on_exceeded: bool,
    daily_budget_usd: Option<f64>,
    monthly_budget_usd: Option<f64>,
    daily_spend_usd: f64,
    monthly_spend_usd: f64,
    daily_exceeded: bool,
    monthly_exceeded: bool,
}

#[derive(Clone, Debug)]
struct CostGuardConfig {
    enabled: bool,
    deny_on_exceeded: bool,
    daily_budget_usd: Option<f64>,
    monthly_budget_usd: Option<f64>,
}

fn enforce_cost_guard(state: &AppState) -> Result<(), ApiErrorResponse> {
    let status = cost_guard_status(&state.storage)
        .map_err(|error| ApiErrorResponse::internal(format!("cost guard failed: {error}")))?;
    if status.enabled
        && status.deny_on_exceeded
        && (status.daily_exceeded || status.monthly_exceeded)
    {
        return Err(ApiErrorResponse::new(
            StatusCode::PAYMENT_REQUIRED,
            "cost guard budget exceeded",
            "budget_exceeded",
        ));
    }
    Ok(())
}

fn cost_guard_status(storage: &Storage) -> Result<CostGuardStatus, StorageError> {
    let config = cost_guard_config(storage)?;
    let now = blackrouter_common::unix_timestamp();
    let day_start = now - (now % 86_400);
    let (_, _, day) = days_to_date((now / 86_400) as i64);
    let month_start = day_start.saturating_sub((day.saturating_sub(1) as u64) * 86_400);
    let daily_spend_usd = storage.total_cost_since(day_start)?;
    let monthly_spend_usd = storage.total_cost_since(month_start)?;

    Ok(CostGuardStatus {
        enabled: config.enabled,
        deny_on_exceeded: config.deny_on_exceeded,
        daily_budget_usd: config.daily_budget_usd,
        monthly_budget_usd: config.monthly_budget_usd,
        daily_spend_usd,
        monthly_spend_usd,
        daily_exceeded: config
            .daily_budget_usd
            .map(|budget| daily_spend_usd >= budget)
            .unwrap_or(false),
        monthly_exceeded: config
            .monthly_budget_usd
            .map(|budget| monthly_spend_usd >= budget)
            .unwrap_or(false),
    })
}

fn cost_guard_config(storage: &Storage) -> Result<CostGuardConfig, StorageError> {
    let settings = storage.settings_json()?;
    let guard = settings
        .get("costGuard")
        .or_else(|| settings.get("cost_guard"))
        .unwrap_or(&Value::Null);

    Ok(CostGuardConfig {
        enabled: guard
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        deny_on_exceeded: guard
            .get("denyOnExceeded")
            .or_else(|| guard.get("deny_on_exceeded"))
            .and_then(Value::as_bool)
            .unwrap_or(true),
        daily_budget_usd: guard
            .get("dailyBudgetUsd")
            .or_else(|| guard.get("daily_budget_usd"))
            .or_else(|| guard.get("dailyBudget"))
            .and_then(Value::as_f64),
        monthly_budget_usd: guard
            .get("monthlyBudgetUsd")
            .or_else(|| guard.get("monthly_budget_usd"))
            .or_else(|| guard.get("monthlyBudget"))
            .and_then(Value::as_f64),
    })
}

/// Record usage to storage asynchronously (non-blocking, fire-and-forget)
fn record_usage_async(
    usage_tx: &tokio::sync::mpsc::UnboundedSender<UsageEntry>,
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
    let _ = usage_tx.send(entry);
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

fn upstream_rate_limit_snapshot(
    headers: &reqwest::header::HeaderMap,
    status: u16,
    model: &str,
) -> Option<Value> {
    let mut values = serde_json::Map::new();
    for header_name in [
        "x-ratelimit-limit-requests",
        "x-ratelimit-remaining-requests",
        "x-ratelimit-reset-requests",
        "x-ratelimit-limit-tokens",
        "x-ratelimit-remaining-tokens",
        "x-ratelimit-reset-tokens",
        "retry-after",
    ] {
        if let Some(value) = headers
            .get(header_name)
            .and_then(|value| value.to_str().ok())
        {
            values.insert(header_name.to_string(), Value::String(value.to_string()));
        }
    }

    if values.is_empty() {
        return None;
    }

    Some(json!({
        "observedAt": blackrouter_common::unix_timestamp().to_string(),
        "status": status,
        "model": model,
        "headers": values,
    }))
}

fn record_provider_rate_limit_async(storage: &Storage, provider_id: &str, snapshot: Value) {
    let storage = storage.clone();
    let provider_id = provider_id.to_string();
    tokio::spawn(async move {
        if let Err(error) = storage.set_provider_rate_limit_snapshot(&provider_id, snapshot) {
            tracing::warn!("failed to record provider rate-limit snapshot: {error}");
        }
    });
}

fn apply_upstream_rate_limit_headers(
    mut builder: axum::http::response::Builder,
    snapshot: Option<&Value>,
) -> axum::http::response::Builder {
    let Some(headers) = snapshot
        .and_then(|value| value.get("headers"))
        .and_then(Value::as_object)
    else {
        return builder;
    };

    for (source, target) in [
        (
            "x-ratelimit-limit-requests",
            "x-upstream-ratelimit-limit-requests",
        ),
        (
            "x-ratelimit-remaining-requests",
            "x-upstream-ratelimit-remaining-requests",
        ),
        (
            "x-ratelimit-reset-requests",
            "x-upstream-ratelimit-reset-requests",
        ),
        (
            "x-ratelimit-limit-tokens",
            "x-upstream-ratelimit-limit-tokens",
        ),
        (
            "x-ratelimit-remaining-tokens",
            "x-upstream-ratelimit-remaining-tokens",
        ),
        (
            "x-ratelimit-reset-tokens",
            "x-upstream-ratelimit-reset-tokens",
        ),
        ("retry-after", "x-upstream-retry-after"),
    ] {
        if let Some(value) = headers.get(source).and_then(Value::as_str) {
            builder = builder.header(target, value);
        }
    }

    builder
}

fn oauth_refresh_token(data: &Value) -> Option<&str> {
    data.get("refreshToken")
        .or_else(|| data.get("refresh_token"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn oauth_token_needs_refresh(data: &Value) -> bool {
    let Some(expires_at) = data
        .get("tokenExpiresAt")
        .or_else(|| data.get("token_expires_at"))
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<u64>().ok())
    else {
        // Older saved OAuth providers may not have an expiry timestamp.
        // If a refresh token is available, refresh proactively instead of
        // waiting for the upstream request to fail with invalid credentials.
        return oauth_refresh_token(data).is_some();
    };
    expires_at <= blackrouter_common::unix_timestamp().saturating_add(60)
}

async fn refresh_google_oauth_access_token(
    state: &AppState,
    provider: &RawProviderConnection,
    data: &mut Value,
) -> Result<bool, ApiErrorResponse> {
    let Some(refresh_token) = oauth_refresh_token(data).map(ToOwned::to_owned) else {
        return Ok(false);
    };

    let provider_kind = provider.provider.to_ascii_lowercase();
    let (client_id, client_secret) = if provider_kind == "antigravity"
        || data.get("format").and_then(Value::as_str) == Some("antigravity")
    {
        (
            std::env::var("OAUTH_ANTIGRAVITY_CLIENT_ID").unwrap_or_default(),
            std::env::var("OAUTH_ANTIGRAVITY_CLIENT_SECRET").unwrap_or_default(),
        )
    } else if provider_kind == "gemini"
        || data.get("format").and_then(Value::as_str) == Some("gemini-cli")
    {
        (
            std::env::var("OAUTH_GOOGLE_CLIENT_ID").unwrap_or_default(),
            std::env::var("OAUTH_GOOGLE_CLIENT_SECRET").unwrap_or_default(),
        )
    } else {
        return Ok(false);
    };

    if client_id.is_empty() || client_secret.is_empty() {
        return Err(ApiErrorResponse::new(
            StatusCode::BAD_GATEWAY,
            "oauth refresh failed: missing OAuth client credentials",
            "provider_error",
        ));
    }

    let response = state
        .http_client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|error| {
            ApiErrorResponse::new(
                StatusCode::BAD_GATEWAY,
                format!("oauth refresh request failed: {error}"),
                "provider_error",
            )
        })?;

    let status = response.status();
    let body: Value = response.json().await.unwrap_or_else(|_| json!({}));
    if !status.is_success() {
        let message = body
            .get("error_description")
            .or_else(|| body.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("token refresh rejected");
        return Err(ApiErrorResponse::new(
            StatusCode::BAD_GATEWAY,
            format!("oauth refresh failed: {message}"),
            "provider_error",
        ));
    }

    let access_token = body
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiErrorResponse::new(
                StatusCode::BAD_GATEWAY,
                "oauth refresh failed: no access_token in response",
                "provider_error",
            )
        })?;
    let token_expires_at = body
        .get("expires_in")
        .and_then(Value::as_u64)
        .map(|seconds| {
            blackrouter_common::unix_timestamp()
                .saturating_add(seconds)
                .to_string()
        });

    if let Some(object) = data.as_object_mut() {
        if object.contains_key("apiKey") {
            object.insert(
                "apiKey".to_string(),
                Value::String(access_token.to_string()),
            );
        }
        object.insert(
            "accessToken".to_string(),
            Value::String(access_token.to_string()),
        );
        if let Some(token_expires_at) = &token_expires_at {
            object.insert(
                "tokenExpiresAt".to_string(),
                Value::String(token_expires_at.clone()),
            );
        }
    }

    state
        .storage
        .set_provider_oauth_access_token(&provider.id, access_token, token_expires_at)
        .map_err(|error| storage_error_to_api(error, "oauth token save failed"))?;

    Ok(true)
}

fn build_upstream_request(
    state: &AppState,
    provider: &RawProviderConnection,
    request_url: &str,
    upstream_body: &Value,
    auth_data: &Value,
    target_format: WireFormat,
    uses_codex_backend: bool,
) -> reqwest::RequestBuilder {
    let request = state.http_client.post(request_url).json(upstream_body);
    let mut request = apply_auth(request, &provider.auth_type, auth_data);

    if target_format == WireFormat::Antigravity {
        request = request
            .header("User-Agent", "antigravity/1.107.0")
            .header("x-request-source", "local");
    } else if target_format == WireFormat::CommandCode {
        request = apply_commandcode_headers(request);
    } else if uses_codex_backend {
        request = apply_codex_headers(request);
    }

    request
}

async fn proxy_chat_completions(
    state: &AppState,
    body: Value,
    route: RouteKind,
    api_key: Option<String>,
) -> Result<Response, ApiErrorResponse> {
    let is_stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);

    let models = match route {
        RouteKind::Single(model) => vec![model],
        RouteKind::Combo { models, .. } => models,
    };

    let mut last_error = None;
    for model in models {
        match proxy_single_chat_completion(state, &body, &model, is_stream, api_key.as_deref())
            .await
        {
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

/// Generate a cache key for a request (Phase 4.2 — response caching)
/// Only used for non-streaming, deterministic requests (temp=0, no tools)
fn generate_cache_key(model: &ModelRef, body: &Value) -> Option<String> {
    // Only cache non-streaming requests
    if body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    // Only cache requests with no tools
    if body.get("tools").is_some() || body.get("tool_choice").is_some() {
        return None;
    }
    // Only cache requests with temperature=0 or absent (deterministic)
    if let Some(temp) = body.get("temperature") {
        if temp.as_f64().map(|t| t != 0.0).unwrap_or(true) {
            return None;
        }
    }

    // Build cache key from model + messages + key params
    let messages = body
        .get("messages")
        .map(|m| m.to_string())
        .unwrap_or_default();
    let max_tokens = body
        .get("max_tokens")
        .map(|m| m.to_string())
        .unwrap_or_default();
    let top_p = body.get("top_p").map(|m| m.to_string()).unwrap_or_default();

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "{}/{}/{}/{}/{}/{}",
        model.provider,
        model.model,
        messages,
        max_tokens,
        top_p,
        body.get("frequency_penalty")
            .map(|v| v.to_string())
            .unwrap_or_default()
    ));
    let hash = hasher.finalize();
    use base64::Engine;
    Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash))
}

fn provider_cooldown_remaining(provider: &RawProviderConnection) -> Option<u64> {
    let cooldown_until = provider.cooldown_until.as_deref()?.parse::<u64>().ok()?;
    cooldown_until.checked_sub(blackrouter_common::unix_timestamp())
}

fn should_cooldown_provider(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// Proxy a single model with load balancing, circuit breaker, and caching (Phase 4)
async fn proxy_single_chat_completion(
    state: &AppState,
    body: &Value,
    model: &ModelRef,
    is_stream: bool,
    api_key: Option<&str>,
) -> Result<Response, ApiErrorResponse> {
    // ── Response Cache check (Phase 4.2) ──────────────────────────────
    if !is_stream {
        if let Some(cache_key) = generate_cache_key(model, body) {
            if let Some((cached_body, content_type)) = state.response_cache.get(&cache_key).await {
                tracing::debug!("cache hit for model {}/{}", model.provider, model.model);
                state
                    .metrics
                    .requests_total
                    .with_label_values(&[&model.provider, &model.model, "cache_hit"])
                    .inc();
                let mut builder = Response::builder().status(StatusCode::OK);
                if let Some(ct) = content_type {
                    builder = builder.header(header::CONTENT_TYPE, ct);
                }
                return builder.body(Body::from(cached_body)).map_err(|error| {
                    ApiErrorResponse::internal(format!("cache response: {error}"))
                });
            }
        }
    }

    enforce_cost_guard(state)?;

    // ── Load Balancing + Circuit Breaker (Phase 4.1) ──────────────────
    let providers = state
        .storage
        .list_active_provider_connections(&model.provider)
        .map_err(|error| storage_error_to_api(error, "provider listing failed"))?;

    if providers.is_empty() {
        return Err(ApiErrorResponse::new(
            StatusCode::BAD_GATEWAY,
            format!("no active provider connections for {}", model.provider),
            "provider_error",
        ));
    }

    // Filter out cooling down and circuit-broken providers.
    let mut available: Vec<&RawProviderConnection> = Vec::new();
    let mut shortest_cooldown: Option<u64> = None;
    for p in &providers {
        if let Some(remaining) = provider_cooldown_remaining(p) {
            shortest_cooldown = Some(
                shortest_cooldown
                    .map(|current| current.min(remaining))
                    .unwrap_or(remaining),
            );
            continue;
        }
        if !state.rtk.is_circuit_open(&p.id).await {
            available.push(p);
        }
    }

    if available.is_empty() && shortest_cooldown.is_some() {
        let retry_after = shortest_cooldown.unwrap_or(1).max(1);
        return Err(ApiErrorResponse::new(
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "provider {} is cooling down, retry after {retry_after}s",
                model.provider
            ),
            "provider_cooldown",
        ));
    }

    // If all are circuit-broken, try the first non-cooldown provider anyway (half-open recovery).
    if available.is_empty() {
        tracing::warn!(
            "All providers circuit-broken for {}, attempting recovery",
            model.provider
        );
        available.push(&providers[0]);
    }

    // Select starting index via load balancing strategy
    let start_idx = state
        .rtk
        .select_provider_index(&model.provider, available.len())
        .await;

    // Try providers starting from selected index
    let mut last_error = None;
    for i in 0..available.len() {
        let idx = (start_idx + i) % available.len();
        let provider = available[idx];

        match proxy_with_specific_provider(state, body, model, is_stream, api_key, provider).await {
            Ok(response) => {
                let _ = state.storage.set_provider_runtime_status(
                    &provider.id,
                    "healthy",
                    None,
                    provider.expires_at.clone(),
                );
                state.rtk.record_circuit_success(&provider.id).await;
                return Ok(response);
            }
            Err(error) => {
                tracing::warn!(
                    "provider {} ({}) failed: {}, trying next",
                    idx,
                    provider.id,
                    error.message
                );
                if should_cooldown_provider(error.status) {
                    let cooldown_until = (blackrouter_common::unix_timestamp() + 30).to_string();
                    let _ = state.storage.set_provider_runtime_status(
                        &provider.id,
                        "cooldown",
                        Some(cooldown_until),
                        provider.expires_at.clone(),
                    );
                    state.rtk.record_circuit_failure(&provider.id).await;
                }
                last_error = Some(error);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        ApiErrorResponse::new(
            StatusCode::BAD_GATEWAY,
            "all providers failed",
            "provider_error",
        )
    }))
}

/// Send request to a specific provider (extracted from proxy_single_chat_completion)
async fn proxy_with_specific_provider(
    state: &AppState,
    body: &Value,
    model: &ModelRef,
    is_stream: bool,
    api_key: Option<&str>,
    provider: &RawProviderConnection,
) -> Result<Response, ApiErrorResponse> {
    let mut auth_data = provider.data.clone();
    let mut format = provider
        .data
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("openai")
        .to_ascii_lowercase();

    let mut base_url = provider
        .data
        .get("baseUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| ApiErrorResponse::bad_request("Missing provider data.baseUrl"))?;

    let uses_codex_backend = provider_uses_codex_model_catalog(provider);
    if uses_codex_backend {
        format = "openai-responses".to_string();
        base_url = CODEX_RESPONSES_URL.to_string();
    }

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

    let mut upstream_body = translated_body;
    if let Some(object) = upstream_body.as_object_mut() {
        if target_format == WireFormat::CommandCode {
            if let Some(params) = object.get_mut("params").and_then(Value::as_object_mut) {
                params.insert("model".to_string(), Value::String(model.model.clone()));
                params.insert("stream".to_string(), Value::Bool(true));
            }
        } else {
            object.insert("model".to_string(), Value::String(model.model.clone()));
        }
    }
    if uses_codex_backend {
        sanitize_codex_responses_body(&mut upstream_body);
    }

    // For translated streaming (OpenAI→Claude/Gemini with stream=true),
    // keep stream flag and use SSE event-by-event translation (Phase 2.3).
    let passthrough_stream = is_stream && target_format == WireFormat::OpenAiChat;
    // No longer strip stream flag — upstream gets stream=true and we translate SSE events

    // Use shared HTTP client (Phase 1.2 — connection pooling)
    // Antigravity uses different endpoints for streaming vs non-streaming
    let request_url = if target_format == WireFormat::Antigravity {
        if is_stream {
            format!("{}/v1internal:streamGenerateContent?alt=sse", base_url)
        } else {
            format!("{}/v1internal:generateContent", base_url)
        }
    } else {
        base_url
    };
    // Add Antigravity-specific body fields before serializing the request body.
    if target_format == WireFormat::Antigravity {
        // Add project (from provider data or generate fallback) and requestId
        if let Some(object) = upstream_body.as_object_mut() {
            let project = auth_data
                .get("projectId")
                .and_then(Value::as_str)
                .unwrap_or("blackrouter");
            object.insert("project".to_string(), Value::String(project.to_string()));
            object.insert(
                "requestId".to_string(),
                Value::String(format!("agent-{}", uuid::Uuid::new_v4())),
            );
        }
    }

    if target_format == WireFormat::Antigravity && oauth_token_needs_refresh(&auth_data) {
        refresh_google_oauth_access_token(state, provider, &mut auth_data).await?;
    }

    let request_key = RequestKey {
        provider: model.provider.clone(),
        model: model.model.clone(),
        api_key: api_key.map(|k| k.to_string()),
    };

    // Check rate limit (with request queuing — Phase 4.3)
    let mut rate_limit_retries = 0u32;
    loop {
        if state.rtk.check_rate_limit(&request_key).await {
            break;
        }
        rate_limit_retries += 1;
        if rate_limit_retries >= 3 {
            return Err(ApiErrorResponse::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded after queuing retries",
                "rate_limit_error",
            ));
        }
        tracing::debug!(
            "rate limited, queuing request (retry {})",
            rate_limit_retries
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // Record request start
    state.rtk.record_request_start(&request_key).await;
    let start_time = std::time::Instant::now();

    // Send request
    let request = build_upstream_request(
        state,
        provider,
        &request_url,
        &upstream_body,
        &auth_data,
        target_format,
        uses_codex_backend,
    );
    let mut response = match request.send().await {
        Ok(r) => r,
        Err(error) => {
            state
                .rtk
                .record_request_end(&request_key, false, start_time.elapsed(), 0, 0, 0.0)
                .await;
            state
                .metrics
                .requests_total
                .with_label_values(&[&model.provider, &model.model, "error"])
                .inc();
            state
                .metrics
                .request_duration
                .with_label_values(&[&model.provider, &model.model])
                .observe(start_time.elapsed().as_secs_f64());
            return Err(ApiErrorResponse::new(
                StatusCode::BAD_GATEWAY,
                format!("provider request failed: {error}"),
                "provider_error",
            ));
        }
    };

    let mut status = response.status();
    let mut upstream_rate_limit =
        upstream_rate_limit_snapshot(response.headers(), status.as_u16(), &model.model);
    if let Some(snapshot) = upstream_rate_limit.clone() {
        record_provider_rate_limit_async(&state.storage, &provider.id, snapshot);
    }

    if target_format == WireFormat::Antigravity
        && matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
        && oauth_refresh_token(&auth_data).is_some()
    {
        refresh_google_oauth_access_token(state, provider, &mut auth_data).await?;
        let retry_request = build_upstream_request(
            state,
            provider,
            &request_url,
            &upstream_body,
            &auth_data,
            target_format,
            uses_codex_backend,
        );
        response = retry_request.send().await.map_err(|error| {
            ApiErrorResponse::new(
                StatusCode::BAD_GATEWAY,
                format!("provider retry after oauth refresh failed: {error}"),
                "provider_error",
            )
        })?;
        status = response.status();
        upstream_rate_limit =
            upstream_rate_limit_snapshot(response.headers(), status.as_u16(), &model.model);
        if let Some(snapshot) = upstream_rate_limit.clone() {
            record_provider_rate_limit_async(&state.storage, &provider.id, snapshot);
        }
    }

    // Non-success: record failure and return error (allows combo fallback)
    if !status.is_success() {
        state
            .rtk
            .record_request_end(&request_key, false, start_time.elapsed(), 0, 0, 0.0)
            .await;
        state
            .metrics
            .requests_total
            .with_label_values(&[&model.provider, &model.model, &status.as_u16().to_string()])
            .inc();
        state
            .metrics
            .request_duration
            .with_label_values(&[&model.provider, &model.model])
            .observe(start_time.elapsed().as_secs_f64());
        let error_bytes = response.bytes().await.unwrap_or_default();
        let error_msg = map_provider_error(status.as_u16(), &error_bytes, &model.provider);
        return Err(ApiErrorResponse::new(
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            error_msg,
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

        let builder = Response::builder()
            .status(status)
            .header(
                header::CONTENT_TYPE,
                content_type.unwrap_or_else(|| "text/event-stream".to_string()),
            )
            .header("Cache-Control", "no-cache")
            .header("Connection", "keep-alive");
        let builder = apply_upstream_rate_limit_headers(builder, upstream_rate_limit.as_ref());
        return builder.body(body).map_err(|error| {
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

        if is_stream {
            let upstream_stream = response.bytes_stream();
            let translated =
                translate_sse_stream(upstream_stream, target_format, model.model.clone());
            let body = Body::from_stream(translated);

            let builder = Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .header("Cache-Control", "no-cache")
                .header("Connection", "keep-alive");
            let builder = apply_upstream_rate_limit_headers(builder, upstream_rate_limit.as_ref());
            return builder.body(body).map_err(|error| {
                ApiErrorResponse::internal(format!(
                    "failed to build translated stream response: {error}"
                ))
            });
        }
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

    let codex_chat_response = if uses_codex_backend && target_format == WireFormat::OpenAiResponses
    {
        Some(codex_stream_to_chat_response(&bytes, &model.model)?)
    } else {
        None
    };

    // Parse token usage from upstream response (Phase 3.1)
    let (prompt_tokens, completion_tokens) =
        if let Some((_, prompt, completion)) = &codex_chat_response {
            (*prompt, *completion)
        } else if target_format == WireFormat::CommandCode {
            commandcode_stream_token_usage(&String::from_utf8_lossy(&bytes))
        } else {
            parse_token_usage(&bytes, target_format)
        };

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

    // Record prometheus metrics
    state
        .metrics
        .requests_total
        .with_label_values(&[&model.provider, &model.model, "success"])
        .inc();
    state
        .metrics
        .request_duration
        .with_label_values(&[&model.provider, &model.model])
        .observe(start_time.elapsed().as_secs_f64());
    state
        .metrics
        .tokens_total
        .with_label_values(&[&model.provider, &model.model, "prompt"])
        .inc_by(prompt_tokens);
    state
        .metrics
        .tokens_total
        .with_label_values(&[&model.provider, &model.model, "completion"])
        .inc_by(completion_tokens);

    // Record to usage storage (async, non-blocking, batched)
    record_usage_async(
        &state.usage_tx,
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
            "upstream_rate_limit": upstream_rate_limit.clone(),
        }),
    );

    // Translate response back to OpenAI format if needed
    let response_body = if let Some((response_value, _, _)) = codex_chat_response {
        serde_json::to_vec(&response_value).unwrap_or_else(|_| bytes.to_vec())
    } else if target_format != WireFormat::OpenAiChat {
        if target_format == WireFormat::CommandCode {
            let translated =
                commandcode_stream_text_to_openai(&String::from_utf8_lossy(&bytes), &model.model);
            serde_json::to_vec(&translated).unwrap_or_else(|_| bytes.to_vec())
        } else {
            let response_value: Value = serde_json::from_slice(&bytes).unwrap_or_else(
                |_| serde_json::json!({"raw": String::from_utf8_lossy(&bytes).to_string()}),
            );

            match translate_response(&response_value, target_format, WireFormat::OpenAiChat) {
                Ok(translated) => {
                    serde_json::to_vec(&translated).unwrap_or_else(|_| bytes.to_vec())
                }
                Err(_) => bytes.to_vec(),
            }
        }
    } else {
        bytes.to_vec()
    };

    let content_type_for_cache = if target_format != WireFormat::OpenAiChat {
        Some("application/json".to_string())
    } else {
        content_type.clone()
    };
    let mut builder = Response::builder().status(status);
    if let Some(content_type) = content_type_for_cache.clone() {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    builder = apply_upstream_rate_limit_headers(builder, upstream_rate_limit.as_ref());
    // Add rate limit headers
    for (key, value) in rate_limit_headers(&state.rtk, &model.provider, &model.model).await {
        builder = builder.header(key, value);
    }

    // Cache response if cacheable (Phase 4.2)
    if !is_stream {
        if let Some(cache_key) = generate_cache_key(model, body) {
            state
                .response_cache
                .put(cache_key, response_body.clone(), content_type_for_cache)
                .await;
        }
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

async fn telegram_webhook(
    State(state): State<AppState>,
    Json(update): Json<TelegramUpdate>,
) -> impl IntoResponse {
    if !state.config.telegram.enabled {
        tracing::debug!("Telegram webhook received while telegram is disabled");
        return (StatusCode::OK, "ok");
    }

    let Some(bot_token) = state.config.telegram.bot_token.clone() else {
        tracing::warn!("Telegram webhook received but bot token is not configured");
        return (StatusCode::OK, "ok");
    };

    let runtime = TelegramRuntime::new(
        state.storage.clone(),
        state.config.telegram.admin_ids.clone(),
    );
    let bot = match TelegramBot::new(TelegramBotConfig {
        bot_token,
        admin_ids: state.config.telegram.admin_ids.clone(),
        webhook_url: state.config.telegram.webhook_url.clone(),
        polling_interval_ms: 1000,
        max_connections: 40,
    }) {
        Ok(bot) => bot,
        Err(error) => {
            tracing::warn!("Telegram webhook bot init failed: {}", error);
            return (StatusCode::OK, "ok");
        }
    };

    if let Err(error) = runtime.dispatch_update(&bot, update).await {
        tracing::warn!("Telegram webhook handling failed: {}", error);
    }

    (StatusCode::OK, "ok")
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
    let mut chat_body = responses_request_to_chat(&body).map_err(|error| {
        ApiErrorResponse::bad_request(format!("responses conversion failed: {error}"))
    })?;

    let route = normalize_model_request_body(&state.storage, &mut chat_body)?;
    let normalized_model = chat_body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(model)
        .to_string();

    // Proxy through chat completions (reuse all streaming/translation/pooling logic)
    let response =
        proxy_chat_completions(&state, chat_body, route, extract_api_key(&headers)).await?;

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

    let responses_body =
        chat_response_to_responses(&chat_response, &normalized_model).map_err(|error| {
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
    Json(mut body): Json<Value>,
) -> Result<Response, ApiErrorResponse> {
    authorize_v1(&state, &headers)?;
    let api_key = extract_api_key(&headers);

    let route = normalize_model_request_body(&state.storage, &mut body)?;

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

        let request = state.http_client.post(base_url).json(&upstream_body);
        let request = apply_auth(request, &provider.auth_type, &provider.data);

        let request_key = RequestKey {
            provider: first_model.provider.clone(),
            model: first_model.model.clone(),
            api_key: api_key.clone(),
        };

        enforce_cost_guard(&state)?;

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

        // Record to usage storage (async, non-blocking, batched)
        record_usage_async(
            &state.usage_tx,
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

    let chat_response = proxy_chat_completions(&state, chat_body, route, api_key.clone()).await?;

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
    use super::{
        builtin_provider_models, classify_provider_post_response, codex_stream_to_chat_response,
        derive_models_url, extract_api_key, extract_codex_model_ids, extract_model_ids,
        provider_check_post_body, provider_check_uses_post, provider_uses_codex_model_catalog,
        sanitize_codex_responses_body, should_cooldown_provider,
    };
    use axum::http::{HeaderMap, StatusCode};
    use base64::Engine;
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
    fn cooldown_provider_only_for_retryable_errors() {
        assert!(should_cooldown_provider(StatusCode::TOO_MANY_REQUESTS));
        assert!(should_cooldown_provider(StatusCode::BAD_GATEWAY));
        assert!(should_cooldown_provider(StatusCode::SERVICE_UNAVAILABLE));
        assert!(!should_cooldown_provider(StatusCode::BAD_REQUEST));
        assert!(!should_cooldown_provider(StatusCode::UNPROCESSABLE_ENTITY));
        assert!(!should_cooldown_provider(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn commandcode_check_uses_post_only_probe() {
        assert!(provider_check_uses_post("commandcode"));
        assert_eq!(provider_check_post_body("commandcode"), json!({}));

        let response = classify_provider_post_response(
            "https://api.commandcode.ai/alpha/generate".into(),
            400,
        );
        assert!(response.ok);
        assert!(response.reachable);
        assert_eq!(response.status, Some(400));

        let not_found = classify_provider_post_response(
            "https://api.commandcode.ai/alpha/generate".into(),
            404,
        );
        assert!(!not_found.ok);
        assert!(not_found.message.contains("not found"));
    }

    #[test]
    fn openai_check_uses_post_only_probe() {
        assert!(provider_check_uses_post("openai"));
        assert!(provider_check_uses_post("openai-chat"));
        assert!(provider_check_uses_post("openai-responses"));
        assert_eq!(provider_check_post_body("openai"), json!({}));

        let response = classify_provider_post_response(
            "https://api.openai.com/v1/chat/completions".into(),
            400,
        );
        assert!(response.ok);
        assert!(response.reachable);
        assert_eq!(response.status, Some(400));
    }

    #[test]
    fn detects_codex_jwt_for_model_catalog() {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            r#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct","chatgpt_plan_type":"plus"}}"#,
        );
        let provider = RawProviderConnection {
            id: "1".to_string(),
            provider: "openai".to_string(),
            auth_type: "oauth".to_string(),
            name: None,
            email: None,
            priority: None,
            is_active: true,
            status: "healthy".to_string(),
            cooldown_until: None,
            expires_at: None,
            data: json!({ "apiKey": format!("{header}.{payload}.sig") }),
            created_at: "1".to_string(),
            updated_at: "1".to_string(),
        };

        assert!(provider_uses_codex_model_catalog(&provider));
    }

    #[test]
    fn extracts_codex_models_without_synthesizing_review_variants() {
        let models = extract_codex_model_ids(&json!({
            "data": [
                { "id": "gpt-5.3-codex", "type": "llm" },
                { "id": "gpt-5.3-codex-review", "type": "llm" },
                { "slug": "gpt-image-2", "type": "image" }
            ]
        }));

        assert!(models.contains(&"gpt-5.3-codex".to_string()));
        assert!(models.contains(&"gpt-5.3-codex-review".to_string()));
        assert!(models.contains(&"gpt-image-2".to_string()));
        assert!(!models.contains(&"gpt-image-2-review".to_string()));
        assert!(!models.contains(&"gpt-5.5-review".to_string()));
    }

    #[test]
    fn codex_backend_body_drops_unsupported_max_output_tokens() {
        let mut body = json!({
            "model": "gpt-5.5",
            "input": [{"role": "user", "content": [{"type": "input_text", "text": "hi"}]}],
            "max_output_tokens": 16,
            "temperature": 0.2,
            "top_p": 0.9,
            "stream": false
        });

        sanitize_codex_responses_body(&mut body);

        assert!(body.get("max_output_tokens").is_none());
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
        assert_eq!(body.get("model").unwrap(), "gpt-5.5");
        assert_eq!(body.get("stream").unwrap(), true);
    }

    #[test]
    fn codex_stream_converts_to_non_stream_chat_response() {
        let raw = concat!(
            "data: {\"type\":\"response.created\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"o\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"k\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}}\n\n",
            "data: [DONE]\n\n"
        );

        let (body, prompt_tokens, completion_tokens) =
            codex_stream_to_chat_response(raw.as_bytes(), "gpt-5.5").unwrap();

        assert_eq!(prompt_tokens, 3);
        assert_eq!(completion_tokens, 1);
        assert_eq!(body["model"], "gpt-5.5");
        assert_eq!(body["choices"][0]["message"]["content"], "ok");
        assert_eq!(body["usage"]["total_tokens"], 4);
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
            status: "healthy".to_string(),
            cooldown_until: None,
            expires_at: None,
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

        let antigravity = RawProviderConnection {
            provider: "antigravity".to_string(),
            data: json!({ "alias": "ag" }),
            ..commandcode
        };
        assert!(builtin_provider_models(&antigravity)
            .unwrap()
            .models
            .contains(&"gemini-2.5-pro"));

        let gemini_cli = RawProviderConnection {
            provider: "gemini-cli".to_string(),
            data: json!({}),
            ..antigravity
        };
        assert!(builtin_provider_models(&gemini_cli)
            .unwrap()
            .models
            .contains(&"gemini-2.0-flash-lite"));
    }
}
