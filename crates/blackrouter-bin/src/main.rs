use anyhow::Context;
use blackrouter_api::{build_router, AppState};
use blackrouter_config::AppConfig;
use blackrouter_storage::Storage;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AppConfig::load().context("failed to load BlackRouter config")?;
    init_tracing(&config.log_level)?;

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
    let app = build_router(AppState::new(config, storage));
    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind {bind_addr}"))?;

    tracing::info!(%bind_addr, "BlackRouter listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("BlackRouter server failed")?;

    Ok(())
}

fn init_tracing(default_directive: &str) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(default_directive))
        .context("invalid tracing filter")?;

    tracing_subscriber::fmt().with_env_filter(filter).init();
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(%error, "failed to install Ctrl-C handler");
    }
    tracing::info!("shutdown signal received");
}
