use blackrouter_storage::{
    NewApiKey, NewCombo, NewProviderConnection, RawProviderConnection, Storage,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, error, info, warn};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum TelegramCommand {
    Start,
    Help,
    Status,
    Health,
    Version,
    ApiKeys,
    ApiKeyCreate {
        name: String,
    },
    Providers,
    Provider {
        provider_id: String,
    },
    ProviderAdd {
        provider: String,
        auth_type: String,
        api_key: Option<String>,
        base_url: String,
        format: String,
    },
    ProviderModelsRefresh {
        connection_id: String,
    },
    Models {
        provider_id: String,
    },
    Combos,
    Combo {
        combo_name: String,
    },
    ComboAdd {
        name: String,
        models: Vec<String>,
    },
    Usage {
        range: UsageRange,
    },
    Logs {
        limit: usize,
    },
    EnableProvider {
        provider_id: String,
    },
    DisableProvider {
        provider_id: String,
    },
    EnableConnection {
        connection_id: String,
    },
    DisableConnection {
        connection_id: String,
    },
    TestProvider {
        provider_id: String,
    },
    TestConnection {
        connection_id: String,
    },
    Rtk {
        enabled: bool,
    },
    Reload,
    Shutdown,
    Link {
        code: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum UsageRange {
    Today,
    SevenDays,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TelegramCommandError {
    #[error("empty command")]
    Empty,
    #[error("unknown command: {0}")]
    Unknown(String),
    #[error("missing argument: {0}")]
    MissingArgument(&'static str),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

impl TelegramCommand {
    pub fn parse(input: &str) -> Result<Self, TelegramCommandError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(TelegramCommandError::Empty);
        }

        let mut parts = trimmed.split_whitespace();
        let command = parts
            .next()
            .ok_or(TelegramCommandError::Empty)?
            .trim_start_matches('/')
            .split('@')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();

        match command.as_str() {
            "start" => Ok(Self::Start),
            "help" => Ok(Self::Help),
            "status" => Ok(Self::Status),
            "health" => Ok(Self::Health),
            "version" => Ok(Self::Version),
            "apikeys" => Ok(Self::ApiKeys),
            "apikey" => parse_apikey(&mut parts),
            "providers" => Ok(Self::Providers),
            "provider" => parse_provider(&mut parts),
            "models" => Ok(Self::Models {
                provider_id: required(&mut parts, "provider_id")?,
            }),
            "combos" => Ok(Self::Combos),
            "combo" => parse_combo(&mut parts),
            "usage" => parse_usage(parts.next()),
            "logs" => parse_logs(parts.next()),
            "enable" => parse_toggle(true, &mut parts),
            "disable" => parse_toggle(false, &mut parts),
            "test" => parse_test(&mut parts),
            "rtk" => parse_rtk(parts.next()),
            "reload" => Ok(Self::Reload),
            "shutdown" => Ok(Self::Shutdown),
            "link" => Ok(Self::Link {
                code: required(&mut parts, "code")?,
            }),
            other => Err(TelegramCommandError::Unknown(other.to_string())),
        }
    }

    pub fn requires_confirmation(&self) -> bool {
        matches!(
            self,
            Self::DisableProvider { .. }
                | Self::DisableConnection { .. }
                | Self::Rtk { enabled: false }
                | Self::Shutdown
        )
    }

    pub fn mutates_state(&self) -> bool {
        matches!(
            self,
            Self::EnableProvider { .. }
                | Self::ApiKeyCreate { .. }
                | Self::ProviderAdd { .. }
                | Self::ProviderModelsRefresh { .. }
                | Self::ComboAdd { .. }
                | Self::DisableProvider { .. }
                | Self::EnableConnection { .. }
                | Self::DisableConnection { .. }
                | Self::Rtk { .. }
                | Self::Reload
                | Self::Shutdown
                | Self::Link { .. }
        )
    }
}

fn parse_apikey<'a>(
    parts: &mut impl Iterator<Item = &'a str>,
) -> Result<TelegramCommand, TelegramCommandError> {
    match required(parts, "subcommand")?.as_str() {
        "create" => {
            let name = parts.collect::<Vec<_>>().join(" ");
            if name.trim().is_empty() {
                return Err(TelegramCommandError::MissingArgument("name"));
            }
            Ok(TelegramCommand::ApiKeyCreate { name })
        }
        other => Err(TelegramCommandError::InvalidArgument(other.to_string())),
    }
}

fn parse_provider<'a>(
    parts: &mut impl Iterator<Item = &'a str>,
) -> Result<TelegramCommand, TelegramCommandError> {
    let first = required(parts, "provider_id|subcommand")?;
    match first.as_str() {
        "add" => {
            let provider = required(parts, "provider")?;
            let auth_type = required(parts, "auth_type")?;
            let values = parts.collect::<Vec<_>>();
            let api_key = key_value(&values, "key").map(ToOwned::to_owned);
            let base_url = key_value(&values, "baseUrl")
                .or_else(|| key_value(&values, "base_url"))
                .ok_or(TelegramCommandError::MissingArgument("baseUrl=<url>"))?
                .to_string();
            let format = key_value(&values, "format").unwrap_or("openai").to_string();

            if auth_type != "none" && api_key.as_deref().unwrap_or_default().is_empty() {
                return Err(TelegramCommandError::MissingArgument("key=<secret>"));
            }

            Ok(TelegramCommand::ProviderAdd {
                provider,
                auth_type,
                api_key,
                base_url,
                format,
            })
        }
        "models-refresh" => Ok(TelegramCommand::ProviderModelsRefresh {
            connection_id: required(parts, "connection_id")?,
        }),
        provider_id => Ok(TelegramCommand::Provider {
            provider_id: provider_id.to_string(),
        }),
    }
}

fn parse_combo<'a>(
    parts: &mut impl Iterator<Item = &'a str>,
) -> Result<TelegramCommand, TelegramCommandError> {
    let first = required(parts, "combo_name|subcommand")?;
    match first.as_str() {
        "add" => {
            let name = required(parts, "name")?;
            let models = parts
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            if models.is_empty() {
                return Err(TelegramCommandError::MissingArgument("provider/model"));
            }
            Ok(TelegramCommand::ComboAdd { name, models })
        }
        combo_name => Ok(TelegramCommand::Combo {
            combo_name: combo_name.to_string(),
        }),
    }
}

fn key_value<'a>(values: &'a [&str], key: &str) -> Option<&'a str> {
    values.iter().find_map(|value| {
        let (candidate, raw_value) = value.split_once('=')?;
        if candidate.eq_ignore_ascii_case(key) {
            Some(raw_value.trim())
        } else {
            None
        }
    })
}

