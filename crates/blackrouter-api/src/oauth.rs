use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Json, Response};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::oneshot;

use crate::AppState;

// ── OAuth Client Credentials (loaded from environment) ─────────────────

/// GitHub Copilot OAuth (Device Code Flow)
fn github_client_id() -> String {
    std::env::var("OAUTH_GITHUB_CLIENT_ID").unwrap_or_else(|_| "Iv1.b507a08c87ecfe98".to_string())
}

/// Google / Gemini OAuth (Authorization Code Flow)
fn google_client_id() -> String {
    std::env::var("OAUTH_GOOGLE_CLIENT_ID").unwrap_or_default()
}
fn google_client_secret() -> String {
    std::env::var("OAUTH_GOOGLE_CLIENT_SECRET").unwrap_or_default()
}

/// Antigravity OAuth credentials (Google Cloud Code Assist).
///
/// These must be supplied via the `OAUTH_ANTIGRAVITY_CLIENT_ID` and
/// `OAUTH_ANTIGRAVITY_CLIENT_SECRET` environment variables. They are no longer
/// embedded here because GitHub push protection blocks committing them; set the
/// env vars in your deployment (e.g. Dockerfile / runtime config) instead.
const ANTIGRAVITY_CLIENT_ID_FALLBACK: &str = "";
const ANTIGRAVITY_CLIENT_SECRET_FALLBACK: &str = "";

fn antigravity_client_id() -> String {
    std::env::var("OAUTH_ANTIGRAVITY_CLIENT_ID")
        .unwrap_or_else(|_| ANTIGRAVITY_CLIENT_ID_FALLBACK.to_string())
}
fn antigravity_client_secret() -> String {
    std::env::var("OAUTH_ANTIGRAVITY_CLIENT_SECRET")
        .unwrap_or_else(|_| ANTIGRAVITY_CLIENT_SECRET_FALLBACK.to_string())
}

/// Antigravity OAuth scopes (cloud-platform + userinfo + cclog + experiments)
const ANTIGRAVITY_SCOPES: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";

/// Codex / OpenAI OAuth (Authorization Code Flow with PKCE)
const CODEX_CLIENT_ID_FALLBACK: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_LOOPBACK_PORT: u16 = 1455;
const CODEX_CALLBACK_PATH: &str = "/auth/callback";
const CODEX_LOOPBACK_TIMEOUT: Duration = Duration::from_secs(300);

fn codex_client_id() -> String {
    std::env::var("OAUTH_CODEX_CLIENT_ID").unwrap_or_else(|_| CODEX_CLIENT_ID_FALLBACK.to_string())
}

// ── Session store for OAuth tokens ────────────────────────────────────

static OAUTH_SESSIONS: std::sync::LazyLock<Mutex<HashMap<String, OAuthSession>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

static CODEX_LOOPBACK_SHUTDOWN: std::sync::LazyLock<Mutex<Option<oneshot::Sender<()>>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

#[derive(Clone, Debug)]
struct OAuthSession {
    #[allow(dead_code)]
    provider: String,
    code_verifier: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    email: Option<String>,
    project_id: Option<String>,
    #[allow(dead_code)]
    expires_at: Option<String>,
    // The exact redirect_uri used in the authorization request. Persisted so the
    // token exchange (whether via the server callback or the /exchange endpoint)
    // always uses the same value the IdP expects — independent of BLACKROUTER_BASE_URL.
    redirect_uri: Option<String>,
    status: String, // "pending", "done", "error"
    error: Option<String>,
}

// ── Request/Response types ─────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct OAuthStartResponse {
    url: String,
    state: String,
    provider: String,
    flow_type: String, // "device_code" or "authorization_code"
    user_code: Option<String>,
    verification_uri: Option<String>,
    expires_in: Option<u64>,
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OAuthStatusQuery {
    state: String,
}

#[derive(Debug, Serialize)]
pub struct OAuthPollResponse {
    status: String,
    access_token: Option<String>,
    refresh_token: Option<String>,
    token_expires_at: Option<String>,
    email: Option<String>,
    project_id: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OAuthExchangeBody {
    code: String,
    state: String,
}

/// Optional body for `oauth_start`. When `redirect_uri` is supplied (frontend
/// computes it from `window.location.origin`), it is used verbatim for the
/// authorization request and persisted on the session — this is what makes the
/// flow work in any deployment (Docker, remote, LAN) without BLACKROUTER_BASE_URL.
#[derive(Debug, Deserialize)]
pub struct OAuthStartBody {
    redirect_uri: Option<String>,
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn generate_state() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("{:08x}", nanos)
}

fn generate_code_verifier() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.gen()).collect();
    base64_url(&bytes)
}

fn base64_url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn sha256_digest(input: &str) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hasher.finalize().to_vec()
}

