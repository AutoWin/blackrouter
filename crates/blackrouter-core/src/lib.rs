use serde::Serialize;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum RouteKind {
    Single(ModelRef),
    Combo { name: String, models: Vec<ModelRef> },
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RoutingError {
    #[error("missing model")]
    MissingModel,
    #[error("invalid model format: {0}")]
    InvalidModel(String),
    #[error("provider unavailable: {0}")]
    ProviderUnavailable(String),
}

pub fn parse_provider_model(value: &str) -> Result<ModelRef, RoutingError> {
    let (provider, model) = value
        .split_once('/')
        .ok_or_else(|| RoutingError::InvalidModel(value.to_string()))?;

    if provider.trim().is_empty() || model.trim().is_empty() {
        return Err(RoutingError::InvalidModel(value.to_string()));
    }

    Ok(ModelRef {
        provider: provider.trim().to_string(),
        model: model.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_provider_model, ModelRef};

    #[test]
    fn parses_provider_model() {
        assert_eq!(
            parse_provider_model("openai/gpt-4.1").unwrap(),
            ModelRef {
                provider: "openai".to_string(),
                model: "gpt-4.1".to_string()
            }
        );
    }
}