fn required<'a>(
    parts: &mut impl Iterator<Item = &'a str>,
    name: &'static str,
) -> Result<String, TelegramCommandError> {
    parts
        .next()
        .map(ToOwned::to_owned)
        .ok_or(TelegramCommandError::MissingArgument(name))
}

fn parse_usage(value: Option<&str>) -> Result<TelegramCommand, TelegramCommandError> {
    match value.unwrap_or("today").to_ascii_lowercase().as_str() {
        "today" => Ok(TelegramCommand::Usage {
            range: UsageRange::Today,
        }),
        "7d" | "7days" | "week" => Ok(TelegramCommand::Usage {
            range: UsageRange::SevenDays,
        }),
        other => Err(TelegramCommandError::InvalidArgument(other.to_string())),
    }
}

fn parse_logs(value: Option<&str>) -> Result<TelegramCommand, TelegramCommandError> {
    let limit = match value {
        Some(raw) => raw
            .parse::<usize>()
            .map_err(|_| TelegramCommandError::InvalidArgument(raw.to_string()))?,
        None => 20,
    };
    Ok(TelegramCommand::Logs {
        limit: limit.clamp(1, 100),
    })
}

fn parse_toggle<'a>(
    enabled: bool,
    parts: &mut impl Iterator<Item = &'a str>,
) -> Result<TelegramCommand, TelegramCommandError> {
    let target_type = required(parts, "target_type")?;
    let target_id = required(parts, "target_id")?;

    match (enabled, target_type.as_str()) {
        (true, "provider") => Ok(TelegramCommand::EnableProvider {
            provider_id: target_id,
        }),
        (false, "provider") => Ok(TelegramCommand::DisableProvider {
            provider_id: target_id,
        }),
        (true, "connection") => Ok(TelegramCommand::EnableConnection {
            connection_id: target_id,
        }),
        (false, "connection") => Ok(TelegramCommand::DisableConnection {
            connection_id: target_id,
        }),
        (_, other) => Err(TelegramCommandError::InvalidArgument(other.to_string())),
    }
}

fn parse_test<'a>(
    parts: &mut impl Iterator<Item = &'a str>,
) -> Result<TelegramCommand, TelegramCommandError> {
    let target_type = required(parts, "target_type")?;
    let target_id = required(parts, "target_id")?;

    match target_type.as_str() {
        "provider" => Ok(TelegramCommand::TestProvider {
            provider_id: target_id,
        }),
        "connection" => Ok(TelegramCommand::TestConnection {
            connection_id: target_id,
        }),
        other => Err(TelegramCommandError::InvalidArgument(other.to_string())),
    }
}

fn parse_rtk(value: Option<&str>) -> Result<TelegramCommand, TelegramCommandError> {
    match value.unwrap_or_default().to_ascii_lowercase().as_str() {
        "on" | "enable" | "enabled" => Ok(TelegramCommand::Rtk { enabled: true }),
        "off" | "disable" | "disabled" => Ok(TelegramCommand::Rtk { enabled: false }),
        "" => Err(TelegramCommandError::MissingArgument("on|off")),
        other => Err(TelegramCommandError::InvalidArgument(other.to_string())),
    }
}

pub fn is_authorized_chat(chat_id: i64, admin_ids: &[i64]) -> bool {
    admin_ids.contains(&chat_id)
}

// ============================================================
// Telegram Bot Runtime
// ============================================================

/// Telegram Bot API client and runtime
#[derive(Clone)]
pub struct TelegramBot {
    client: reqwest::Client,
    bot_token: String,
    api_base: String,
    admin_ids: Vec<i64>,
    _webhook_url: Option<String>,
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Telegram Bot API response for getUpdates
#[derive(Clone, Debug, Deserialize)]
pub struct UpdateResponse {
    pub ok: bool,
    pub result: Option<Vec<Update>>,
}

/// Telegram Update
#[derive(Clone, Debug, Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
    pub callback_query: Option<CallbackQuery>,
}

/// Telegram Message
#[derive(Clone, Debug, Deserialize)]
pub struct Message {
    pub message_id: i64,
    pub from: Option<User>,
    pub chat: Chat,
    pub date: i64,
    pub text: Option<String>,
}

/// Telegram User
#[derive(Clone, Debug, Deserialize)]
pub struct User {
    pub id: i64,
    pub is_bot: bool,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}

/// Telegram Chat
#[derive(Clone, Debug, Deserialize)]
pub struct Chat {
    pub id: i64,
    #[serde(rename = "type")]
    pub chat_type: String,
    pub title: Option<String>,
}

/// Telegram CallbackQuery
#[derive(Clone, Debug, Deserialize)]
pub struct CallbackQuery {
    pub id: String,
    pub from: User,
    pub message: Option<Message>,
    pub data: Option<String>,
}

/// Response handler for commands
pub type CommandHandler =
    Box<dyn Fn(TelegramCommand, i64) -> futures::future::BoxFuture<'static, String> + Send + Sync>;

/// Telegram Bot configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TelegramBotConfig {
    pub bot_token: String,
    pub admin_ids: Vec<i64>,
    pub webhook_url: Option<String>,
    pub polling_interval_ms: u64,
    pub max_connections: u32,
}

impl Default for TelegramBotConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            admin_ids: Vec::new(),
            webhook_url: None,
            polling_interval_ms: 1000,
            max_connections: 40,
        }
    }
}

/// Error type for Telegram bot operations
#[derive(Debug, Error)]
pub enum TelegramBotError {
    #[error("HTTP request failed: {0}")]
    Http(String),
    #[error("API error: {0}")]
    Api(String),
    #[error("Invalid configuration: {0}")]
    Config(String),
    #[error("Unauthorized chat: {0}")]
    Unauthorized(i64),
    #[error("Command parsing failed: {0}")]
    Command(#[from] TelegramCommandError),
}

fn telegram_http_error(operation: &'static str, error: reqwest::Error) -> TelegramBotError {
    let kind = if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_decode() {
        "decode"
    } else if error.is_body() {
        "body"
    } else if error.is_status() {
        "status"
    } else if error.is_request() {
        "request"
    } else if error.is_builder() {
        "builder"
    } else if error.is_redirect() {
        "redirect"
    } else {
        "transport"
    };

    let status = error
        .status()
        .map(|status| format!(", status {}", status.as_u16()))
        .unwrap_or_default();

    TelegramBotError::Http(format!("{operation} failed ({kind}{status})"))
}

#[derive(Clone)]
pub struct TelegramRuntime {
    storage: Storage,
    admin_ids: Vec<i64>,
    started_at: std::time::Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelegramRuntimeMessage {
    pub chat_id: i64,
    pub text: String,
    pub parse_mode: Option<&'static str>,
}

impl TelegramRuntime {
    pub fn new(storage: Storage, admin_ids: Vec<i64>) -> Self {
        Self {
            storage,
            admin_ids,
            started_at: std::time::Instant::now(),
        }
    }

