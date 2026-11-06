//! Rust owns routing, per-call grants and credentials. Coder processes never get API keys.
use crate::worker::{Worker, WorkerSpec};
use harness_core::{
    domain::{Task, UseCase},
    messages::{PriceSource, ToolRequest, ToolResponse, Usage},
    protocol::{Envelope, MessageKind, ProtocolVersion},
    registry::{ModelDefinition, ModelRegistry},
    routing::{RoutingPolicy, route},
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LiveConfig {
    #[serde(default)]
    pub providers: std::collections::BTreeMap<String, crate::provider_config::ProviderKind>,
    #[serde(default)]
    pub roles: std::collections::BTreeMap<UseCase, RoutingPolicy>,
    pub registry: ModelRegistry,
    pub routing: RoutingPolicy,
    pub run_cap_microdollars: u64,
    pub daily_cap_microdollars: u64,
}

impl LiveConfig {
    pub fn select(&self, task: &Task) -> Result<ModelDefinition, String> {
        self.registry.validate().map_err(|e| e.to_string())?;
        let selected = route(
            &self.registry,
            self.roles.get(&task.spec.use_case).unwrap_or(&self.routing),
            task.spec.complexity,
            task.spec.risk,
        )
        .map_err(|e| e.to_string())?;
        let model = self
            .registry
            .require(&selected.model_id, Some("repo.patch"), true)
            .map_err(|e| e.to_string())?;
        if (!matches!(model.provider.as_str(), "openai" | "anthropic")
            && !self.providers.contains_key(&model.provider))
            || model.context_window < 8192
            || self.run_cap_microdollars == 0
            || self.daily_cap_microdollars == 0
            || task.spec.limits.cost_cap_usd <= 0.0
        {
            return Err("invalid live model capabilities or zero budget".into());
        }
        Ok(model.clone())
    }
}

pub struct LiveBroker {
    pub model: ModelDefinition,
    task_cap: f64,
    authorized_cost: f64,
    remaining_output: u64,
    calls: u32,
    grant: Option<(u32, usize)>,
    python: PathBuf,
    api_key: String,
    profile: crate::provider_config::ProviderProfile,
    run: String,
    task: String,
    attempt: u32,
}
impl LiveBroker {
    pub fn new(model: ModelDefinition, task: &Task, python: PathBuf, api_key: String) -> Self {
        harness_core::redaction::register_secret(&api_key);
        let (kind, base_url, variable) = if model.provider == "anthropic" {
            (
                crate::provider_config::ProviderKind::Anthropic,
                "https://api.anthropic.com/v1",
                "ANTHROPIC_API_KEY",
            )
        } else {
            (
                crate::provider_config::ProviderKind::Openai,
                "https://api.openai.com/v1",
                "OPENAI_API_KEY",
            )
        };
        Self {
            model: model.clone(),
            task_cap: task.spec.limits.cost_cap_usd,
            authorized_cost: 0.0,
            remaining_output: u64::from(task.spec.limits.max_output_tokens),
            calls: 0,
            grant: None,
            python,
            profile: crate::provider_config::ProviderProfile {
                id: model.provider.clone(),
                kind,
                base_url: base_url.into(),
                credential: crate::provider_config::CredentialRef::Environment {
                    variable: variable.into(),
                },
            },
            api_key,
            run: task.run_id.to_string(),
            task: task.id().to_string(),
            attempt: task.attempt,
        }
    }
    pub fn with_profile(mut self, profile: crate::provider_config::ProviderProfile) -> Self {
        self.profile = profile;
        self
    }
    pub fn context_cap(&self) -> usize {
        (self
            .model
            .context_window
            .saturating_sub(self.model.max_output_tokens)
            .saturating_sub(4096) as usize)
            .clamp(1024, 32768)
    }
    pub fn runtime_context(&self) -> serde_json::Value {
        serde_json::json!({"max_output_tokens": self.model.max_output_tokens, "context_byte_cap":self.context_cap(), "input_usd_per_million_tokens":self.model.input_usd_per_million_tokens, "output_usd_per_million_tokens":self.model.output_usd_per_million_tokens})
    }
    pub fn authorize(&mut self, request: &ToolRequest) -> ToolResponse {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            model_id: String,
            max_output_tokens: u32,
            input_byte_bound: usize,
            retry_attempt: u32,
        }
        let Ok(args) =
            serde_json::from_value::<Args>(serde_json::Value::Object(request.arguments.clone()))
        else {
            return denied("invalid model grant");
        };
        if self.grant.is_some()
            || self.calls >= 48
            || args.model_id != self.model.model_id
            || !(1..=3).contains(&args.retry_attempt)
            || args.max_output_tokens == 0
            || args.max_output_tokens > self.model.max_output_tokens
            || u64::from(args.max_output_tokens) > self.remaining_output
            || args.input_byte_bound > self.context_cap() + 4096
            || (args.input_byte_bound as u64 + u64::from(args.max_output_tokens))
                > u64::from(self.model.context_window)
        {
            return denied("model grant exceeds policy");
        }
        let cost = (args.input_byte_bound as f64 * self.model.input_usd_per_million_tokens
            + f64::from(args.max_output_tokens) * self.model.output_usd_per_million_tokens)
            / 1_000_000.0;
        if !cost.is_finite() || self.authorized_cost + cost > self.task_cap {
            return denied("task spend cap exhausted");
        }
        self.authorized_cost += cost;
        self.calls += 1;
        self.grant = Some((args.max_output_tokens, args.input_byte_bound));
        ToolResponse {
            ok: true,
            result: Some(serde_json::json!({"authorized":true})),
            error: None,
        }
    }
    pub async fn complete(&mut self, request: &ToolRequest) -> (ToolResponse, Option<Usage>) {
        let Some((output_cap, input_bound)) = self.grant.take() else {
            return (denied("no authorized model grant"), None);
        };
        let args = &request.arguments;
        let input_size = serde_json::to_vec(&args.get("messages"))
            .map_or(usize::MAX, |v| v.len())
            .saturating_add(serde_json::to_vec(&args.get("schema")).map_or(usize::MAX, |v| v.len()))
            .saturating_add(256);
        if args.get("model_id").and_then(|v| v.as_str()) != Some(self.model.model_id.as_str())
            || args.get("max_output_tokens").and_then(|v| v.as_u64()) != Some(u64::from(output_cap))
            || input_size > input_bound
        {
            return (denied("model request differs from its grant"), None);
        }
        let timeout = args
            .get("timeout_seconds")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        if !timeout.is_finite() || timeout <= 0.0 || timeout > 120.0 {
            return (denied("invalid provider timeout"), None);
        }
        let mut spec = WorkerSpec::python_worker(&self.python);
        spec.args = ["-I", "-m", "harness_ai", "provider"]
            .map(Into::into)
            .to_vec();
        spec.env = self.profile.adapter_env(&self.api_key);
        // Only this adapter subprocess gets credentials; cwd is never a repository.
        let outcome = invoke(
            spec,
            request,
            &self.run,
            &self.task,
            self.attempt,
            Duration::from_secs_f64(timeout + 5.0),
        )
        .await;
        let unknown = Usage {
            provider: self.model.provider.clone(),
            model_id: self.model.model_id.clone(),
            input_tokens: None,
            output_tokens: None,
            cached_input_tokens: None,
            cost_usd: None,
            price_source: PriceSource::Unknown,
        };
        let response = match outcome {
            Ok(v) => v,
            Err(reason) => {
                return (
                    ToolResponse {
                        ok: false,
                        result: None,
                        error: Some(format!("transient_provider: {reason}")),
                    },
                    Some(unknown),
                );
            }
        };
        if !response.ok {
            return (response, Some(unknown));
        }
        let Some(result) = response.result.as_ref() else {
            return (denied("missing provider result"), Some(unknown));
        };
        let input = result["usage"]["input_tokens"].as_u64();
        let output = result["usage"]["output_tokens"].as_u64();
        let cached = result["usage"]["cached_input_tokens"].as_u64();
        if result["provider"].as_str() != Some(self.model.provider.as_str())
            || result["model_id"].as_str() != Some(self.model.model_id.as_str())
            || input.is_some_and(|n| n > input_bound as u64)
            || output.is_some_and(|n| n > u64::from(output_cap))
            || matches!((cached,input), (Some(c),Some(i)) if c>i)
        {
            return (denied("invalid provider measurements"), Some(unknown));
        }
        self.remaining_output = output.map_or(0, |n| self.remaining_output.saturating_sub(n));
        let cost = input.zip(output).map(|(i, o)| {
            (i as f64 * self.model.input_usd_per_million_tokens
                + o as f64 * self.model.output_usd_per_million_tokens)
                / 1_000_000.0
        });
        (
            response,
            Some(Usage {
                cost_usd: cost,
                input_tokens: input,
                output_tokens: output,
                cached_input_tokens: cached,
                price_source: if cost.is_some() {
                    PriceSource::RegistryEstimate
                } else {
                    PriceSource::Unknown
                },
                ..unknown
            }),
        )
    }
}
fn denied(message: &str) -> ToolResponse {
    ToolResponse {
        ok: false,
        result: None,
        error: Some(message.into()),
    }
}
pub(crate) async fn invoke(
    spec: WorkerSpec,
    request: &ToolRequest,
    run: &str,
    task: &str,
    attempt: u32,
    timeout: Duration,
) -> Result<ToolResponse, String> {
    let mut worker = Worker::spawn(&spec).map_err(|_| "provider spawn failed")?;
    let result = async {
        worker
            .handshake(Duration::from_secs(10))
            .await
            .map_err(|_| "provider handshake failed")?;
        let id = uuid::Uuid::new_v4();
        let envelope = Envelope {
            protocol_version: ProtocolVersion::V1_0,
            kind: MessageKind::ToolRequest,
            message_id: id,
            correlation_id: None,
            run_id: Some(run.into()),
            task_id: Some(task.into()),
            attempt: Some(attempt),
            timestamp: time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .expect("timestamp"),
            payload: serde_json::to_value(request)
                .unwrap()
                .as_object()
                .unwrap()
                .clone(),
        };
        worker
            .send(&envelope)
            .await
            .map_err(|_| "provider send failed")?;
        let frame = worker
            .recv(timeout)
            .await
            .map_err(|_| "provider timeout")?
            .ok_or("provider closed")?;
        if frame.kind != MessageKind::ToolResponse
            || frame.correlation_id != Some(id.to_string())
            || frame.run_id != envelope.run_id
            || frame.task_id != envelope.task_id
            || frame.attempt != Some(attempt)
        {
            return Err("provider response correlation mismatch");
        }
        serde_json::from_value(frame.payload.into()).map_err(|_| "provider response invalid")
    }
    .await;
    worker.shutdown(Duration::from_millis(500)).await;
    result.map_err(str::to_owned)
}
