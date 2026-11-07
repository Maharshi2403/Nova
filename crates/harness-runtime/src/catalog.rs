//! Account discovery does not generate tokens. Unknown metadata stays unavailable.
use crate::{
    provider_config::{ProviderKind, ProviderProfile, SavedModel},
    worker::WorkerSpec,
};
use serde_json::Value;
use std::{path::Path, time::Duration};
pub async fn discover(
    profile: &ProviderProfile,
    key: &str,
    python: &Path,
) -> Result<Vec<SavedModel>, String> {
    let mut spec = WorkerSpec::python_worker(python);
    spec.args = ["-I", "-m", "harness_ai", "provider"]
        .map(Into::into)
        .to_vec();
    spec.env = profile.adapter_env(key);
    let request = harness_core::messages::ToolRequest {
        tool: "models.list".into(),
        arguments: Default::default(),
    };
    let response = crate::live::invoke(
        spec,
        &request,
        "provider-setup",
        "catalog",
        1,
        Duration::from_secs(35),
    )
    .await?;
    if !response.ok {
        return Err("provider discovery failed; check the API key and endpoint".into());
    }
    let result = response.result.ok_or("missing model catalog")?;
    let rows = result["models"].as_array().ok_or("invalid model catalog")?;
    Ok(rows
        .iter()
        .filter_map(|row| normalize(profile, row))
        .collect())
}
pub fn normalize(profile: &ProviderProfile, row: &Value) -> Option<SavedModel> {
    let id = row["id"].as_str()?;
    let known = match (profile.kind, id) {
        (ProviderKind::Openai, "gpt-6-astra") => Some((10., 50., 1_050_000, 128_000, "high")),
        (ProviderKind::Openai, "gpt-6.1-sol") => Some((2., 10., 1_050_000, 128_000, "medium")),
        (ProviderKind::Openai, "gpt-6.1-terra") => Some((1., 5., 1_050_000, 128_000, "medium")),
        (ProviderKind::Openai, "gpt-6-luna") => Some((0.1, 0.5, 1_050_000, 128_000, "low")),
        (ProviderKind::Anthropic, "claude-opus-5-5") => Some((4., 20., 1_000_000, 128_000, "high")),
        (ProviderKind::Anthropic, "claude-sonnet-5-5") => {
            Some((2., 10., 1_000_000, 128_000, "medium"))
        }
        // Haiku's higher-context rate is used conservatively for every call.
        (ProviderKind::Anthropic, "claude-haiku-5-5") => {
            Some((0.5, 2.5, 1_000_000, 128_000, "low"))
        }
        _ => None,
    };
    let mut model = SavedModel {
        id: format!("{}/{}", profile.id, id),
        provider: profile.id.clone(),
        model_id: id.into(),
        display_name: row["name"].as_str().unwrap_or(id).into(),
        context_window: None,
        max_output_tokens: None,
        input_usd_per_million_tokens: None,
        output_usd_per_million_tokens: None,
        cached_input_usd_per_million_tokens: None,
        cache_write_usd_per_million_tokens: None,
        structured_output: false,
        tier: "medium".into(),
        metadata_source: "unknown; configure explicitly".into(),
    };
    if let Some((input, output, context, cap, tier)) = known {
        model.context_window = Some(context);
        model.max_output_tokens = Some(cap);
        model.input_usd_per_million_tokens = Some(input);
        model.output_usd_per_million_tokens = Some(output);
        model.structured_output = true;
        model.tier = tier.into();
        model.metadata_source = "bundled official metadata 2026-11-05; estimated pricing".into();
    }
    if profile.kind == ProviderKind::Compatible {
        let price = |name: &str| {
            row["pricing"][name]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v >= 0.)
                .map(|v| v * 1_000_000.)
        };
        model.context_window = row["context_length"]
            .as_u64()
            .and_then(|n| n.try_into().ok());
        model.max_output_tokens = row["top_provider"]["max_completion_tokens"]
            .as_u64()
            .and_then(|n| n.try_into().ok());
        model.input_usd_per_million_tokens = price("prompt");
        model.output_usd_per_million_tokens = price("completion");
        model.structured_output = row["supported_parameters"]
            .as_array()
            .is_some_and(|p| p.iter().any(|p| p == "structured_outputs"));
        if row["pricing"]["request"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .is_some_and(|v| v != 0.)
        {
            model.input_usd_per_million_tokens = None;
        }
        model.metadata_source = "provider catalog; estimated pricing".into();
    }
    Some(model)
}
