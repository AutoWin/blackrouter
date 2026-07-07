use anyhow::{bail, Context};
use axum::extract::Query;
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

// ── OAuth Configuration (matches 9router-master) ─────────────────────

// Antigravity credentials (Gemini Code Assist)
const ANTIGRAVITY_SCOPES: &str = "\
    https://www.googleapis.com/auth/cloud-platform \
    https://www.googleapis.com/auth/userinfo.email \
    https://www.googleapis.com/auth/userinfo.profile \
    https://www.googleapis.com/auth/cclog \
    https://www.googleapis.com/auth/experimentsandconfigs";

// Gemini CLI credentials (Gemini API)
const GEMINI_SCOPES: &str = "\
    https://www.googleapis.com/auth/cloud-platform \
    https://www.googleapis.com/auth/userinfo.email \
    https://www.googleapis.com/auth/userinfo.profile";

// Common endpoints
const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const USER_INFO_URL: &str = "https://www.googleapis.com/oauth2/v1/userinfo";
const LOAD_CODE_ASSIST_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
const ONBOARD_USER_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal:onboardUser";

const USER_AGENT: &str = "google-api-nodejs-client/9.15.1";
const API_CLIENT: &str = "google-cloud-sdk vscode_cloudshelleditor/0.1";

fn oauth_env(key: &'static str) -> anyhow::Result<String> {
    std::env::var(key)
        .map(|value| value.trim().to_string())
        .ok()
        .filter(|value| !value.is_empty())
        .with_context(|| format!("{key} is required for this OAuth login"))
}

// ── Platform enum (matches 9router-master, arch-aware) ────────────────