    pub fn is_authorized(&self, chat_id: i64, user_id: Option<i64>) -> bool {
        self.admin_ids.is_empty()
            || self.admin_ids.contains(&chat_id)
            || user_id
                .map(|id| self.admin_ids.contains(&id))
                .unwrap_or(false)
    }

    pub async fn handle_update(
        &self,
        update: Update,
    ) -> Result<Option<TelegramRuntimeMessage>, TelegramBotError> {
        if let Some(message) = update.message {
            return Ok(self.handle_message(message).await);
        }

        if let Some(callback) = update.callback_query {
            let chat_id = callback.message.as_ref().map(|m| m.chat.id).unwrap_or(0);
            let user_id = Some(callback.from.id);
            if !self.is_authorized(chat_id, user_id) {
                return Err(TelegramBotError::Unauthorized(chat_id));
            }

            let text = match callback.data.as_deref() {
                Some(data) => format!("Callback received: {}", escape_html(data)),
                None => "Callback received".to_string(),
            };
            return Ok(Some(TelegramRuntimeMessage {
                chat_id,
                text,
                parse_mode: Some("HTML"),
            }));
        }

        Ok(None)
    }

    pub async fn dispatch_update(
        &self,
        bot: &TelegramBot,
        update: Update,
    ) -> Result<(), TelegramBotError> {
        match self.handle_update(update).await {
            Ok(Some(message)) => {
                bot.send_message(message.chat_id, &message.text, message.parse_mode)
                    .await?;
            }
            Ok(None) => {}
            Err(TelegramBotError::Unauthorized(chat_id)) => {
                let _ = bot
                    .send_message(chat_id, "Unauthorized. You are not an admin.", None)
                    .await;
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    pub async fn start_polling(&self, bot: TelegramBot) -> Result<(), TelegramBotError> {
        bot.running.store(true, std::sync::atomic::Ordering::SeqCst);

        info!("Starting Telegram bot runtime polling");
        let mut offset: Option<i64> = None;

        while bot.running.load(std::sync::atomic::Ordering::SeqCst) {
            match bot.get_updates(offset, 30).await {
                Ok(updates) => {
                    for update in updates {
                        offset = Some(update.update_id + 1);
                        if let Err(error) = self.dispatch_update(&bot, update).await {
                            warn!("Telegram update handling failed: {}", error);
                        }
                    }
                }
                Err(error) => {
                    error!("Failed to get Telegram updates: {}", error);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }

        info!("Telegram bot runtime polling stopped");
        Ok(())
    }

    async fn handle_message(&self, message: Message) -> Option<TelegramRuntimeMessage> {
        let text = message.text.as_deref()?.trim();
        if text.is_empty() {
            return None;
        }

        let chat_id = message.chat.id;
        let user_id = message.from.as_ref().map(|user| user.id);
        if !self.is_authorized(chat_id, user_id) {
            warn!("Unauthorized Telegram access attempt from chat {}", chat_id);
            return Some(TelegramRuntimeMessage {
                chat_id,
                text: "Unauthorized. You are not an admin.".to_string(),
                parse_mode: None,
            });
        }

        let response = match TelegramCommand::parse(text) {
            Ok(command) => {
                debug!("Received Telegram command: {:?}", command);
                self.handle_command(command, chat_id).await
            }
            Err(TelegramCommandError::Empty) => return None,
            Err(TelegramCommandError::Unknown(command)) => format!(
                "Unknown command: /{}\n\nType /help for available commands.",
                escape_html(&command)
            ),
            Err(error) => format!("Error: {}", escape_html(&error.to_string())),
        };

        Some(TelegramRuntimeMessage {
            chat_id,
            text: response,
            parse_mode: Some("HTML"),
        })
    }

    pub async fn handle_command(&self, command: TelegramCommand, chat_id: i64) -> String {
        match command {
            TelegramCommand::Start => {
                "Welcome to BlackRouter!\n\nType /help for available commands.".to_string()
            }
            TelegramCommand::Help => TelegramBot::help_message(),
            TelegramCommand::Status => self.status_message(),
            TelegramCommand::Health => self.health_message(),
            TelegramCommand::Version => {
                let build_info = blackrouter_common::BuildInfo::default();
                format!(
                    "<b>BlackRouter Version</b>\n\nName: {}\nVersion: {}\nRuntime: {}",
                    escape_html(build_info.name),
                    escape_html(build_info.version),
                    build_info.rust_runtime
                )
            }
            TelegramCommand::ApiKeys => self.api_keys_message(),
            TelegramCommand::ApiKeyCreate { name } => self.create_api_key_message(&name),
            TelegramCommand::Providers => self.providers_message(),
            TelegramCommand::Provider { provider_id } => self.provider_message(&provider_id),
            TelegramCommand::ProviderAdd {
                provider,
                auth_type,
                api_key,
                base_url,
                format,
            } => self.create_provider_message(&provider, &auth_type, api_key, &base_url, &format),
            TelegramCommand::ProviderModelsRefresh { connection_id } => {
                self.refresh_provider_models_message(&connection_id).await
            }
            TelegramCommand::Models { provider_id } => self.models_message(&provider_id),
            TelegramCommand::Combos => self.combos_message(),
            TelegramCommand::Combo { combo_name } => self.combo_message(&combo_name),
            TelegramCommand::ComboAdd { name, models } => self.create_combo_message(&name, models),
            TelegramCommand::Usage { range } => self.usage_message(&range),
            TelegramCommand::Logs { limit } => format!(
                "<b>Recent Logs</b>\n\nLimit: {}\n\nLog retrieval is not implemented yet.",
                limit
            ),
            TelegramCommand::EnableProvider { provider_id } => {
                self.toggle_connection_message(&provider_id, true)
            }
            TelegramCommand::DisableProvider { provider_id } => {
                self.toggle_connection_message(&provider_id, false)
            }
            TelegramCommand::EnableConnection { connection_id } => {
                self.toggle_connection_message(&connection_id, true)
            }
            TelegramCommand::DisableConnection { connection_id } => {
                self.toggle_connection_message(&connection_id, false)
            }
            TelegramCommand::TestProvider { provider_id } => self.test_connection_message(&provider_id),
            TelegramCommand::TestConnection { connection_id } => {
                self.test_connection_message(&connection_id)
            }
            TelegramCommand::Rtk { enabled } => format!(
                "<b>RTK Rate Limiting</b>\n\nRequested status: {}\n\nRTK toggle is not implemented in telegram runtime yet.",
                if enabled { "Enabled" } else { "Disabled" }
            ),
            TelegramCommand::Reload => {
                "<b>Reload Configuration</b>\n\nConfig reload is not implemented yet.".to_string()
            }
            TelegramCommand::Shutdown => {
                "<b>Shutdown</b>\n\nShutdown is not implemented from Telegram runtime.".to_string()
            }
            TelegramCommand::Link { code } => format!(
                "<b>Link Account</b>\n\nChat: {}\nCode: {}\n\nAccount linking is not implemented yet.",
                chat_id,
                escape_html(&code)
            ),
        }
    }

    fn status_message(&self) -> String {
        match self.storage.status() {
            Ok(status) => format!(
                "<b>BlackRouter Status</b>\n\nStatus: Running\nDatabase: {}\nTables: {}\nSchema Compatible: {}\nTelegram Uptime: {}",
                escape_html(&status.database_path.display().to_string()),
                status.table_counts.len(),
                if status.schema_compatible { "Yes" } else { "No" },
                format_duration(self.started_at.elapsed())
            ),
            Err(error) => format!("Error getting status: {}", escape_html(&error.to_string())),
        }
    }

    fn health_message(&self) -> String {
        match self.storage.status() {
            Ok(status) if status.schema_compatible => {
                "<b>Health Check</b>\n\nStatus: Healthy".to_string()
            }
            Ok(_) => {
                "<b>Health Check</b>\n\nStatus: Degraded\nReason: Schema incompatible".to_string()
            }
            Err(error) => format!(
                "<b>Health Check Failed</b>\n\nError: {}",
                escape_html(&error.to_string())
            ),
        }
    }

    fn api_keys_message(&self) -> String {
        match self.storage.list_api_keys() {
            Ok(keys) if keys.is_empty() => "No API keys configured".to_string(),
            Ok(keys) => {
                let mut message = "<b>API Keys</b>\n\n".to_string();
                for key in keys {
                    message.push_str(&format!(
                        "- {} ({}) [{}] created {}\n",
                        escape_html(key.name.as_deref().unwrap_or("unnamed")),
                        escape_html(&key.key_masked),
                        if key.is_active { "active" } else { "inactive" },
                        escape_html(&key.created_at)
                    ));
                }
                message
            }
            Err(error) => format!(
                "Error listing API keys: {}",
                escape_html(&error.to_string())
            ),
        }
    }

    fn create_api_key_message(&self, name: &str) -> String {
        match self.storage.create_api_key(NewApiKey {
            name: Some(name.to_string()),
            machine_id: Some("telegram".to_string()),
            tenant_id: None,
            policy: Default::default(),
        }) {
            Ok(created) => format!(
                "<b>API Key Created</b>\n\nName: {}\nID: {}\nKey: <code>{}</code>\n\nThis is shown once.",
                escape_html(name),
                escape_html(&created.record.id),
                escape_html(&created.key)
            ),
            Err(error) => format!("Failed to create API key: {}", escape_html(&error.to_string())),
        }
    }

    fn create_provider_message(
        &self,
        provider: &str,
        auth_type: &str,
        api_key: Option<String>,
        base_url: &str,
        format: &str,
    ) -> String {
        let mut data = serde_json::Map::new();
        data.insert("baseUrl".to_string(), Value::String(base_url.to_string()));
        data.insert("format".to_string(), Value::String(format.to_string()));
        if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
            data.insert("apiKey".to_string(), Value::String(api_key));
        }
        if let Some(models_url) = derive_models_url(base_url) {
            data.insert("modelsUrl".to_string(), Value::String(models_url));
        }

        match self.storage.create_provider_connection(NewProviderConnection {
            provider: provider.to_string(),
            auth_type: auth_type.to_string(),
            name: Some(provider.to_string()),
            email: None,
            priority: None,
            is_active: Some(true),
            status: Some("unknown".to_string()),
            cooldown_until: None,
            expires_at: None,
            data: Some(Value::Object(data)),
        }) {
            Ok(record) => format!(
                "<b>Provider Created</b>\n\nID: {}\nProvider: {}\nAuth: {}\nFormat: {}\nBase URL: {}",
                escape_html(&record.id),
                escape_html(&record.provider),
                escape_html(&record.auth_type),
                escape_html(format),
                escape_html(base_url)
            ),
            Err(error) => format!("Failed to create provider: {}", escape_html(&error.to_string())),
        }
    }

    async fn refresh_provider_models_message(&self, connection_id: &str) -> String {
        let provider = match self.storage.get_provider_connection_raw(connection_id) {
            Ok(provider) => provider,
            Err(error) => {
                return format!("Provider not found: {}", escape_html(&error.to_string()));
            }
        };

        let models_url = match provider_models_url(&provider.data) {
            Some(url) => url,
            None => {
                return "Could not derive models URL. Set data.modelsUrl or data.baseUrl."
                    .to_string();
            }
        };

        match fetch_provider_model_ids(&provider, &models_url).await {
            Ok(models) if models.is_empty() => {
                "Model refresh completed but no model IDs were found".to_string()
            }
            Ok(models) => {
                let count = models.len();
                match self.storage.set_provider_connection_models(
                    connection_id,
                    models,
                    Some(models_url.clone()),
                ) {
                    Ok(_) => format!(
                        "<b>Provider Models Refreshed</b>\n\nConnection: {}\nModels: {}\nSource: {}",
                        escape_html(connection_id),
                        count,
                        escape_html(&models_url)
                    ),
                    Err(error) => {
                        format!("Failed to save models: {}", escape_html(&error.to_string()))
                    }
                }
            }
            Err(error) => format!("Failed to fetch models: {}", escape_html(&error)),
        }
    }

    fn providers_message(&self) -> String {
        match self.storage.list_provider_connections() {
            Ok(providers) if providers.is_empty() => "No providers configured".to_string(),
            Ok(providers) => {
                let mut message = "<b>Providers</b>\n\n".to_string();
                for provider in providers {
                    let active = if provider.is_active {
                        "active"
                    } else {
                        "inactive"
                    };
                    message.push_str(&format!(
                        "- {} ({}) [{} / {}] priority {}\n",
                        escape_html(provider.name.as_deref().unwrap_or(&provider.provider)),
                        escape_html(&provider.provider),
                        active,
                        escape_html(&provider.status),
                        provider.priority.unwrap_or(0)
                    ));
                }
                message
            }
            Err(error) => format!(
                "Error listing providers: {}",
                escape_html(&error.to_string())
            ),
        }
    }

    fn provider_message(&self, provider_id: &str) -> String {
        match self.storage.get_provider_connection_raw(provider_id) {
            Ok(provider) => format!(
                "<b>Provider: {}</b>\n\nID: {}\nProvider: {}\nStatus: {} / {}\nAuth Type: {}\nPriority: {}\nCooldown Until: {}\nExpires At: {}\nCreated: {}",
                escape_html(provider.name.as_deref().unwrap_or(&provider.provider)),
                escape_html(&provider.id),
                escape_html(&provider.provider),
                if provider.is_active { "active" } else { "inactive" },
                escape_html(&provider.status),
                escape_html(&provider.auth_type),
                provider.priority.unwrap_or(0),
                escape_html(provider.cooldown_until.as_deref().unwrap_or("-")),
                escape_html(provider.expires_at.as_deref().unwrap_or("-")),
                escape_html(&provider.created_at)
            ),
            Err(error) => format!("Provider not found: {}", escape_html(&error.to_string())),
        }
    }

    fn models_message(&self, provider_id: &str) -> String {
        match self.storage.get_provider_connection_raw(provider_id) {
            Ok(provider) => {
                let models = provider
                    .data
                    .get("models")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .take(80)
                            .map(escape_html)
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_else(|| "No models fetched".to_string());

                format!(
                    "<b>Models for {}</b>\n\n{}",
                    escape_html(provider.name.as_deref().unwrap_or(&provider.provider)),
                    models
                )
            }
            Err(error) => format!("Provider not found: {}", escape_html(&error.to_string())),
        }
    }

    fn combos_message(&self) -> String {
        match self.storage.list_combos() {
            Ok(combos) if combos.is_empty() => "No combos configured".to_string(),
            Ok(combos) => {
                let mut message = "<b>Combos</b>\n\n".to_string();
                for combo in combos {
                    message.push_str(&format!(
                        "- {} ({}) - {} models\n",
                        escape_html(&combo.name),
                        escape_html(&combo.kind),
                        combo.models.len()
                    ));
                }
                message
            }
            Err(error) => format!("Error listing combos: {}", escape_html(&error.to_string())),
        }
    }

    fn create_combo_message(&self, name: &str, models: Vec<String>) -> String {
        match self.storage.create_combo(NewCombo {
            name: name.to_string(),
            kind: Some("llm".to_string()),
            models,
        }) {
            Ok(combo) => format!(
                "<b>Combo Created</b>\n\nName: {}\nModels: {}",
                escape_html(&combo.name),
                combo.models.len()
            ),
            Err(error) => format!(
                "Failed to create combo: {}",
                escape_html(&error.to_string())
            ),
        }
    }

    fn combo_message(&self, combo_name: &str) -> String {
        match self.storage.resolve_model_route(combo_name) {
            Ok(blackrouter_core::RouteKind::Single(model)) => format!(
                "<b>Route: {}</b>\n\nType: Single\nProvider: {}\nModel: {}",
                escape_html(combo_name),
                escape_html(&model.provider),
                escape_html(&model.model)
            ),
            Ok(blackrouter_core::RouteKind::Combo { name, models }) => {
                let models = models
                    .iter()
                    .map(|model| {
                        format!(
                            "- {}/{}",
                            escape_html(&model.provider),
                            escape_html(&model.model)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "<b>Combo: {}</b>\n\nType: Fallback\nModels:\n{}",
                    escape_html(&name),
                    models
                )
            }
            Err(error) => format!("Combo not found: {}", escape_html(&error.to_string())),
        }
    }

    fn usage_message(&self, range: &UsageRange) -> String {
        let since = usage_since(range);
        match self.storage.usage_stats(Some(&since)) {
            Ok(rows) if rows.is_empty() => format!(
                "<b>Usage Statistics</b>\n\nPeriod: {}\n\nNo usage data recorded",
                range_label(range)
            ),
            Ok(rows) => {
                let mut message = format!(
                    "<b>Usage Statistics</b>\n\nPeriod: {}\n",
                    range_label(range)
                );
                for row in rows {
                    message.push_str(&format!(
                        "\n- {}/{}: {} reqs, {} prompt + {} completion tokens, ${:.4}",
                        escape_html(&row.provider),
                        escape_html(&row.model),
                        row.count,
                        row.prompt_tokens,
                        row.completion_tokens,
                        row.cost
                    ));
                }
                message
            }
            Err(error) => format!("Error fetching usage: {}", escape_html(&error.to_string())),
        }
    }

    fn toggle_connection_message(&self, id: &str, enabled: bool) -> String {
        match self.storage.set_provider_connection_active(id, enabled) {
            Ok(_) => format!(
                "{} {}",
                if enabled { "Enabled" } else { "Disabled" },
                escape_html(id)
            ),
            Err(error) => format!(
                "Failed to update provider connection: {}",
                escape_html(&error.to_string())
            ),
        }
    }

    fn test_connection_message(&self, id: &str) -> String {
        match self.storage.get_provider_connection_raw(id) {
            Ok(provider) => {
                let base_url = provider
                    .data
                    .get("baseUrl")
                    .or_else(|| provider.data.get("base_url"))
                    .and_then(Value::as_str)
                    .unwrap_or("-");
                format!(
                    "<b>Connection Check</b>\n\nID: {}\nProvider: {}\nActive: {}\nStatus: {}\nBase URL: {}",
                    escape_html(&provider.id),
                    escape_html(&provider.provider),
                    provider.is_active,
                    escape_html(&provider.status),
                    escape_html(base_url)
                )
            }
            Err(error) => format!("Connection not found: {}", escape_html(&error.to_string())),
        }
    }
}

impl TelegramBot {
    /// Create a new Telegram bot instance
    pub fn new(config: TelegramBotConfig) -> Result<Self, TelegramBotError> {
        if config.bot_token.is_empty() {
            return Err(TelegramBotError::Config(
                "Bot token is required".to_string(),
            ));
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| telegram_http_error("build client", e))?;

        Ok(Self {
            client,
            bot_token: config.bot_token,
            api_base: "https://api.telegram.org".to_string(),
            admin_ids: config.admin_ids,
            _webhook_url: config.webhook_url,
            running: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    /// Get bot info
    pub async fn get_me(&self) -> Result<User, TelegramBotError> {
        let url = format!("{}/bot{}/getMe", self.api_base, self.bot_token);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| telegram_http_error("getMe request", e))?;

        let body: Value = response
            .json()
            .await
            .map_err(|e| telegram_http_error("getMe response decode", e))?;

        if !body.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            let description = body
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("Unknown error");
            return Err(TelegramBotError::Api(description.to_string()));
        }

        serde_json::from_value(body.get("result").cloned().unwrap_or(Value::Null))
            .map_err(|e| TelegramBotError::Api(e.to_string()))
    }

    /// Send a message to a chat
    pub async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        parse_mode: Option<&str>,
    ) -> Result<Message, TelegramBotError> {
        let url = format!("{}/bot{}/sendMessage", self.api_base, self.bot_token);

        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
        });

        if let Some(mode) = parse_mode {
            body.as_object_mut()
                .unwrap()
                .insert("parse_mode".to_string(), Value::String(mode.to_string()));
        }

        let response = self
            .client
            .post(url.clone())
            .json(&body)
            .send()
            .await
            .map_err(|e| telegram_http_error("sendMessage request", e))?;

        let resp: Value = response
            .json()
            .await
            .map_err(|e| telegram_http_error("sendMessage response decode", e))?;

        if !resp.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            let description = resp
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("Unknown error");
            return Err(TelegramBotError::Api(description.to_string()));
        }

        serde_json::from_value(resp.get("result").cloned().unwrap_or(Value::Null))
            .map_err(|e| TelegramBotError::Api(e.to_string()))
    }

    /// Get updates using long polling
    pub async fn get_updates(
        &self,
        offset: Option<i64>,
        timeout: u32,
    ) -> Result<Vec<Update>, TelegramBotError> {
        let url = format!("{}/bot{}/getUpdates", self.api_base, self.bot_token);

        let mut body = serde_json::json!({
            "timeout": timeout,
        });

        if let Some(offset) = offset {
            body.as_object_mut()
                .unwrap()
                .insert("offset".to_string(), Value::Number(offset.into()));
        }

        let response = self
            .client
            .post(url.clone())
            .json(&body)
            .send()
            .await
            .map_err(|e| telegram_http_error("getUpdates request", e))?;

        let resp: UpdateResponse = response
            .json()
            .await
            .map_err(|e| telegram_http_error("getUpdates response decode", e))?;

        if !resp.ok {
            return Err(TelegramBotError::Api("Failed to get updates".to_string()));
        }

        Ok(resp.result.unwrap_or_default())
    }

    /// Set webhook for receiving updates
    pub async fn set_webhook(&self, url: &str) -> Result<bool, TelegramBotError> {
        let api_url = format!("{}/bot{}/setWebhook", self.api_base, self.bot_token);

        let body = serde_json::json!({
            "url": url,
            "max_connections": 40,
            "allowed_updates": ["message", "callback_query"],
        });

        let response = self
            .client
            .post(api_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| telegram_http_error("setWebhook request", e))?;

        let resp: Value = response
            .json()
            .await
            .map_err(|e| telegram_http_error("setWebhook response decode", e))?;

        Ok(resp.get("ok").and_then(Value::as_bool).unwrap_or(false))
    }

    /// Remove webhook
    pub async fn delete_webhook(&self) -> Result<bool, TelegramBotError> {
        let url = format!("{}/bot{}/deleteWebhook", self.api_base, self.bot_token);

        let response = self
            .client
            .post(url.clone())
            .send()
            .await
            .map_err(|e| telegram_http_error("deleteWebhook request", e))?;

        let resp: Value = response
            .json()
            .await
            .map_err(|e| telegram_http_error("deleteWebhook response decode", e))?;

        Ok(resp.get("ok").and_then(Value::as_bool).unwrap_or(false))
    }

    /// Start long polling loop
    pub async fn start_polling<F>(&self, handler: F) -> Result<(), TelegramBotError>
    where
        F: Fn(TelegramCommand, i64) -> futures::future::BoxFuture<'static, String>
            + Send
            + Sync
            + 'static,
    {
        self.running
            .store(true, std::sync::atomic::Ordering::SeqCst);

        info!("Starting Telegram bot polling");

        let mut offset: Option<i64> = None;
        let handler = std::sync::Arc::new(handler);

        while self.running.load(std::sync::atomic::Ordering::SeqCst) {
            match self.get_updates(offset, 30).await {
                Ok(updates) => {
                    for update in updates {
                        offset = Some(update.update_id + 1);

                        if let Some(message) = update.message {
                            if let Some(text) = &message.text {
                                let chat_id = message.chat.id;
                                let from_id = message.from.as_ref().map(|u| u.id).unwrap_or(0);

                                // Check authorization
                                if !self.is_authorized(from_id) {
                                    warn!("Unauthorized access attempt from chat {}", chat_id);
                                    let _ = self
                                        .send_message(
                                            chat_id,
                                            "⚠️ Unauthorized. You are not an admin.",
                                            None,
                                        )
                                        .await;
                                    continue;
                                }

                                // Parse and handle command
                                match TelegramCommand::parse(text) {
                                    Ok(command) => {
                                        debug!("Received command: {:?}", command);

                                        // Check if command requires confirmation
                                        if command.requires_confirmation() {
                                            // For now, auto-confirm. In production, implement confirmation flow
                                            info!("Command requires confirmation: {:?}", command);
                                        }

                                        let response = handler(command, chat_id).await;
                                        let _ = self
                                            .send_message(chat_id, &response, Some("HTML"))
                                            .await;
                                    }
                                    Err(TelegramCommandError::Empty) => {
                                        // Ignore empty messages
                                    }
                                    Err(TelegramCommandError::Unknown(cmd)) => {
                                        let _ = self
                                            .send_message(
                                                chat_id,
                                                &format!(
                                                    "❓ Unknown command: /{}\n\nType /help for available commands.",
                                                    cmd
                                                ),
                                                None,
                                            )
                                            .await;
                                    }
                                    Err(e) => {
                                        let _ = self
                                            .send_message(
                                                chat_id,
                                                &format!("❌ Error: {}", e),
                                                None,
                                            )
                                            .await;
                                    }
                                }
                            }
                        }

                        // Handle callback queries (inline keyboard buttons)
                        if let Some(callback) = update.callback_query {
                            if let Some(data) = &callback.data {
                                let chat_id =
                                    callback.message.as_ref().map(|m| m.chat.id).unwrap_or(0);

                                info!("Callback query: {} from chat {}", data, chat_id);

                                // Handle callback data (e.g., confirmation buttons)
                                // This is a placeholder - implement based on your needs
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to get updates: {}", e);
                    // Wait before retrying
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }

        info!("Telegram bot polling stopped");
        Ok(())
    }

    /// Stop the bot
    pub fn stop(&self) {
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Check if a user is authorized
    fn is_authorized(&self, user_id: i64) -> bool {
        self.admin_ids.contains(&user_id)
    }

    /// Format help message
    pub fn help_message() -> String {
        r#"🤖 <b>BlackRouter Telegram Bot</b>

<b>Read-only Commands:</b>
/status - Show system status
/health - Health check
/version - Show version
/apikeys - List API keys
/providers - List all providers
/provider <id> - Show provider details
/models <provider_id> - List provider models
/combos - List all combos
/combo <name> - Show combo details
/usage [today|7d] - Show usage statistics
/logs [limit] - Show recent logs

<b>Control Commands:</b>
/apikey create <name> - Create API key
/provider add <provider> <auth_type> key=<secret> baseUrl=<url> format=<format> - Add provider
/provider models-refresh <connection_id> - Fetch and save provider models
/combo add <name> <provider/model> ... - Create combo
/enable provider <id> - Enable provider
/disable provider <id> - Disable provider
/enable connection <id> - Enable connection
/disable connection <id> - Disable connection
/test provider <id> - Test provider connection
/test connection <id> - Test connection
/rtk on|off - Enable/disable rate limiting
/reload - Reload configuration
/shutdown - Shutdown BlackRouter
/link <code> - Link Telegram account

<b>Info:</b>
/help - Show this help message"#
            .to_string()
    }

    /// Format status message
    pub fn format_status(
        uptime: Duration,
        total_requests: u64,
        success_rate: f64,
        active_providers: usize,
    ) -> String {
        format!(
            r#"📊 <b>BlackRouter Status</b>

⏱ Uptime: {}
📈 Total Requests: {}
✅ Success Rate: {:.1}%
🔌 Active Providers: {}"#,
            format_duration(uptime),
            total_requests,
            success_rate * 100.0,
            active_providers,
        )
    }
}

async fn fetch_provider_model_ids(
    provider: &RawProviderConnection,
    models_url: &str,
) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| telegram_http_error("build model client", error).to_string())?;
    let request = apply_provider_auth(client.get(models_url), &provider.auth_type, &provider.data);
    let response = request
        .send()
        .await
        .map_err(|error| telegram_http_error("fetch provider models", error).to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "provider models endpoint returned HTTP {}",
            status.as_u16()
        ));
    }

    let value = response
        .json::<Value>()
        .await
        .map_err(|error| telegram_http_error("decode provider models", error).to_string())?;

    let mut models = extract_model_ids(&value);
    models.sort();
    models.dedup();
    Ok(models)
}

fn provider_models_url(data: &Value) -> Option<String> {
    data.get("modelsUrl")
        .or_else(|| data.get("models_url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            data.get("baseUrl")
                .or_else(|| data.get("base_url"))
                .and_then(Value::as_str)
                .and_then(derive_models_url)
        })
}

fn derive_models_url(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.ends_with("/models") {
        return Some(trimmed.to_string());
    }
    if let Some(prefix) = trimmed.strip_suffix("/chat/completions") {
        return Some(format!("{prefix}/models"));
    }
    if let Some(prefix) = trimmed.strip_suffix("/messages") {
        return Some(format!("{prefix}/models"));
    }
    if trimmed.contains("generativelanguage.googleapis.com") {
        return Some(trimmed.to_string());
    }
    Some(format!("{trimmed}/models"))
}

fn extract_model_ids(value: &Value) -> Vec<String> {
    let mut models = Vec::new();
    if let Some(items) = value.get("data").and_then(Value::as_array) {
        collect_model_ids(items, &mut models);
    }
    if let Some(items) = value.get("models").and_then(Value::as_array) {
        collect_model_ids(items, &mut models);
    }
    if let Some(items) = value.as_array() {
        collect_model_ids(items, &mut models);
    }
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

fn apply_provider_auth(
    mut request: reqwest::RequestBuilder,
    auth_type: &str,
    data: &Value,
) -> reqwest::RequestBuilder {
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
                data.get("username").and_then(Value::as_str),
                data.get("password").and_then(Value::as_str),
            ) {
                request = request.basic_auth(username, Some(password));
            }
        }
        "header" => {
            if let (Some(header_name), Some(header_value)) = (
                data.get("headerName").and_then(Value::as_str),
                data.get("headerValue")
                    .and_then(Value::as_str)
                    .or_else(|| data.get("apiKey").and_then(Value::as_str)),
            ) {
                request = request.header(header_name, header_value);
            }
        }
        _ => {
            if let Some(token) = provider_token(data) {
                request = request.bearer_auth(token);
            }
        }
    }

    request
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

/// Format duration to human readable string
fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d {}h", secs / 86400, (secs % 86400) / 3600)
    }
}

fn range_label(range: &UsageRange) -> &'static str {
    match range {
        UsageRange::Today => "Today",
        UsageRange::SevenDays => "Last 7 Days",
    }
}

fn usage_since(range: &UsageRange) -> String {
    let now = blackrouter_common::unix_timestamp();
    let start_of_period = match range {
        UsageRange::Today => now - (now % 86_400),
        UsageRange::SevenDays => now.saturating_sub(7 * 86_400),
    };

    let days = start_of_period / 86_400;
    let remaining_secs = start_of_period % 86_400;
    let hours = remaining_secs / 3_600;
    let minutes = (remaining_secs % 3_600) / 60;
    let seconds = remaining_secs % 60;

    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::{Chat, Message, TelegramCommand, TelegramRuntime, Update, UsageRange, User};
    use blackrouter_storage::{NewProviderConnection, Storage};
    use serde_json::json;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn temp_storage(label: &str) -> (Storage, std::path::PathBuf) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!(
            "blackrouter-telegram-{label}-{}-{nanos}.sqlite",
            std::process::id()
        ));
        let storage = Storage::new(&path);
        storage.initialize().expect("schema initializes");
        (storage, path)
    }

