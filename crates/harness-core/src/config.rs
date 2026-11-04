//! Layered configuration: CLI > env > file > defaults.
//!
//! Pure functions only: callers supply file text, environment pairs and CLI overrides, so
//! this crate performs no I/O. The safe defaults are `provider = mock`, `concurrency = 2`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub const MAX_CONCURRENCY_CEILING: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Parse(String),
    Invalid(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "config parse error: {e}"),
            Self::Invalid(e) => write!(f, "invalid config: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    TrustedLocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Mock,
    Openai,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WorkerConfig {
    pub default_concurrency: u32,
    pub maximum_concurrency: u32,
    pub heartbeat_interval_seconds: u32,
    pub heartbeat_timeout_seconds: u32,
    pub default_timeout_seconds: u32,
    pub max_tool_calls: u32,
    pub max_ipc_message_bytes: u32,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            default_concurrency: 2,
            maximum_concurrency: MAX_CONCURRENCY_CEILING,
            heartbeat_interval_seconds: 5,
            heartbeat_timeout_seconds: 20,
            default_timeout_seconds: 180,
            max_tool_calls: 30,
            max_ipc_message_bytes: 1_048_576,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RoutingConfig {
    pub default_low_model: String,
    pub default_medium_model: String,
    pub default_high_model: String,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        let m = || "mock-coder".to_string();
        Self {
            default_low_model: m(),
            default_medium_model: m(),
            default_high_model: m(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BudgetConfig {
    pub max_task_usd: f64,
    pub max_run_usd: f64,
    pub max_daily_usd: f64,
    pub max_output_tokens: u32,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_task_usd: 0.50,
            max_run_usd: 5.00,
            max_daily_usd: 10.00,
            max_output_tokens: 3000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ExecutionConfig {
    pub allow_network: bool,
    pub allow_auto_push: bool,
    pub require_approval_for_commands: bool,
    pub require_approved_plan: bool,
    pub allowed_test_commands: Vec<Vec<String>>,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            allow_network: false,
            allow_auto_push: false,
            require_approval_for_commands: true,
            require_approved_plan: true,
            allowed_test_commands: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SecurityConfig {
    pub loopback_only: bool,
    pub log_raw_prompts: bool,
    pub log_raw_secrets: bool,
    pub allow_untrusted_without_sandbox: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            loopback_only: true,
            log_raw_prompts: false,
            log_raw_secrets: false,
            allow_untrusted_without_sandbox: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    pub protocol_version: String,
    pub mode: Mode,
    pub provider: Provider,
    pub worker: WorkerConfig,
    pub routing: RoutingConfig,
    pub budget: BudgetConfig,
    pub execution: ExecutionConfig,
    pub security: SecurityConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            protocol_version: "1.0".to_string(),
            mode: Mode::TrustedLocal,
            provider: Provider::Mock,
            worker: WorkerConfig::default(),
            routing: RoutingConfig::default(),
            budget: BudgetConfig::default(),
            execution: ExecutionConfig::default(),
            security: SecurityConfig::default(),
        }
    }
}

/// Command-line overrides; `None` means "not given".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliOverrides {
    pub provider: Option<String>,
    pub concurrency: Option<u32>,
}

impl Config {
    /// Resolve with precedence CLI > env > file > defaults, then validate.
    /// `env` is a list of `(name, value)` pairs; only `HARNESS_*` names are read.
    pub fn resolve<'a>(
        file_yaml: Option<&str>,
        env: impl IntoIterator<Item = (&'a str, &'a str)>,
        cli: &CliOverrides,
    ) -> Result<Self, ConfigError> {
        let mut merged =
            serde_json::to_value(Self::default()).map_err(|e| ConfigError::Parse(e.to_string()))?;

        if let Some(text) = file_yaml {
            let file: Value =
                serde_yaml_ng::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;
            if !file.is_null() {
                if !file.is_object() {
                    return Err(ConfigError::Invalid("file root must be a mapping".into()));
                }
                merge(&mut merged, file);
            }
        }

        merge(&mut merged, env_layer(env)?);

        let mut cli_layer = Map::new();
        if let Some(p) = &cli.provider {
            cli_layer.insert("provider".into(), Value::String(p.clone()));
        }
        if let Some(c) = cli.concurrency {
            cli_layer.insert("worker".into(), json!({ "default_concurrency": c }));
        }
        merge(&mut merged, Value::Object(cli_layer));

        let config: Self =
            serde_json::from_value(merged).map_err(|e| ConfigError::Invalid(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |m: &str| Err(ConfigError::Invalid(m.to_string()));
        if self.protocol_version != "1.0" {
            return invalid("unsupported protocol_version");
        }
        let w = &self.worker;
        if w.maximum_concurrency == 0 || w.maximum_concurrency > MAX_CONCURRENCY_CEILING {
            return invalid("worker.maximum_concurrency must be between 1 and 4");
        }
        if w.default_concurrency == 0 || w.default_concurrency > w.maximum_concurrency {
            return invalid("worker.default_concurrency must be between 1 and maximum_concurrency");
        }
        if w.heartbeat_interval_seconds == 0
            || w.heartbeat_timeout_seconds <= w.heartbeat_interval_seconds
        {
            return invalid("heartbeat_timeout_seconds must exceed heartbeat_interval_seconds");
        }
        if w.default_timeout_seconds == 0 || w.max_tool_calls == 0 {
            return invalid("worker timeout and max_tool_calls must be positive");
        }
        if w.max_ipc_message_bytes == 0 || w.max_ipc_message_bytes > 1_048_576 {
            return invalid("worker.max_ipc_message_bytes must be between 1 and 1048576");
        }
        let b = &self.budget;
        let money = [b.max_task_usd, b.max_run_usd, b.max_daily_usd];
        if money.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return invalid("budget amounts must be finite and non-negative");
        }
        if b.max_task_usd > b.max_run_usd || b.max_run_usd > b.max_daily_usd {
            return invalid("budget must satisfy max_task_usd <= max_run_usd <= max_daily_usd");
        }
        if self.execution.allow_auto_push {
            return invalid("execution.allow_auto_push is not supported in v1");
        }
        if self.security.log_raw_secrets {
            return invalid("security.log_raw_secrets must be false");
        }
        Ok(())
    }
}

fn env_layer<'a>(env: impl IntoIterator<Item = (&'a str, &'a str)>) -> Result<Value, ConfigError> {
    let mut layer = Map::new();
    for (name, value) in env {
        match name {
            "HARNESS_PROVIDER" => {
                layer.insert("provider".into(), Value::String(value.to_string()));
            }
            "HARNESS_CONCURRENCY" => {
                let n: u32 = value.parse().map_err(|_| {
                    ConfigError::Invalid("HARNESS_CONCURRENCY must be a positive integer".into())
                })?;
                layer.insert("worker".into(), json!({ "default_concurrency": n }));
            }
            _ => {}
        }
    }
    Ok(Value::Object(layer))
}

/// Deep-merge `over` into `base`; objects merge recursively, everything else is replaced.
fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(existing) => merge(existing, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}
