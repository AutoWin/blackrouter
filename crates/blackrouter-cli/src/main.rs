mod antigravity;

use anyhow::{bail, Context};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use std::env;
use std::path::Path;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("login") => match args.get(2).map(|s| s.as_str()) {
            Some("antigravity") => antigravity::login("antigravity").await?,
            Some("gemini-cli") => antigravity::login("gemini-cli").await?,
            Some(provider) => {
                eprintln!("Unknown provider: {provider}");
                eprintln!("Supported providers: antigravity, gemini-cli");
                std::process::exit(1);
            }
            None => {
                eprintln!("Usage: blackrouter-cli login <provider>");
                eprintln!("Supported providers: antigravity, gemini-cli");
                std::process::exit(1);
            }
        },
        Some("doctor") => print_remote_json("/api/doctor").await?,
        Some("migrate") => {
            let config = blackrouter_config::AppConfig::load()?;
            let path = args
                .get(2)
                .map(std::path::PathBuf::from)
                .unwrap_or(config.database_path);
            let storage = blackrouter_storage::Storage::new(path);
            let status = storage.initialize().context("database migration failed")?;
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        Some("config") if args.get(2).map(String::as_str) == Some("apply") => {
            let file = args
                .get(3)
                .context("usage: blackrouter-cli config apply <file>")?;
            let body = read_json(file)?;
            let value =
                control_request(reqwest::Method::PUT, "/api/setup/config", Some(body)).await?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        Some("usage") if args.get(2).map(String::as_str) == Some("export") => {
            let value = control_request(reqwest::Method::GET, "/api/usage", None).await?;
            let output = serde_json::to_string_pretty(&value)?;
            if let Some(file) = args.get(3) {
                std::fs::write(file, format!("{output}\n"))
                    .with_context(|| format!("failed to write {file}"))?;
                println!("Wrote {file}");
            } else {
                println!("{output}");
            }
        }
        Some("--help") | Some("-h") | None => {
            println!("BlackRouter CLI");
            println!();
            println!("USAGE:");
            println!("    blackrouter-cli login <provider>");
            println!("    blackrouter-cli doctor");
            println!("    blackrouter-cli migrate [database-path]");
            println!("    blackrouter-cli config apply <file>");
            println!("    blackrouter-cli usage export [file]");
            println!();
            println!("PROVIDERS:");
            println!("    antigravity    Google Antigravity (Gemini Code Assist)");
            println!("    gemini-cli     Google Gemini CLI (Gemini API)");
        }
        Some(unknown) => {
            eprintln!("Unknown command: {unknown}");
            eprintln!("Run 'blackrouter-cli --help' for usage.");
            std::process::exit(1);
        }
    }

    Ok(())
}

fn read_json(path: impl AsRef<Path>) -> anyhow::Result<Value> {
    let path = path.as_ref();
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("invalid JSON in {}", path.display()))
}

fn base_url() -> String {
    env::var("BLACKROUTER_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:20129".to_string())
        .trim_end_matches('/')
        .to_string()
}

async fn control_request(
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<Value> {
    let client = reqwest::Client::new();
    let mut request = client
        .request(method, format!("{}{path}", base_url()))
        .header(CONTENT_TYPE, "application/json");
    if let Ok(token) = env::var("BLACKROUTER_CONTROL_TOKEN") {
        request = request.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.context("BlackRouter request failed")?;
    let status = response.status();
    let bytes = response.bytes().await?;
    let value = serde_json::from_slice::<Value>(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    if !status.is_success() {
        bail!("BlackRouter returned {status}: {value}");
    }
    Ok(value)
}

async fn print_remote_json(path: &str) -> anyhow::Result<()> {
    let value = control_request(reqwest::Method::GET, path, None).await?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
