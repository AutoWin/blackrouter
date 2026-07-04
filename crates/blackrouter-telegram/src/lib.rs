use serde::Serialize;
use thiserror::Error;

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

#[cfg(test)]
mod tests {
    use super::{TelegramCommand, UsageRange};

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
}