fn urlencoding(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

fn redirect_uri(port: u16, provider: &str) -> String {
    let base = std::env::var("BLACKROUTER_BASE_URL")
        .unwrap_or_else(|_| format!("http://localhost:{}", port));
    format!(
        "{}/api/oauth/{}/callback",
        base.trim_end_matches('/'),
        provider
    )
}

/// Validate a client-supplied `redirect_uri` before trusting it in the
/// authorization request. We only accept it when it points back to this app's
/// own origin (same host as the incoming request, or localhost) and ends with
/// the well-known `/oauth/callback` path. Anything else falls back to the
/// server-derived `redirect_uri(port, provider)`.
fn sanitize_redirect_uri(provided: &Option<String>, request_host: &str) -> Option<String> {
    let provided = provided.as_ref()?;
    let url = reqwest::Url::parse(provided).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    if !url.path().ends_with("/oauth/callback") {
        return None;
    }
    let host = url.host_str().unwrap_or("");
    let request_host = request_host.split(':').next().unwrap_or(request_host);
    if host == request_host || host == "localhost" || host == "127.0.0.1" {
        Some(provided.clone())
    } else {
        None
    }
}

fn codex_redirect_uri() -> String {
    format!("http://localhost:{CODEX_LOOPBACK_PORT}{CODEX_CALLBACK_PATH}")
}

fn oauth_json_error(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": message.into() })))
}

async fn start_codex_loopback_callback(state: AppState) -> Result<(), String> {
    if CODEX_LOOPBACK_SHUTDOWN.lock().unwrap().is_some() {
        return Ok(());
    }

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", CODEX_LOOPBACK_PORT))
        .await
        .map_err(|err| {
            format!("OpenAI OAuth callback port {CODEX_LOOPBACK_PORT} is not available: {err}")
        })?;
    let (tx, rx) = oneshot::channel();
    *CODEX_LOOPBACK_SHUTDOWN.lock().unwrap() = Some(tx);

    let app = Router::new()
        .route(CODEX_CALLBACK_PATH, get(codex_loopback_callback))
        .route("/callback", get(codex_loopback_callback))
        .with_state(state);

    tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async {
            tokio::select! {
                _ = rx => {}
                _ = tokio::time::sleep(CODEX_LOOPBACK_TIMEOUT) => {}
            }
        });
        if let Err(err) = server.await {
            tracing::warn!("OpenAI OAuth loopback callback server failed: {err}");
        }
        CODEX_LOOPBACK_SHUTDOWN.lock().unwrap().take();
    });

    Ok(())
}

fn stop_codex_loopback_callback() {
    if let Some(tx) = CODEX_LOOPBACK_SHUTDOWN.lock().unwrap().take() {
        let _ = tx.send(());
    }
}

// ── Generic OAuth Start ─────────────────────────────────────────────────

