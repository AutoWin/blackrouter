use anyhow::Context;
use blackrouter_api::{build_router, AppState};
use blackrouter_config::AppConfig;
use blackrouter_storage::Storage;
use blackrouter_telegram::{TelegramBot, TelegramBotConfig, TelegramCommand};
use std::sync::Arc;
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
    let app_state = AppState::new(config.clone(), storage.clone());
    let app = build_router(app_state.clone());
    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind {bind_addr}"))?;

    tracing::info!(%bind_addr, "BlackRouter listening");

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

                    let bot = Arc::new(bot);
                    let storage = storage.clone();

                    let handle = tokio::spawn(async move {
                        let bot_clone = bot.clone();
                        let storage_clone = storage.clone();

                        let handler = move |command: TelegramCommand, chat_id: i64| {
                            let bot = bot_clone.clone();
                            let storage = storage_clone.clone();

                            Box::pin(async move {
                                handle_telegram_command(&bot, &storage, command, chat_id).await
                            })
                                as std::pin::Pin<
                                    Box<dyn std::future::Future<Output = String> + Send>,
                                >
                        };

                        if let Err(e) = bot.start_polling(handler).await {
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

async fn handle_telegram_command(
    _bot: &TelegramBot,
    storage: &Storage,
    command: TelegramCommand,
    _chat_id: i64,
) -> String {
    match command {
        TelegramCommand::Start => {
            format!("🤖 Welcome to BlackRouter!\n\nType /help for available commands.")
        }
        TelegramCommand::Help => TelegramBot::help_message(),
        TelegramCommand::Status => match storage.status() {
            Ok(status) => {
                let table_count = status.table_counts.len();
                format!(
                        "📊 <b>BlackRouter Status</b>\n\n✅ Status: Running\n📁 Database: {}\n📋 Tables: {}\n🔌 Schema Compatible: {}",
                        status.database_path.display(),
                        table_count,
                        if status.schema_compatible { "Yes" } else { "No" }
                    )
            }
            Err(e) => format!("❌ Error getting status: {}", e),
        },
        TelegramCommand::Health => match storage.status() {
            Ok(status) => {
                if status.schema_compatible {
                    "✅ <b>Health Check</b>\n\nStatus: Healthy".to_string()
                } else {
                    "⚠️ <b>Health Check</b>\n\nStatus: Degraded\nReason: Schema incompatible"
                        .to_string()
                }
            }
            Err(e) => format!("❌ <b>Health Check Failed</b>\n\nError: {}", e),
        },
        TelegramCommand::Version => {
            let build_info = blackrouter_common::BuildInfo::default();
            format!(
                "📦 <b>BlackRouter Version</b>\n\nName: {}\nVersion: {}\nRuntime: {}",
                build_info.name, build_info.version, build_info.rust_runtime
            )
        }
        TelegramCommand::Providers => match storage.list_provider_connections() {
            Ok(providers) => {
                if providers.is_empty() {
                    "📭 No providers configured".to_string()
                } else {
                    let mut msg = "🔌 <b>Providers</b>\n\n".to_string();
                    for p in &providers {
                        let status = if p.is_active { "✅" } else { "❌" };
                        msg.push_str(&format!(
                            "{} {} ({}) - Priority: {}\n",
                            status,
                            p.name.as_deref().unwrap_or(&p.provider),
                            p.provider,
                            p.priority.unwrap_or(0)
                        ));
                    }
                    msg
                }
            }
            Err(e) => format!("❌ Error listing providers: {}", e),
        },
        TelegramCommand::Provider { provider_id } => {
            match storage.get_provider_connection_raw(&provider_id) {
                Ok(provider) => {
                    let status = if provider.is_active {
                        "✅ Active"
                    } else {
                        "❌ Inactive"
                    };
                    format!(
                        "🔌 <b>Provider: {}</b>\n\nID: {}\nStatus: {}\nAuth Type: {}\nPriority: {}\nCreated: {}",
                        provider.name.as_deref().unwrap_or(&provider.provider),
                        provider.id,
                        status,
                        provider.auth_type,
                        provider.priority.unwrap_or(0),
                        provider.created_at
                    )
                }
                Err(e) => format!("❌ Provider not found: {}", e),
            }
        }
        TelegramCommand::Models { provider_id } => {
            match storage.get_provider_connection_raw(&provider_id) {
                Ok(provider) => {
                    let models = provider
                        .data
                        .get("models")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_else(|| "No models fetched".to_string());

                    format!(
                        "🤖 <b>Models for {}</b>\n\n{}",
                        provider.name.as_deref().unwrap_or(&provider.provider),
                        models
                    )
                }
                Err(e) => format!("❌ Provider not found: {}", e),
            }
        }
        TelegramCommand::Combos => match storage.list_combos() {
            Ok(combos) => {
                if combos.is_empty() {
                    "📭 No combos configured".to_string()
                } else {
                    let mut msg = "🎯 <b>Combos</b>\n\n".to_string();
                    for c in &combos {
                        msg.push_str(&format!(
                            "• {} ({}) - {} models\n",
                            c.name,
                            c.kind,
                            c.models.len()
                        ));
                    }
                    msg
                }
            }
            Err(e) => format!("❌ Error listing combos: {}", e),
        },
        TelegramCommand::Combo { combo_name } => match storage.resolve_model_route(&combo_name) {
            Ok(route) => match route {
                blackrouter_core::RouteKind::Single(model) => {
                    format!(
                        "🎯 <b>Route: {}</b>\n\nType: Single\nProvider: {}\nModel: {}",
                        combo_name, model.provider, model.model
                    )
                }
                blackrouter_core::RouteKind::Combo { name, models } => {
                    let models_str = models
                        .iter()
                        .map(|m| format!("• {}/{}", m.provider, m.model))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!(
                        "🎯 <b>Combo: {}</b>\n\nType: Fallback\nModels:\n{}",
                        name, models_str
                    )
                }
            },
            Err(e) => format!("❌ Combo not found: {}", e),
        },
        TelegramCommand::Usage { range } => {
            // TODO: Implement usage tracking
            format!(
                "📈 <b>Usage Statistics</b>\n\nPeriod: {}\n\n⚠️ Usage tracking not implemented yet",
                match range {
                    blackrouter_telegram::UsageRange::Today => "Today",
                    blackrouter_telegram::UsageRange::SevenDays => "Last 7 Days",
                }
            )
        }
        TelegramCommand::Logs { limit } => {
            // TODO: Implement log retrieval
            format!(
                "📋 <b>Recent Logs</b>\n\nLimit: {}\n\n⚠️ Log retrieval not implemented yet",
                limit
            )
        }
        TelegramCommand::EnableProvider { provider_id } => {
            match storage.set_provider_connection_active(&provider_id, true) {
                Ok(_) => format!("✅ Provider {} enabled", provider_id),
                Err(e) => format!("❌ Failed to enable provider: {}", e),
            }
        }
        TelegramCommand::DisableProvider { provider_id } => {
            match storage.set_provider_connection_active(&provider_id, false) {
                Ok(_) => format!("✅ Provider {} disabled", provider_id),
                Err(e) => format!("❌ Failed to disable provider: {}", e),
            }
        }
        TelegramCommand::EnableConnection { connection_id } => {
            match storage.set_provider_connection_active(&connection_id, true) {
                Ok(_) => format!("✅ Connection {} enabled", connection_id),
                Err(e) => format!("❌ Failed to enable connection: {}", e),
            }
        }
        TelegramCommand::DisableConnection { connection_id } => {
            match storage.set_provider_connection_active(&connection_id, false) {
                Ok(_) => format!("✅ Connection {} disabled", connection_id),
                Err(e) => format!("❌ Failed to disable connection: {}", e),
            }
        }
        TelegramCommand::TestProvider { provider_id } => {
            // TODO: Implement provider testing
            format!(
                "🧪 <b>Test Provider</b>\n\nProvider: {}\n\n⚠️ Provider testing not implemented yet",
                provider_id
            )
        }
        TelegramCommand::TestConnection { connection_id } => {
            // TODO: Implement connection testing
            format!(
                "🧪 <b>Test Connection</b>\n\nConnection: {}\n\n⚠️ Connection testing not implemented yet",
                connection_id
            )
        }
        TelegramCommand::Rtk { enabled } => {
            // TODO: Implement RTK toggle
            format!(
                "⚡ <b>RTK Rate Limiting</b>\n\nStatus: {}\n\n⚠️ RTK toggle not implemented yet",
                if enabled { "Enabled" } else { "Disabled" }
            )
        }
        TelegramCommand::Reload => {
            // TODO: Implement config reload
            "🔄 <b>Reload Configuration</b>\n\n⚠️ Config reload not implemented yet".to_string()
        }
        TelegramCommand::Shutdown => {
            "🛑 <b>Shutdown</b>\n\n⚠️ Shutdown not implemented yet".to_string()
        }
        TelegramCommand::Link { code } => {
            // TODO: Implement account linking
            format!(
                "🔗 <b>Link Account</b>\n\nCode: {}\n\n⚠️ Account linking not implemented yet",
                code
            )
        }
    }
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