    fn text_update(chat_id: i64, user_id: i64, text: &str) -> Update {
        Update {
            update_id: 1,
            message: Some(Message {
                message_id: 1,
                from: Some(User {
                    id: user_id,
                    is_bot: false,
                    first_name: "Admin".to_string(),
                    last_name: None,
                    username: Some("admin".to_string()),
                }),
                chat: Chat {
                    id: chat_id,
                    chat_type: "private".to_string(),
                    title: None,
                },
                date: 1,
                text: Some(text.to_string()),
            }),
            callback_query: None,
        }
    }

    fn provider(provider: &str) -> NewProviderConnection {
        NewProviderConnection {
            provider: provider.to_string(),
            auth_type: "none".to_string(),
            name: Some(provider.to_string()),
            email: None,
            priority: None,
            is_active: Some(true),
            status: Some("healthy".to_string()),
            cooldown_until: None,
            expires_at: None,
            data: Some(json!({
                "baseUrl": "http://127.0.0.1:20130/health",
                "format": "openai",
                "models": ["model-a"]
            })),
        }
    }

    #[test]
    fn parses_read_only_commands() {
        assert_eq!(
            TelegramCommand::parse("/status").unwrap(),
            TelegramCommand::Status
        );
        assert_eq!(
            TelegramCommand::parse("/usage 7d").unwrap(),
            TelegramCommand::Usage {
                range: UsageRange::SevenDays
            }
        );
    }