/// POST /api/oauth/{provider}/start
/// Unified OAuth entry point: returns URL for user to open in browser
pub async fn oauth_start(
    Path(provider): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<OAuthStartBody>>,
) -> Result<Json<OAuthStartResponse>, (StatusCode, Json<Value>)> {
    let port = state.config.port;
    let session_state = generate_state();
    let request_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let client_redirect_uri = body.as_ref().and_then(|b| b.redirect_uri.clone());

    // Validate credentials are configured
    let configured = match provider.as_str() {
        "github" => !github_client_id().is_empty(),
        "google" | "gemini" => !google_client_id().is_empty() && !google_client_secret().is_empty(),
        "antigravity" => {
            !antigravity_client_id().is_empty() && !antigravity_client_secret().is_empty()
        }
        "codex" | "openai" => !codex_client_id().is_empty(),
        _ => false,
    };
    if !configured {
        return Err(oauth_json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "OAuth for {} is not configured. Set the required OAUTH_*_CLIENT_ID env vars.\n\
                 Copy from .env.example or register your own OAuth app.",
                provider
            ),
        ));
    }

    match provider.as_str() {
        "github" => {
            // GitHub: Device Code Flow
            let client_id = github_client_id();
            let resp = state
                .http_client
                .post("https://github.com/login/device/code")
                .header("Accept", "application/json")
                .json(&json!({
                    "client_id": client_id,
                    "scope": "read:user"
                }))
                .send()
                .await
                .map_err(|e| {
                    oauth_json_error(StatusCode::BAD_GATEWAY, format!("GitHub device flow: {e}"))
                })?;

            let body: Value = resp.json().await.map_err(|e| {
                oauth_json_error(StatusCode::BAD_GATEWAY, format!("GitHub response: {e}"))
            })?;

            let device_code = body["device_code"].as_str().unwrap_or("").to_string();
            let user_code = body["user_code"].as_str().unwrap_or("").to_string();
            let verification_uri = body["verification_uri"]
                .as_str()
                .unwrap_or("https://github.com/login/device")
                .to_string();
            let expires_in = body["expires_in"].as_u64().unwrap_or(900);
            let interval = body["interval"].as_u64().unwrap_or(5);

            // Store session for polling
            OAUTH_SESSIONS.lock().unwrap().insert(
                session_state.clone(),
                OAuthSession {
                    provider: "github".to_string(),
                    code_verifier: None,
                    access_token: Some(device_code.clone()), // Store device_code in access_token for polling
                    refresh_token: None,
                    email: None,
                    project_id: None,
                    expires_at: None,
                    redirect_uri: None,
                    status: "pending".to_string(),
                    error: None,
                },
            );

            Ok(Json(OAuthStartResponse {
                url: verification_uri.clone(),
                state: session_state,
                provider: "github".to_string(),
                flow_type: "device_code".to_string(),
                user_code: Some(user_code),
                verification_uri: Some(verification_uri),
                expires_in: Some(expires_in),
                interval: Some(interval),
            }))
        }

        "google" | "gemini" => {
            // Google: Authorization Code Flow.
            // Prefer the origin-based redirect_uri the frontend computed from
            // `window.location.origin` (works in any deployment). Fall back to
            // BLACKROUTER_BASE_URL only when the client didn't/couldn't supply one.
            let callback_url = sanitize_redirect_uri(&client_redirect_uri, &request_host)
                .unwrap_or_else(|| redirect_uri(port, &provider));
            tracing::info!("Google/Gemini OAuth start: redirect_uri={callback_url}, client_redirect={client_redirect_uri:?}, request_host={request_host}");
            let client_id = google_client_id();

            let auth_url = format!(
                "https://accounts.google.com/o/oauth2/v2/auth?\
                 client_id={}&\
                 redirect_uri={}&\
                 response_type=code&\
                 scope=https://www.googleapis.com/auth/userinfo.email&\
                 access_type=offline&\
                 prompt=consent&\
                 state={}",
                urlencoding(&client_id),
                urlencoding(&callback_url),
                urlencoding(&session_state),
            );

            OAUTH_SESSIONS.lock().unwrap().insert(
                session_state.clone(),
                OAuthSession {
                    provider: provider.clone(),
                    code_verifier: None,
                    access_token: None,
                    refresh_token: None,
                    email: None,
                    project_id: None,
                    expires_at: None,
                    redirect_uri: Some(callback_url.clone()),
                    status: "pending".to_string(),
                    error: None,
                },
            );

            Ok(Json(OAuthStartResponse {
                url: auth_url,
                state: session_state,
                provider,
                flow_type: "authorization_code".to_string(),
                user_code: None,
                verification_uri: None,
                expires_in: None,
                interval: None,
            }))
        }

        "antigravity" => {
            // Antigravity: Authorization Code Flow with extended scopes.
            let callback_url = sanitize_redirect_uri(&client_redirect_uri, &request_host)
                .unwrap_or_else(|| redirect_uri(port, &provider));
            tracing::info!("Antigravity OAuth start: redirect_uri={callback_url}, client_redirect={client_redirect_uri:?}, request_host={request_host}");
            let client_id = antigravity_client_id();

            let auth_url = format!(
                "https://accounts.google.com/o/oauth2/v2/auth?\
                 client_id={}&\
                 redirect_uri={}&\
                 response_type=code&\
                 scope={}&\
                 access_type=offline&\
                 prompt=consent&\
                 state={}",
                urlencoding(&client_id),
                urlencoding(&callback_url),
                urlencoding(ANTIGRAVITY_SCOPES),
                urlencoding(&session_state),
            );

            OAUTH_SESSIONS.lock().unwrap().insert(
                session_state.clone(),
                OAuthSession {
                    provider: provider.clone(),
                    code_verifier: None,
                    access_token: None,
                    refresh_token: None,
                    email: None,
                    project_id: None,
                    expires_at: None,
                    redirect_uri: Some(callback_url.clone()),
                    status: "pending".to_string(),
                    error: None,
                },
            );

            Ok(Json(OAuthStartResponse {
                url: auth_url,
                state: session_state,
                provider,
                flow_type: "authorization_code".to_string(),
                user_code: None,
                verification_uri: None,
                expires_in: None,
                interval: None,
            }))
        }

        "codex" | "openai" => {
            // Codex/OpenAI: Authorization Code Flow with PKCE.
            //
            // NOTE: OpenAI's OAuth app (including the shared fallback client id)
            // only allows the loopback redirect `http://localhost:1455/auth/callback`,
            // so we keep the loopback callback server for same-machine logins.
            // When the browser cannot reach that loopback (remote/Docker, or the
            // port is busy) the loopback bind is allowed to fail and the UI offers
            // a manual "paste the callback URL" fallback instead of hard-failing.
            if let Err(err) = start_codex_loopback_callback(state.clone()).await {
                tracing::warn!(
                    "OpenAI/Codex loopback callback unavailable; continuing with manual fallback: {err}"
                );
            }
            let callback_url = codex_redirect_uri();
            let code_verifier = generate_code_verifier();
            let code_challenge = base64_url(&sha256_digest(&code_verifier));
            let client_id = codex_client_id();

            let auth_url = format!(
                "https://auth.openai.com/oauth/authorize?\
                 client_id={}&\
                 redirect_uri={}&\
                 response_type=code&\
                 scope=openid%20profile%20email%20offline_access&\
                 code_challenge={}&\
                 code_challenge_method=S256&\
                 state={}&\
                 id_token_add_organizations=true&\
                 codex_cli_simplified_flow=true&\
                 originator=codex_cli_rs",
                urlencoding(&client_id),
                urlencoding(&callback_url),
                urlencoding(&code_challenge),
                urlencoding(&session_state),
            );

            OAUTH_SESSIONS.lock().unwrap().insert(
                session_state.clone(),
                OAuthSession {
                    provider: provider.clone(),
                    code_verifier: Some(code_verifier),
                    access_token: None,
                    refresh_token: None,
                    email: None,
                    project_id: None,
                    expires_at: None,
                    redirect_uri: Some(callback_url.clone()),
                    status: "pending".to_string(),
                    error: None,
                },
            );

            Ok(Json(OAuthStartResponse {
                url: auth_url,
                state: session_state,
                provider,
                flow_type: "authorization_code".to_string(),
                user_code: None,
                verification_uri: None,
                expires_in: None,
                interval: None,
            }))
        }

        _ => Err(oauth_json_error(
            StatusCode::BAD_REQUEST,
            format!("Unsupported OAuth provider: {}", provider),
        )),
    }
}