fn platform_enum() -> u32 {
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

fn client_metadata() -> String {
    serde_json::to_string(&json!({
        "ideType": 9,
        "platform": platform_enum(),
        "pluginType": 2
    }))
    .unwrap_or_default()
}

// ── Callback data ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

// ── Main login flow ───────────────────────────────────────────────────

pub async fn login(provider: &str) -> anyhow::Result<()> {
    let (client_id, client_secret, scopes, is_antigravity) = match provider {
        "antigravity" => (
            oauth_env("OAUTH_ANTIGRAVITY_CLIENT_ID")?,
            oauth_env("OAUTH_ANTIGRAVITY_CLIENT_SECRET")?,
            ANTIGRAVITY_SCOPES,
            true,
        ),
        "gemini-cli" => (
            oauth_env("OAUTH_GEMINI_CLI_CLIENT_ID")?,
            oauth_env("OAUTH_GEMINI_CLI_CLIENT_SECRET")?,
            GEMINI_SCOPES,
            false,
        ),
        _ => bail!("Unknown provider: {provider}"),
    };

    let provider_label = if is_antigravity {
        "Antigravity"
    } else {
        "Gemini CLI"
    };

    println!("\u{1f510} {provider_label} OAuth Login");
    println!(
        "\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}"
    );
    println!();

    // 1. Start local server for callback
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("failed to bind local server")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://localhost:{port}/callback");

    println!("\u{1f4e1} Local server started on port {port}");

    // 2. Generate state for CSRF protection
    let state = generate_state();

    // 3. Build authorization URL
    let auth_url = format!(
        "{AUTHORIZE_URL}?client_id={client_id}\
        &response_type=code\
        &redirect_uri={}\
        &scope={}\
        &state={state}\
        &access_type=offline\
        &prompt=consent",
        urlencoding(&redirect_uri),
        urlencoding(scopes),
    );

    // 4. Set up callback handler
    let callback_data: Arc<Mutex<Option<CallbackParams>>> = Arc::new(Mutex::new(None));
    let callback_data_clone = callback_data.clone();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let shutdown_tx = Arc::new(Mutex::new(Some(shutdown_tx)));

    let app = Router::new().route(
        "/callback",
        get(move |Query(params): Query<CallbackParams>| {
            let data = callback_data_clone.clone();
            let shutdown = shutdown_tx.clone();
            async move {
                let html = if params.error.is_some() {
                    r#"<!DOCTYPE html><html><head><title>OAuth Error</title></head>
<body style="font-family:system-ui;max-width:500px;margin:100px auto;text-align:center">
<h1 style="color:#b5453f">\u{274c} OAuth Failed</h1>
<p>The authorization was denied or an error occurred.</p>
<p>You can close this window.</p>
</body></html>"#
                } else {
                    r#"<!DOCTYPE html><html><head><title>OAuth Success</title></head>
<body style="font-family:system-ui;max-width:500px;margin:100px auto;text-align:center">
<h1 style="color:#2f7d5b">\u{2705} Authorization Complete</h1>
<p>You can close this window and return to the terminal.</p>
<script>window.close();</script>
</body></html>"#
                };

                *data.lock().unwrap() = Some(params);
                if let Some(tx) = shutdown.lock().unwrap().take() {
                    let _ = tx.send(());
                }
                Html(html)
            }
        }),
    );

    // 5. Start server in background
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                shutdown_rx.await.ok();
            })
            .await
    });

    // 6. Open browser
    println!("\u{1f310} Opening browser for authentication...\n");
    println!("   If browser doesn't open, visit:");
    println!("   {auth_url}\n");

    if let Err(e) = open::that(&auth_url) {
        eprintln!("\u{26a0}\u{fe0f}  Failed to open browser: {e}");
        eprintln!("   Please open the URL manually.");
    }

    // 7. Wait for callback
    println!("\u{23f3} Waiting for authorization...");

    let params = loop {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let data = callback_data.lock().unwrap();
        if let Some(params) = data.as_ref() {
            break CallbackParams {
                code: params.code.clone(),
                state: params.state.clone(),
                error: params.error.clone(),
            };
        }
    };

    let _ = server_handle.await;

    if let Some(error) = &params.error {
        bail!("OAuth error: {error}");
    }

    let code = params.code.context("No authorization code received")?;
    println!("\u{2705} Authorization code received\n");

    // 8. Exchange code for tokens
    println!("\u{1f504} Exchanging code for tokens...");
    let client = reqwest::Client::new();

    let token_resp = client
        .post(TOKEN_URL)
        .form(&vec![
            ("grant_type", "authorization_code".to_string()),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code.clone()),
            ("redirect_uri", redirect_uri.clone()),
        ])
        .send()
        .await
        .context("token exchange request failed")?;

    if !token_resp.status().is_success() {
        let error_text = token_resp.text().await.unwrap_or_default();
        bail!("Token exchange failed: {error_text}");
    }

    let tokens: serde_json::Value = token_resp
        .json()
        .await
        .context("failed to parse token response")?;

    let access_token = tokens["access_token"]
        .as_str()
        .context("no access_token in response")?;
    let _refresh_token = tokens["refresh_token"].as_str().unwrap_or("");

    println!("\u{2705} Tokens received\n");

    // 9. Fetch user info
    println!("\u{1f464} Fetching user info...");
    let userinfo_resp = client
        .get(format!("{USER_INFO_URL}?alt=json"))
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
        .context("user info request failed")?;

    let userinfo: serde_json::Value = userinfo_resp.json().await.unwrap_or_default();
    let email = userinfo["email"].as_str().unwrap_or("unknown");
    println!("   Email: {email}\n");

    // 10. LoadCodeAssist
    println!("\u{1f4cb} Loading Code Assist configuration...");
    let metadata = client_metadata();
    let load_headers = build_api_headers(access_token, &metadata);

    let metadata_val: serde_json::Value = serde_json::from_str(&metadata).unwrap_or(json!({}));

    let load_resp = client
        .post(LOAD_CODE_ASSIST_URL)
        .headers(load_headers.clone())
        .json(&json!({ "metadata": metadata_val }))
        .send()
        .await
        .context("loadCodeAssist request failed")?;

    if !load_resp.status().is_success() {
        let error_text = load_resp.text().await.unwrap_or_default();
        bail!("loadCodeAssist failed: {error_text}");
    }

    let load_data: serde_json::Value = load_resp
        .json()
        .await
        .context("failed to parse loadCodeAssist response")?;

    let project_id = load_data
        .get("cloudaicompanionProject")
        .and_then(|p| {
            p.get("id")
                .and_then(serde_json::Value::as_str)
                .or_else(|| p.as_str())
        })
        .unwrap_or("");

    let tier_id = load_data
        .get("allowedTiers")
        .and_then(serde_json::Value::as_array)
        .and_then(|tiers| {
            tiers.iter().find(|t| {
                t.get("isDefault")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
            })
        })
        .and_then(|t| t.get("id").and_then(serde_json::Value::as_str))
        .unwrap_or("legacy-tier");

    if project_id.is_empty() {
        bail!("No GCP project found. Please ensure you have a GCP project with Gemini Code Assist enabled.");
    }

    println!("   Project: {project_id}");
    println!("   Tier: {tier_id}\n");

    // 11. Onboard user (Antigravity only)
    if is_antigravity {
        println!("\u{1f680} Onboarding to Gemini Code Assist...");

        let mut onboarded = false;
        for attempt in 0..10u32 {
            let metadata_onboard: serde_json::Value =
                serde_json::from_str(&metadata).unwrap_or(json!({}));

            let onboard_resp = client
                .post(ONBOARD_USER_URL)
                .headers(load_headers.clone())
                .json(&json!({ "tierId": tier_id, "metadata": metadata_onboard }))
                .send()
                .await;

            match onboard_resp {
                Ok(resp) if resp.status().is_success() => {
                    let result: serde_json::Value = resp.json().await.unwrap_or_default();
                    if result
                        .get("done")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                    {
                        onboarded = true;
                        break;
                    }
                }
                _ => {}
            }

            if attempt < 9 {
                println!("   Attempt {}/10 \u{2014} waiting...", attempt + 1);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }

        if onboarded {
            println!("\u{2705} Onboarding complete\n");
        } else {
            println!("\u{26a0}\u{fe0f}  Onboarding timeout (tokens still usable)\n");
        }
    }

    // 12. Fetch available models
    println!("\u{1f4e6} Fetching available models...");
    let models = fetch_available_models(&client, access_token, is_antigravity).await;
    if !models.is_empty() {
        println!("   Available models:");
        for m in &models {
            println!("   \u{2022} {provider}/{m}");
        }
        println!();
    } else {
        println!("   No models found\n");
    }

    // 13. Save to BlackRouter server
    let server_url =
        std::env::var("BLACKROUTER_URL").unwrap_or_else(|_| "http://localhost:20130".to_string());
    let api_key = std::env::var("BLACKROUTER_API_KEY").unwrap_or_default();

    let format = if is_antigravity {
        "antigravity"
    } else {
        "gemini-cli"
    };
    let base_url = "https://cloudcode-pa.googleapis.com";

    if api_key.is_empty() {
        println!(
            "\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}"
        );
        println!("\u{2705} {provider_label} login successful!");
        println!();
        println!("To save this connection to BlackRouter, set:");
        println!("   export BLACKROUTER_API_KEY=<your-api-key>");
        println!("   blackrouter-cli login {provider}");
        println!();
        println!("Or add manually via the setup UI at:");
        println!("   {server_url}/setup");
    } else {
        println!("\u{1f4be} Saving connection to BlackRouter...");
        let save_resp = client
            .post(format!("{server_url}/api/setup/providers"))
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&json!({
                "provider": provider,
                "authType": "oauth",
                "name": format!("{provider_label} ({email})"),
                "email": email,
                "isActive": true,
                "data": {
                    "format": format,
                    "baseUrl": base_url,
                    "accessToken": access_token,
                    "refreshToken": tokens["refresh_token"],
                    "projectId": project_id,
                    "models": &models,
                }
            }))
            .send()
            .await
            .context("failed to save connection")?;

        if save_resp.status().is_success() {
            println!("\u{2705} Connection saved to BlackRouter!\n");
        } else {
            let error_text = save_resp.text().await.unwrap_or_default();
            println!("\u{26a0}\u{fe0f}  Failed to save connection: {error_text}");
            println!("   You can add it manually via the setup UI at {server_url}/setup\n");
        }
    }

    println!(
        "\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}"
    );
    println!("Provider: {provider_label}");
    println!("Email:    {email}");
    println!("Project:  {project_id}");
    println!("Server:   {server_url}");
    println!("Models:   {} available", models.len());
    println!(
        "\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}"
    );

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────