    #[test]
    fn parses_control_commands() {
        let command = TelegramCommand::parse("/disable provider openai").unwrap();
        assert_eq!(
            command,
            TelegramCommand::DisableProvider {
                provider_id: "openai".to_string()
            }
        );
        assert!(command.requires_confirmation());
        assert!(command.mutates_state());
    }

    #[test]
    fn parses_runtime_management_commands() {
        assert_eq!(
            TelegramCommand::parse("/apikeys").unwrap(),
            TelegramCommand::ApiKeys
        );
        assert_eq!(
            TelegramCommand::parse("/apikey create mobile").unwrap(),
            TelegramCommand::ApiKeyCreate {
                name: "mobile".to_string()
            }
        );
        assert_eq!(
            TelegramCommand::parse(
                "/provider add openai api-key key=sk-test baseUrl=https://api.openai.com/v1/chat/completions format=openai"
            )
            .unwrap(),
            TelegramCommand::ProviderAdd {
                provider: "openai".to_string(),
                auth_type: "api-key".to_string(),
                api_key: Some("sk-test".to_string()),
                base_url: "https://api.openai.com/v1/chat/completions".to_string(),
                format: "openai".to_string(),
            }
        );
        assert_eq!(
            TelegramCommand::parse("/provider models-refresh conn-1").unwrap(),
            TelegramCommand::ProviderModelsRefresh {
                connection_id: "conn-1".to_string()
            }
        );
        assert_eq!(
            TelegramCommand::parse("/combo add black-gemini antigravity/gemini-2.5-pro").unwrap(),
            TelegramCommand::ComboAdd {
                name: "black-gemini".to_string(),
                models: vec!["antigravity/gemini-2.5-pro".to_string()]
            }
        );
    }