async fn codex_loopback_callback(
    State(state): State<AppState>,
    Query(query): Query<OAuthCallbackQuery>,
) -> Response {
    let session_state = query.state.clone().unwrap_or_default();

    let result = async {
        if let Some(error) = &query.error {
            return Err(error.clone());
        }
        let code = query
            .code
            .as_deref()
            .ok_or_else(|| "Missing code parameter".to_string())?;
        if session_state.is_empty() {
            return Err("Missing state parameter".to_string());
        }

        exchange_code_for_token(&state, "openai", code, &session_state).await
    }
    .await;

    match result {
        Ok(access_token) => {
            if let Some(session) = OAUTH_SESSIONS.lock().unwrap().get_mut(&session_state) {
                session.access_token = Some(access_token);
                session.status = "done".to_string();
            }
            stop_codex_loopback_callback();
            Html(oauth_result_html(true, "You can close this window now.")).into_response()
        }
        Err(err) => {
            if let Some(session) = OAUTH_SESSIONS.lock().unwrap().get_mut(&session_state) {
                session.status = "error".to_string();
                session.error = Some(err.clone());
            }
            stop_codex_loopback_callback();
            Html(oauth_result_html(false, &err)).into_response()
        }
    }
}

fn oauth_result_html(success: bool, message: &str) -> String {
    let title = if success {
        "OAuth Success"
    } else {
        "OAuth Error"
    };
    let heading = if success {
        "Authorization Complete"
    } else {
        "Authorization Failed"
    };
    let color = if success { "#2f7d5b" } else { "#b5453f" };
    format!(
        r#"<!DOCTYPE html>
<html><head><title>{title}</title>
<style>
body {{ font-family: system-ui; max-width: 500px; margin: 100px auto; text-align: center; }}
h1 {{ color: {color}; }}
</style></head>
<body>
<h1>{heading}</h1>
<p>{}</p>
<script>setTimeout(() => window.close(), 1500);</script>
</body></html>"#,
        html_escape(message)
    )
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// GET /oauth/callback
/// Static relay page. The OAuth popup lands here after the IdP redirects back.
/// It reads `code`/`state` from the URL and relays them to the opener window
/// (and to same-origin tabs via BroadcastChannel / localStorage) so the setup
/// page can complete the exchange. This makes the flow work in any deployment
/// without relying on a localhost loopback or BLACKROUTER_BASE_URL.
pub async fn oauth_callback_page() -> Html<&'static str> {
    Html(include_str!("../static/callback.html"))
}

fn oauth_redirect_html(success: bool, message: &str) -> String {
    let title = if success {
        "OAuth Success"
    } else {
        "OAuth Error"
    };
    let heading = if success {
        "Authorization Complete"
    } else {
        "Authorization Failed"
    };
    let color = if success { "#2f7d5b" } else { "#b5453f" };
    format!(
        r#"<!DOCTYPE html>
<html><head><title>{title}</title>
<style>
body {{ font-family: system-ui; max-width: 500px; margin: 100px auto; text-align: center; }}
h1 {{ color: {color}; }}
</style></head>
<body>
<h1>{heading}</h1>
<p>{}</p>
<p>Returning to BlackRouter setup...</p>
<script>setTimeout(() => window.location.replace('/setup'), 1200);</script>
</body></html>"#,
        html_escape(message)
    )
}

// ── OAuth Callback ──────────────────────────────────────────────────────

/// GET /api/oauth/{provider}/callback
/// Handles the redirect back from the OAuth provider. Exchanges code for token immediately.
pub async fn oauth_callback(
    Path(provider): Path<String>,
    State(state): State<AppState>,
    Query(query): Query<OAuthCallbackQuery>,
) -> Response {
    if let Some(error) = &query.error {
        let html = oauth_redirect_html(false, error);
        return Html(html).into_response();
    }

    let code = match &query.code {
        Some(c) => c.clone(),
        None => return (StatusCode::BAD_REQUEST, "Missing code parameter").into_response(),
    };

    let session_state = query.state.clone().unwrap_or_default();

    // Exchange code for token immediately
    let token_result = exchange_code_for_token(&state, &provider, &code, &session_state).await;

    match token_result {
        Ok(access_token) => {
            if provider == "antigravity" {
                match antigravity_onboard(&state, &access_token).await {
                    Ok(project_id) => {
                        if let Some(session) =
                            OAUTH_SESSIONS.lock().unwrap().get_mut(&session_state)
                        {
                            session.access_token = Some(access_token.clone());
                            session.project_id = Some(project_id);
                            session.status = "done".to_string();
                            tracing::info!("Antigravity onboarding complete");
                        }
                    }
                    Err(err) => {
                        if let Some(session) =
                            OAUTH_SESSIONS.lock().unwrap().get_mut(&session_state)
                        {
                            session.status = "error".to_string();
                            session.error = Some(format!("Antigravity onboarding failed: {err}"));
                        }
                        tracing::warn!("Antigravity onboarding failed: {err}");
                    }
                }
            } else {
                // Store in session
                if let Some(session) = OAUTH_SESSIONS.lock().unwrap().get_mut(&session_state) {
                    session.access_token = Some(access_token.clone());
                    session.status = "done".to_string();
                }
            }

            Html(oauth_redirect_html(
                true,
                "Token has been sent to BlackRouter. Returning to Setup.",
            ))
            .into_response()
        }
        Err(err_msg) => {
            // Store error in session
            if let Some(session) = OAUTH_SESSIONS.lock().unwrap().get_mut(&session_state) {
                session.status = "error".to_string();
                session.error = Some(err_msg.clone());
            }
            let html = oauth_redirect_html(false, &format!("Token exchange failed: {err_msg}"));
            Html(html).into_response()
        }
    }
}