fn generate_state() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("{:08x}", nanos)
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

fn build_api_headers(access_token: &str, metadata: &str) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "Authorization",
        format!("Bearer {access_token}").parse().unwrap(),
    );
    headers.insert("Content-Type", "application/json".parse().unwrap());
    headers.insert("User-Agent", USER_AGENT.parse().unwrap());
    headers.insert("X-Goog-Api-Client", API_CLIENT.parse().unwrap());
    headers.insert("Client-Metadata", metadata.parse().unwrap());
    headers.insert("x-request-source", "local".parse().unwrap());
    headers
}

// ── Model fetching ────────────────────────────────────────────────────

/// Fetch available models
async fn fetch_available_models(
    client: &reqwest::Client,
    access_token: &str,
    is_antigravity: bool,
) -> Vec<String> {
    // Try Gemini API first
    let resp = client
        .get("https://generativelanguage.googleapis.com/v1beta/models")
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await;

    if let Ok(resp) = resp {
        if resp.status().is_success() {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                if let Some(models) = data.get("models").and_then(serde_json::Value::as_array) {
                    let mut result: Vec<String> = models
                        .iter()
                        .filter_map(|m| {
                            let name = m.get("name")?.as_str()?;
                            let id = name.strip_prefix("models/").unwrap_or(name);
                            if id.starts_with("gemini") {
                                Some(id.to_string())
                            } else {
                                None
                            }
                        })
                        .collect();
                    result.sort();
                    if !result.is_empty() {
                        return result;
                    }
                }
            }
        }
    }

    // Fallback: built-in models
    if is_antigravity {
        vec![
            "gemini-3-flash-agent".to_string(),
            "gemini-3.5-flash-low".to_string(),
            "gemini-3.5-flash-extra-low".to_string(),
            "gemini-pro-agent".to_string(),
            "gemini-3.1-pro-low".to_string(),
            "claude-sonnet-4-6".to_string(),
            "claude-opus-4-6-thinking".to_string(),
            "gpt-oss-120b-medium".to_string(),
            "gemini-3-flash".to_string(),
            "gemini-3-flash-preview".to_string(),
            "gemini-3-pro-preview".to_string(),
        ]
    } else {
        // Gemini CLI models (standard Gemini API)
        vec![
            "gemini-2.0-flash-lite".to_string(),
            "gemini-2.0-flash".to_string(),
            "gemini-2.5-flash".to_string(),
            "gemini-1.5-flash".to_string(),
            "gemini-1.5-pro".to_string(),
            "gemini-2.5-pro".to_string(),
            "gemini-3-flash-preview".to_string(),
            "gemini-3-pro-preview".to_string(),
        ]
    }
}
