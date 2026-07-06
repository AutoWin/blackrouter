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
    Providers,
    Provider { provider_id: String },
    Models { provider_id: String },
    Combos,
    Combo { combo_name: String },
    Usage { range: UsageRange },
    Logs { limit: usize },
    EnableProvider { provider_id: String },
    DisableProvider { provider_id: String },
    EnableConnection { connection_id: String },
    DisableConnection { connection_id: String },
    TestProvider { provider_id: String },
    TestConnection { connection_id: String },
    Rtk { enabled: bool },
    Reload,
    Shutdown,
    Link { code: String },
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
            "providers" => Ok(Self::Providers),
            "provider" => Ok(Self::Provider {
                provider_id: required(&mut parts, "provider_id")?,
            }),
            "models" => Ok(Self::Models {
                provider_id: required(&mut parts, "provider_id")?,
            }),
            "combos" => Ok(Self::Combos),
            "combo" => Ok(Self::Combo {
                combo_name: required(&mut parts, "combo_name")?,
            }),
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
            .map_err(|e| TelegramBotError::Http(e.to_string()))?;

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
            .map_err(|e| TelegramBotError::Http(e.to_string()))?;

        let body: Value = response
            .json()
            .await
            .map_err(|e| TelegramBotError::Http(e.to_string()))?;

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
            .map_err(|e| TelegramBotError::Http(e.to_string()))?;

        let resp: Value = response
            .json()
            .await
            .map_err(|e| TelegramBotError::Http(e.to_string()))?;

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
            .map_err(|e| TelegramBotError::Http(e.to_string()))?;

        let resp: UpdateResponse = response
            .json()
            .await
            .map_err(|e| TelegramBotError::Http(e.to_string()))?;

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
            .map_err(|e| TelegramBotError::Http(e.to_string()))?;

        let resp: Value = response
            .json()
            .await
            .map_err(|e| TelegramBotError::Http(e.to_string()))?;

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
            .map_err(|e| TelegramBotError::Http(e.to_string()))?;

        let resp: Value = response
            .json()
            .await
            .map_err(|e| TelegramBotError::Http(e.to_string()))?;

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
/providers - List all providers
/provider <id> - Show provider details
/models <provider_id> - List provider models
/combos - List all combos
/combo <name> - Show combo details
/usage [today|7d] - Show usage statistics
/logs [limit] - Show recent logs

<b>Control Commands:</b>
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

#[cfg(test)]
mod tests {
    use super::{TelegramCommand, UsageRange};
    use std::time::Duration;

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
}