/// Exchange authorization code for access token (called from callback)
async fn exchange_code_for_token(
    state: &AppState,
    provider: &str,
    code: &str,
    session_state: &str,
) -> Result<String, String> {
    match provider {
        "google" | "gemini" => {
            let callback_url = session_redirect_uri(session_state, state.config.port, provider);
            let client_id = google_client_id();
            let client_secret = google_client_secret();
            let resp = state
                .http_client
                .post("https://oauth2.googleapis.com/token")
                .form(&[
                    ("client_id", client_id.as_str()),
                    ("client_secret", client_secret.as_str()),
                    ("code", code),
                    ("redirect_uri", &callback_url),
                    ("grant_type", "authorization_code"),
                ])
                .send()
                .await
                .map_err(|e| format!("Token exchange failed: {e}"))?;

            let token: Value = resp
                .json()
                .await
                .map_err(|e| format!("Invalid response: {e}"))?;
            let access_token = token["access_token"]
                .as_str()
                .map(String::from)
                .ok_or_else(|| {
                    token["error_description"]
                        .as_str()
                        .unwrap_or("No access token")
                        .to_string()
                })?;
            if let Some(session) = OAUTH_SESSIONS.lock().unwrap().get_mut(session_state) {
                session.refresh_token = token["refresh_token"].as_str().map(String::from);
                session.expires_at = token["expires_in"].as_u64().map(|seconds| {
                    (blackrouter_common::unix_timestamp().saturating_add(seconds)).to_string()
                });
            }
            Ok(access_token)
        }

        "antigravity" => {
            let callback_url = session_redirect_uri(session_state, state.config.port, provider);
            let client_id = antigravity_client_id();
            let client_secret = antigravity_client_secret();
            let resp = state
                .http_client
                .post("https://oauth2.googleapis.com/token")
                .form(&[
                    ("client_id", client_id.as_str()),
                    ("client_secret", client_secret.as_str()),
                    ("code", code),
                    ("redirect_uri", &callback_url),
                    ("grant_type", "authorization_code"),
                ])
                .send()
                .await
                .map_err(|e| format!("Token exchange failed: {e}"))?;

            let token: Value = resp
                .json()
                .await
                .map_err(|e| format!("Invalid response: {e}"))?;
            let access_token = token["access_token"]
                .as_str()
                .map(String::from)
                .ok_or_else(|| {
                    token["error_description"]
                        .as_str()
                        .unwrap_or("No access token")
                        .to_string()
                })?;
            if let Some(session) = OAUTH_SESSIONS.lock().unwrap().get_mut(session_state) {
                session.refresh_token = token["refresh_token"].as_str().map(String::from);
                session.expires_at = token["expires_in"].as_u64().map(|seconds| {
                    (blackrouter_common::unix_timestamp().saturating_add(seconds)).to_string()
                });
            }
            Ok(access_token)
        }

        "codex" | "openai" => {
            let callback_url = session_redirect_uri(session_state, state.config.port, "openai");
            let code_verifier = OAUTH_SESSIONS
                .lock()
                .unwrap()
                .get(session_state)
                .and_then(|s| s.code_verifier.clone());
            let client_id = codex_client_id();

            let mut form: Vec<(&str, &str)> = vec![
                ("client_id", client_id.as_str()),
                ("code", code),
                ("redirect_uri", &callback_url),
                ("grant_type", "authorization_code"),
            ];
            let cv_str;
            if let Some(ref cv) = code_verifier {
                cv_str = cv.clone();
                form.push(("code_verifier", &cv_str));
            }

            let resp = state
                .http_client
                .post("https://auth.openai.com/oauth/token")
                .form(&form)
                .send()
                .await
                .map_err(|e| format!("Token exchange failed: {e}"))?;

            let token: Value = resp
                .json()
                .await
                .map_err(|e| format!("Invalid response: {e}"))?;

            // Extract email from id_token JWT
            let email = token["id_token"].as_str().and_then(|jwt| {
                jwt.split('.').nth(1).and_then(|payload| {
                    let padded = format!("{}{}", payload, "=".repeat((4 - payload.len() % 4) % 4));
                    base64_decode(&padded).ok().and_then(|json_str| {
                        serde_json::from_str::<Value>(&json_str)
                            .ok()
                            .and_then(|v| v.get("email").and_then(|e| e.as_str().map(String::from)))
                    })
                })
            });

            if let Some(session) = OAUTH_SESSIONS.lock().unwrap().get_mut(session_state) {
                session.email = email.clone();
            }

            token["access_token"]
                .as_str()
                .map(String::from)
                .ok_or_else(|| "No access token".to_string())
        }

        _ => Err(format!("Unsupported provider for callback: {provider}")),
    }
}

