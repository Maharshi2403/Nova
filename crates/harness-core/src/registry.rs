//! Provider-independent model registry and capability validation.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyClass {
    Local,
    Private,
    Public,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDefinition {
    pub id: String,
    pub provider: String,
    pub model_id: String,
    pub supported_tools: Vec<String>,
    pub structured_output: bool,
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub input_usd_per_million_tokens: f64,
    pub output_usd_per_million_tokens: f64,
    pub available: bool,
    pub privacy: PrivacyClass,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRegistry {
    pub models: Vec<ModelDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    Parse(String),
    Invalid(String),
    UnknownModel(String),
    UnsupportedCapability { model: String, capability: String },
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(error) => write!(f, "invalid model registry: {error}"),
            Self::Invalid(error) => write!(f, "invalid model registry: {error}"),
            Self::UnknownModel(model) => write!(f, "unknown model {model:?}"),
            Self::UnsupportedCapability { model, capability } => {
                write!(f, "model {model:?} does not support {capability:?}")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

impl ModelRegistry {
    pub fn from_json(text: &str) -> Result<Self, RegistryError> {
        let registry: Self =
            serde_json::from_str(text).map_err(|error| RegistryError::Parse(error.to_string()))?;
        registry.validate()?;
        Ok(registry)
    }

    pub fn validate(&self) -> Result<(), RegistryError> {
        if self.models.is_empty() {
            return Err(RegistryError::Invalid(
                "at least one model is required".into(),
            ));
        }
        let mut ids = HashSet::new();
        for model in &self.models {
            for (field, value) in [
                ("id", &model.id),
                ("provider", &model.provider),
                ("model_id", &model.model_id),
            ] {
                if value.trim().is_empty() || value.chars().count() > 200 {
                    return Err(RegistryError::Invalid(format!(
                        "{field} must be 1..=200 characters"
                    )));
                }
            }
            if !ids.insert(&model.id) {
                return Err(RegistryError::Invalid(format!(
                    "duplicate model id {:?}",
                    model.id
                )));
            }
            if model.context_window == 0 || model.max_output_tokens == 0 {
                return Err(RegistryError::Invalid(
                    "token limits must be positive".into(),
                ));
            }
            if model.max_output_tokens > model.context_window {
                return Err(RegistryError::Invalid(
                    "max_output_tokens cannot exceed context_window".into(),
                ));
            }
            if !model.input_usd_per_million_tokens.is_finite()
                || !model.output_usd_per_million_tokens.is_finite()
                || model.input_usd_per_million_tokens < 0.0
                || model.output_usd_per_million_tokens < 0.0
            {
                return Err(RegistryError::Invalid(
                    "prices must be finite and non-negative".into(),
                ));
            }
            if model
                .supported_tools
                .iter()
                .any(|tool| tool.trim().is_empty())
            {
                return Err(RegistryError::Invalid(
                    "supported tools must not be empty".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn model(&self, id: &str) -> Result<&ModelDefinition, RegistryError> {
        self.models
            .iter()
            .find(|model| model.id == id)
            .ok_or_else(|| RegistryError::UnknownModel(id.to_owned()))
    }

    pub fn require(
        &self,
        id: &str,
        tool: Option<&str>,
        structured_output: bool,
    ) -> Result<&ModelDefinition, RegistryError> {
        let model = self.model(id)?;
        if !model.available {
            return Err(RegistryError::UnsupportedCapability {
                model: id.to_owned(),
                capability: "availability".into(),
            });
        }
        if structured_output && !model.structured_output {
            return Err(RegistryError::UnsupportedCapability {
                model: id.to_owned(),
                capability: "structured_output".into(),
            });
        }
        if let Some(tool) = tool
            && !model
                .supported_tools
                .iter()
                .any(|supported| supported == tool)
        {
            return Err(RegistryError::UnsupportedCapability {
                model: id.to_owned(),
                capability: format!("tool:{tool}"),
            });
        }
        Ok(model)
    }
}
