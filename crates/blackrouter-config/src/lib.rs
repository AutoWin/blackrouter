use serde::Serialize;
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid port in {key}: {value}")]
    InvalidPort { key: &'static str, value: String },
    #[error("invalid host address: {0}")]
    InvalidHost(String),
}

pub type Result<T> = std::result::Result<T, ConfigError>;

#[derive(Clone, Debug, Serialize)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub database_url: String,
    pub database_path: PathBuf,
    pub compat_9router_db: bool,
    pub require_api_key: bool,
    pub control_api_enabled: bool,
    #[serde(skip_serializing)]
    pub control_token: Option<String>,
    #[serde(skip_serializing)]
    pub redis_url: Option<String>,
    pub shared_state_prefix: String,
    pub log_level: String,
    pub telegram: TelegramConfig,
}

#[derive(Clone, Debug, Serialize)]
pub struct TelegramConfig {
    pub enabled: bool,
    #[serde(skip_serializing)]
    pub bot_token: Option<String>,
    pub admin_ids: Vec<i64>,
    pub link_code_ttl_seconds: u64,
    pub use_webhook: bool,
    pub webhook_url: Option<String>,
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        dotenvy::dotenv().ok();

        let host = env_string("BLACKROUTER_HOST").unwrap_or_else(|| "0.0.0.0".to_string());
        let port = env_u16("BLACKROUTER_PORT")?
            .or(env_u16("PORT")?)
            .unwrap_or(20129);

        let compat_9router_db = env_bool("BLACKROUTER_COMPAT_9ROUTER_DB").unwrap_or(true);
        let require_api_key = env_bool("BLACKROUTER_REQUIRE_API_KEY").unwrap_or(false);
        let control_api_enabled = env_bool("BLACKROUTER_CONTROL_API_ENABLED").unwrap_or(false);

        let data_dir = env_string("BLACKROUTER_DATA_DIR")
            .or_else(|| env_string("DATA_DIR"))
            .map(|value| expand_tilde(&value))
            .unwrap_or_else(default_data_dir);

        let database_url = env_string("BLACKROUTER_DATABASE_URL").unwrap_or_else(|| {
            let database_path = if compat_9router_db {
                data_dir.join("db").join("data.sqlite")
            } else {
                data_dir.join("blackrouter.db")
            };
            format!("sqlite://{}", database_path.display())
        });

        let database_path = sqlite_url_to_path(&database_url);

        Ok(Self {
            host,
            port,
            data_dir,
            database_url,
            database_path,
            compat_9router_db,
            require_api_key,
            control_api_enabled,
            control_token: env_string("BLACKROUTER_CONTROL_TOKEN"),
            redis_url: env_string("BLACKROUTER_REDIS_URL"),
            shared_state_prefix: env_string("BLACKROUTER_SHARED_STATE_PREFIX")
                .unwrap_or_else(|| "blackrouter".to_string()),
            log_level: env_string("BLACKROUTER_LOG_LEVEL").unwrap_or_else(|| "info".to_string()),
            telegram: TelegramConfig::load(),
        })
    }

    pub fn bind_addr(&self) -> Result<SocketAddr> {
        let ip: IpAddr = self
            .host
            .parse()
            .map_err(|_| ConfigError::InvalidHost(self.host.clone()))?;
        Ok(SocketAddr::new(ip, self.port))
    }
}

impl TelegramConfig {
    fn load() -> Self {
        let bot_token = env_string("TELEGRAM_BOT_TOKEN").filter(|value| !value.is_empty());
        let admin_ids = env_string("TELEGRAM_ADMIN_IDS")
            .map(|value| {
                value
                    .split(',')
                    .filter_map(|part| part.trim().parse::<i64>().ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Self {
            enabled: env_bool("TELEGRAM_ENABLED").unwrap_or_else(|| bot_token.is_some()),
            bot_token,
            admin_ids,
            link_code_ttl_seconds: env_u64("TELEGRAM_LINK_CODE_TTL_SECONDS").unwrap_or(300),
            use_webhook: env_bool("TELEGRAM_USE_WEBHOOK").unwrap_or(false),
            webhook_url: env_string("TELEGRAM_WEBHOOK_URL").filter(|value| !value.is_empty()),
        }
    }
}

fn env_string(key: &'static str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_u16(key: &'static str) -> Result<Option<u16>> {
    match env_string(key) {
        Some(value) => value
            .parse::<u16>()
            .map(Some)
            .map_err(|_| ConfigError::InvalidPort { key, value }),
        None => Ok(None),
    }
}

fn env_u64(key: &'static str) -> Option<u64> {
    env_string(key).and_then(|value| value.parse::<u64>().ok())
}

fn env_bool(key: &'static str) -> Option<bool> {
    env_string(key).and_then(|value| match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    })
}

fn default_data_dir() -> PathBuf {
    env::var("HOME")
        .map(|home| PathBuf::from(home).join(".9router"))
        .unwrap_or_else(|_| PathBuf::from(".blackrouter"))
}

fn expand_tilde(value: &str) -> PathBuf {
    if value == "~" {
        return default_home_dir();
    }

    if let Some(rest) = value.strip_prefix("~/") {
        return default_home_dir().join(rest);
    }

    PathBuf::from(value)
}

fn default_home_dir() -> PathBuf {
    env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn sqlite_url_to_path(url: &str) -> PathBuf {
    let path = url.strip_prefix("sqlite://").unwrap_or(url);
    expand_tilde(path)
}

pub fn parent_dir(path: &Path) -> Option<PathBuf> {
    path.parent().map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::sqlite_url_to_path;
    use std::path::PathBuf;

    #[test]
    fn parses_absolute_sqlite_url() {
        assert_eq!(
            sqlite_url_to_path("sqlite:///tmp/blackrouter.db"),
            PathBuf::from("/tmp/blackrouter.db")
        );
    }
}