/// Resolve the redirect_uri to use for the token exchange. Prefers the value
/// persisted on the session (the exact one used in the authorization request),
/// falling back to the server-derived default for safety.
fn session_redirect_uri(session_state: &str, port: u16, provider: &str) -> String {
    OAUTH_SESSIONS
        .lock()
        .unwrap()
        .get(session_state)
        .and_then(|s| s.redirect_uri.clone())
        .unwrap_or_else(|| redirect_uri(port, provider))
}

// ── Exchange Code for Token ─────────────────────────────────────────────

/// POST /api/oauth/{provider}/exchange
/// Exchanges authorization code for access token
pub async fn oauth_exchange(
    Path(provider): Path<String>,
    State(state): State<AppState>,
    Json(body): Json<OAuthExchangeBody>,
) -> Result<Json<OAuthPollResponse>, (StatusCode, String)> {
    let port = state.config.port;
    match provider.as_str() {
        "google" | "gemini" => {
            let callback_url = session_redirect_uri(&body.state, port, &provider);
            tracing::info!(
                "Google/Gemini token exchange: redirect_uri={callback_url}, state={}",
                body.state
            );
            let client_id = google_client_id();
            let client_secret = google_client_secret();
            let resp = state
                .http_client
                .post("https://oauth2.googleapis.com/token")
                .form(&[
                    ("client_id", client_id.as_str()),
                    ("client_secret", client_secret.as_str()),
                    ("code", &body.code),
                    ("redirect_uri", &callback_url),
                    ("grant_type", "authorization_code"),
                ])
                .send()
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Google exchange: {e}")))?;

            let token: Value = resp
                .json()
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Google response: {e}")))?;

            let access_token = token["access_token"].as_str().unwrap_or("").to_string();
            let refresh_token = token["refresh_token"].as_str().unwrap_or("").to_string();

            if access_token.is_empty() {
                let err_detail = token["error"].as_str().unwrap_or("");
                let err_desc = token["error_description"].as_str().unwrap_or("");
                return Err((
                    StatusCode::BAD_REQUEST,
                    if !err_detail.is_empty() {
                        format!("No access token received: {err_detail} — {err_desc}")
                    } else {
                        "No access token received".to_string()
                    },
                ));
            }

            // Update session
            if let Some(session) = OAUTH_SESSIONS.lock().unwrap().get_mut(&body.state) {
                session.access_token = Some(access_token.clone());
                session.refresh_token = if refresh_token.is_empty() {
                    None
                } else {
                    Some(refresh_token)
                };
                session.expires_at = token["expires_in"].as_u64().map(|seconds| {
                    (blackrouter_common::unix_timestamp().saturating_add(seconds)).to_string()
                });
                session.status = "done".to_string();
            }

            Ok(Json(OAuthPollResponse {
                status: "done".to_string(),
                access_token: Some(access_token),
                refresh_token: None,
                token_expires_at: token["expires_in"].as_u64().map(|seconds| {
                    (blackrouter_common::unix_timestamp().saturating_add(seconds)).to_string()
                }),
                email: None,
                project_id: None,
                error: None,
            }))
        }

        "antigravity" => {
            let callback_url = session_redirect_uri(&body.state, port, &provider);
            tracing::info!(
                "Antigravity token exchange: redirect_uri={callback_url}, state={}",
                body.state
            );
            let client_id = antigravity_client_id();
            let client_secret = antigravity_client_secret();
            let resp = state
                .http_client
                .post("https://oauth2.googleapis.com/token")
                .form(&[
                    ("client_id", client_id.as_str()),
                    ("client_secret", client_secret.as_str()),
                    ("code", &body.code),
                    ("redirect_uri", &callback_url),
                    ("grant_type", "authorization_code"),
                ])
                .send()
                .await
                .map_err(|e| {
                    (
                        StatusCode::BAD_GATEWAY,
                        format!("Antigravity exchange: {e}"),
                    )
                })?;

            let token: Value = resp.json().await.map_err(|e| {
                (
                    StatusCode::BAD_GATEWAY,
                    format!("Antigravity response: {e}"),
                )
            })?;

            let access_token = token["access_token"].as_str().unwrap_or("").to_string();
            let refresh_token = token["refresh_token"].as_str().unwrap_or("").to_string();

            if access_token.is_empty() {
                let err_detail = token["error"].as_str().unwrap_or("");
                let err_desc = token["error_description"].as_str().unwrap_or("");
                return Err((
                    StatusCode::BAD_REQUEST,
                    if !err_detail.is_empty() {
                        format!("No access token received: {err_detail} — {err_desc}")
                    } else {
                        "No access token received".to_string()
                    },
                ));
            }

            // Run Antigravity onboarding to get project_id
            let project_id = antigravity_onboard(&state, &access_token)
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;

            // Update session
            if let Some(session) = OAUTH_SESSIONS.lock().unwrap().get_mut(&body.state) {
                session.access_token = Some(access_token.clone());
                session.refresh_token = if refresh_token.is_empty() {
                    None
                } else {
                    Some(refresh_token.clone())
                };
                session.expires_at = token["expires_in"].as_u64().map(|seconds| {
                    (blackrouter_common::unix_timestamp().saturating_add(seconds)).to_string()
                });
                session.project_id = Some(project_id.clone());
                session.status = "done".to_string();
            }

            Ok(Json(OAuthPollResponse {
                status: "done".to_string(),
                access_token: Some(access_token),
                refresh_token: if refresh_token.is_empty() {
                    None
                } else {
                    Some(refresh_token)
                },
                token_expires_at: token["expires_in"].as_u64().map(|seconds| {
                    (blackrouter_common::unix_timestamp().saturating_add(seconds)).to_string()
                }),
                email: None,
                project_id: Some(project_id),
                error: None,
            }))
        }

        "codex" | "openai" => {
            let callback_url = session_redirect_uri(&body.state, port, "openai");

            // Get code_verifier from session
            let code_verifier = {
                let sessions = OAUTH_SESSIONS.lock().unwrap();
                sessions
                    .get(&body.state)
                    .and_then(|s| s.code_verifier.clone())
            };

            let mut form_data = vec![
                ("client_id".to_string(), codex_client_id()),
                ("code".to_string(), body.code.clone()),
                ("redirect_uri".to_string(), callback_url),
                ("grant_type".to_string(), "authorization_code".to_string()),
            ];
            if let Some(cv) = &code_verifier {
                form_data.push(("code_verifier".to_string(), cv.clone()));
            }

            let resp = state
                .http_client
                .post("https://auth.openai.com/oauth/token")
                .form(
                    &form_data
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect::<Vec<_>>(),
                )
                .send()
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Codex exchange: {e}")))?;

            let token: Value = resp
                .json()
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Codex response: {e}")))?;

            let access_token = token["access_token"].as_str().unwrap_or("").to_string();

            if access_token.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "No access token received".to_string(),
                ));
            }

            // Extract email from id_token if present
            let email = token["id_token"].as_str().and_then(|jwt| {
                jwt.split('.').nth(1).and_then(|payload| {
                    let padded = format!("{}{}", payload, "=".repeat((4 - payload.len() % 4) % 4));
                    base64_decode(&padded).ok().and_then(|json_str| {
                        serde_json::from_str::<Value>(&json_str)
                            .ok()
                            .and_then(|v| v.get("email").and_then(|e| e.as_str().map(String::from)))
                    })
                })
            });

            if let Some(session) = OAUTH_SESSIONS.lock().unwrap().get_mut(&body.state) {
                session.access_token = Some(access_token.clone());
                session.email = email.clone();
                session.status = "done".to_string();
            }

            Ok(Json(OAuthPollResponse {
                status: "done".to_string(),
                access_token: Some(access_token),
                refresh_token: None,
                token_expires_at: None,
                email,
                project_id: None,
                error: None,
            }))
        }

        "github" => {
            // GitHub device flow: poll for token
            let device_code = {
                let sessions = OAUTH_SESSIONS.lock().unwrap();
                let session = sessions
                    .get(&body.state)
                    .ok_or((StatusCode::NOT_FOUND, "Session not found".to_string()))?;
                session.access_token.clone().unwrap_or_default()
            };

            let client_id = github_client_id();
            let resp = state
                .http_client
                .post("https://github.com/login/oauth/access_token")
                .header("Accept", "application/json")
                .json(&json!({
                    "client_id": client_id,
                    "device_code": device_code,
                    "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
                }))
                .send()
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, format!("GitHub poll: {e}")))?;

            let token: Value = resp
                .json()
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, format!("GitHub response: {e}")))?;

            if let Some(error) = token["error"].as_str() {
                if error == "authorization_pending" {
                    return Ok(Json(OAuthPollResponse {
                        status: "pending".to_string(),
                        access_token: None,
                        refresh_token: None,
                        token_expires_at: None,
                        email: None,
                        project_id: None,
                        error: None,
                    }));
                }
                return Err((
                    StatusCode::BAD_REQUEST,
                    token["error_description"]
                        .as_str()
                        .unwrap_or(error)
                        .to_string(),
                ));
            }

            let access_token = token["access_token"].as_str().unwrap_or("").to_string();

            if access_token.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "No access token received".to_string(),
                ));
            }

            // Get Copilot token from GitHub
            let copilot_token = get_github_copilot_token(&state, &access_token).await;

            if let Some(session) = OAUTH_SESSIONS.lock().unwrap().get_mut(&body.state) {
                session.access_token = Some(copilot_token.unwrap_or(access_token.clone()));
                session.status = "done".to_string();
            }

            Ok(Json(OAuthPollResponse {
                status: "done".to_string(),
                access_token: Some(access_token),
                refresh_token: None,
                token_expires_at: None,
                email: None,
                project_id: None,
                error: None,
            }))
        }

        _ => Err((
            StatusCode::BAD_REQUEST,
            format!("Unsupported provider: {}", provider),
        )),
    }
}