    #[test]
    fn test_help_message() {
        let help = super::TelegramBot::help_message();
        assert!(help.contains("/status"));
        assert!(help.contains("/providers"));
        assert!(help.contains("/help"));
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(super::format_duration(Duration::from_secs(30)), "30s");
        assert_eq!(super::format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(super::format_duration(Duration::from_secs(3661)), "1h 1m");
        assert_eq!(super::format_duration(Duration::from_secs(90000)), "1d 1h");
    }

    #[tokio::test]
    async fn runtime_handles_authorized_status_update() {
        let (storage, path) = temp_storage("status");
        let runtime = TelegramRuntime::new(storage, vec![100]);
        let message = runtime
            .handle_update(text_update(100, 100, "/status"))
            .await
            .expect("update handles")
            .expect("response message");

        assert_eq!(message.chat_id, 100);
        assert!(message.text.contains("BlackRouter Status"));
        assert_eq!(message.parse_mode, Some("HTML"));

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn runtime_rejects_unauthorized_chat() {
        let (storage, path) = temp_storage("unauthorized");
        let runtime = TelegramRuntime::new(storage, vec![100]);
        let message = runtime
            .handle_update(text_update(200, 200, "/status"))
            .await
            .expect("update handles")
            .expect("response message");

        assert_eq!(message.chat_id, 200);
        assert!(message.text.contains("Unauthorized"));
        assert_eq!(message.parse_mode, None);

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn runtime_can_toggle_provider_connection() {
        let (storage, path) = temp_storage("toggle");
        let record = storage
            .create_provider_connection(provider("cline"))
            .expect("provider creates");
        let runtime = TelegramRuntime::new(storage.clone(), vec![100]);

        let response = runtime
            .handle_command(
                TelegramCommand::DisableConnection {
                    connection_id: record.id.clone(),
                },
                100,
            )
            .await;

        assert!(response.contains("Disabled"));
        assert!(
            !storage
                .get_provider_connection_raw(&record.id)
                .expect("provider exists")
                .is_active
        );

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn runtime_can_create_api_key_provider_and_combo() {
        let (storage, path) = temp_storage("create-resources");
        let runtime = TelegramRuntime::new(storage.clone(), vec![100]);

        let api_key = runtime
            .handle_command(
                TelegramCommand::ApiKeyCreate {
                    name: "mobile".to_string(),
                },
                100,
            )
            .await;
        assert!(api_key.contains("API Key Created"));
        assert_eq!(storage.list_api_keys().unwrap().len(), 1);

        let provider_response = runtime
            .handle_command(
                TelegramCommand::ProviderAdd {
                    provider: "cline".to_string(),
                    auth_type: "api-key".to_string(),
                    api_key: Some("secret".to_string()),
                    base_url: "https://api.cline.bot/api/v1/chat/completions".to_string(),
                    format: "openai".to_string(),
                },
                100,
            )
            .await;
        assert!(provider_response.contains("Provider Created"));

        let combo = runtime
            .handle_command(
                TelegramCommand::ComboAdd {
                    name: "code".to_string(),
                    models: vec!["cline/model-a".to_string()],
                },
                100,
            )
            .await;
        assert!(combo.contains("Combo Created"));
        assert!(storage.resolve_model_route("code").is_ok());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn derives_and_extracts_model_catalogs() {
        assert_eq!(
            super::derive_models_url("https://api.openai.com/v1/chat/completions").as_deref(),
            Some("https://api.openai.com/v1/models")
        );
        assert_eq!(
            super::extract_model_ids(&json!({
                "data": [{"id": "gpt-4.1"}, {"name": "models/gemini-2.5-pro"}]
            })),
            vec!["gpt-4.1".to_string(), "gemini-2.5-pro".to_string()]
        );
    }
}
