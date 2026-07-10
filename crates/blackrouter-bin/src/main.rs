use anyhow::Context;
use blackrouter_api::auth;
use blackrouter_api::{build_router, AppState};
use blackrouter_config::AppConfig;
use blackrouter_storage::Storage;
use blackrouter_telegram::{TelegramBot, TelegramBotConfig, TelegramRuntime};
use tokio::net::TcpListener;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AppConfig::load().context("failed to load BlackRouter config")?;
    init_tracing(&config.log_level)?;

    // Validate control-plane configuration at startup.
    auth::validate_control_config(&config).map_err(|s| anyhow::anyhow!("{s}"))?;

    let storage = Storage::new(config.database_path.clone());
    let storage_status = storage
        .initialize()
        .context("failed to initialize storage")?;

    tracing::info!(
        service = "blackrouter",
        host = %config.host,
        port = config.port,
        database = %storage.database_path().display(),
        schema_compatible = storage_status.schema_compatible,
        "starting BlackRouter Rust runtime"
    );

    let bind_addr = config.bind_addr().context("failed to build bind address")?;
    let app_state = AppState::new(config.clone(), storage.clone()).await;
    let app = build_router(app_state.clone());
    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind {bind_addr}"))?;

    tracing::info!(%bind_addr, "BlackRouter listening");

    // OpenTelemetry traces are wired into the tracing subscriber when
    // OTEL_EXPORTER_OTLP_ENDPOINT is set (see blackrouter_api::telemetry).
    if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok() {
        tracing::info!("OpenTelemetry tracing enabled (OTLP export)");
    }

    // Start Telegram bot if configured
    let telegram_handle = if config.telegram.enabled {
        if let Some(bot_token) = &config.telegram.bot_token {
            let bot_config = TelegramBotConfig {
                bot_token: bot_token.clone(),
                admin_ids: config.telegram.admin_ids.clone(),
                webhook_url: config.telegram.webhook_url.clone(),
                polling_interval_ms: 1000,
                max_connections: 40,
            };

            match TelegramBot::new(bot_config) {
                Ok(bot) => {
                    tracing::info!("Starting Telegram bot");

                    let runtime =
                        TelegramRuntime::new(storage.clone(), config.telegram.admin_ids.clone());

                    let handle = tokio::spawn(async move {
                        if let Err(e) = runtime.start_polling(bot).await {
                            tracing::error!("Telegram bot error: {}", e);
                        }
                    });

                    Some(handle)
                }
                Err(e) => {
                    tracing::error!("Failed to create Telegram bot: {}", e);
                    None
                }
            }
        } else {
            tracing::warn!("Telegram enabled but no bot token configured");
            None
        }
    } else {
        tracing::info!("Telegram bot disabled");
        None
    };

    // Start HTTP server
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .context("BlackRouter server failed")
    });

    // Wait for server and optionally telegram
    if let Some(telegram_handle) = telegram_handle {
        tokio::select! {
            result = server_handle => {
                result.context("Server task failed")??;
            }
            _ = telegram_handle => {
                tracing::info!("Telegram bot stopped");
            }
        }
    } else {
        // No telegram bot, just wait for server
        server_handle.await.context("Server task failed")??;
    }

    Ok(())
}

fn init_tracing(default_directive: &str) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(default_directive))
        .context("invalid tracing filter")?;

    // When OTEL_EXPORTER_OTLP_ENDPOINT is set, `init_layer()` returns an
    // OpenTelemetry layer; otherwise `None`. `Option<L>: Layer` (blanket
    // impl) keeps the subscriber type stable either way.
    let otel_layer = blackrouter_api::telemetry::init_layer();

    tracing_subscriber::registry()
        .with(otel_layer)
        .with(filter)
        .with(fmt::layer())
        .init();
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(%error, "failed to install Ctrl-C handler");
    }
    tracing::info!("shutdown signal received");
}