/// Poll for OAuth status (used by frontend)
/// GET /api/oauth/{provider}/status?state=...
pub async fn oauth_status(
    Path(_provider): Path<String>,
    Query(query): Query<OAuthStatusQuery>,
) -> Result<Json<OAuthPollResponse>, (StatusCode, String)> {
    let sessions = OAUTH_SESSIONS.lock().unwrap();
    let session = sessions
        .get(&query.state)
        .ok_or((StatusCode::NOT_FOUND, "Session not found".to_string()))?;

    Ok(Json(OAuthPollResponse {
        status: session.status.clone(),
        access_token: session.access_token.clone(),
        refresh_token: session.refresh_token.clone(),
        token_expires_at: session.expires_at.clone(),
        email: session.email.clone(),
        project_id: session.project_id.clone(),
        error: session.error.clone(),
    }))
}

// ── Antigravity Onboarding ────────────────────────────────────────────

async fn antigravity_onboard(state: &AppState, access_token: &str) -> Result<String, String> {
    let metadata = json!({
        "ideType": 9,
        "platform": antigravity_platform_enum(),
        "pluginType": 2
    });

    let load_resp = state
        .http_client
        .post("https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "google-api-nodejs-client/9.15.1")
        .header(
            "X-Goog-Api-Client",
            "google-cloud-sdk vscode_cloudshelleditor/0.1",
        )
        .header("Client-Metadata", metadata.to_string())
        .json(&json!({"metadata": metadata}))
        .send()
        .await
        .map_err(|e| format!("loadCodeAssist failed: {e}"))?;

    if !load_resp.status().is_success() {
        let status = load_resp.status();
        let body = load_resp.text().await.unwrap_or_default();
        return Err(format!("loadCodeAssist HTTP {}: {}", status.as_u16(), body));
    }

    let data: Value = load_resp
        .json()
        .await
        .map_err(|e| format!("loadCodeAssist invalid JSON: {e}"))?;

    let project_id = data.get("cloudaicompanionProject")
        .and_then(|p| p.get("id").and_then(Value::as_str).or_else(|| p.as_str()))
        .map(String::from)
        .ok_or_else(|| "No cloudaicompanionProject found. Ensure you have a GCP project with Gemini Code Assist enabled.".to_string())?;

    let tier_id = data
        .get("allowedTiers")
        .and_then(Value::as_array)
        .and_then(|tiers| {
            tiers
                .iter()
                .find(|t| t.get("isDefault").and_then(Value::as_bool).unwrap_or(false))
        })
        .and_then(|t| t.get("id").and_then(Value::as_str))
        .unwrap_or("legacy-tier")
        .to_string();

    tracing::info!(
        "Antigravity loadCodeAssist: project={}, tier={}",
        project_id,
        tier_id
    );

    for attempt in 0..10u32 {
        let onboard_resp = state
            .http_client
            .post("https://cloudcode-pa.googleapis.com/v1internal:onboardUser")
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .header("User-Agent", "google-api-nodejs-client/9.15.1")
            .header(
                "X-Goog-Api-Client",
                "google-cloud-sdk vscode_cloudshelleditor/0.1",
            )
            .header("Client-Metadata", metadata.to_string())
            .json(&json!({"tierId": tier_id, "metadata": metadata}))
            .send()
            .await
            .map_err(|e| format!("onboardUser attempt {}: {}", attempt + 1, e))?;

        if !onboard_resp.status().is_success() {
            let status = onboard_resp.status();
            let body = onboard_resp.text().await.unwrap_or_default();
            return Err(format!("onboardUser HTTP {}: {}", status.as_u16(), body));
        }

        let result: Value = onboard_resp
            .json()
            .await
            .map_err(|e| format!("onboardUser invalid JSON: {e}"))?;

        if result.get("done").and_then(Value::as_bool).unwrap_or(false) {
            let final_project = result
                .get("response")
                .and_then(|r| r.get("cloudaicompanionProject"))
                .and_then(|p| p.get("id").and_then(Value::as_str).or_else(|| p.as_str()))
                .unwrap_or(&project_id)
                .to_string();

            tracing::info!("Antigravity onboardUser done: project={}", final_project);
            return Ok(final_project);
        }

        tracing::debug!("Antigravity onboardUser pending, retrying...");
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }

    Err("Antigravity onboarding timeout after 10 attempts".to_string())
}

/// Map the current OS to Antigravity's platform enum value
/// Matches 9router-master: macos-x64=1, macos-arm64=2, linux-x64=3, linux-arm64=4, windows=5
fn antigravity_platform_enum() -> u32 {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("macos", "aarch64") => 2,
        ("macos", _) => 1,
        ("linux", "aarch64") => 4,
        ("linux", _) => 3,
        ("windows", _) => 5,
        _ => 0,
    }
}

// ── GitHub Copilot token ────────────────────────────────────────────────

async fn get_github_copilot_token(state: &AppState, github_token: &str) -> Option<String> {
    let resp = state
        .http_client
        .get("https://api.github.com/copilot_internal/v2/token")
        .header("Authorization", format!("Bearer {}", github_token))
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;
    let body: Value = resp.json().await.ok()?;
    body.get("token").and_then(|t| t.as_str().map(String::from))
}

fn base64_decode(input: &str) -> Result<String, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}
