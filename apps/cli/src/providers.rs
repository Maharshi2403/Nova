use clap::Subcommand;
use harness_runtime::{
    provider_config::*,
    vault::{CredentialVault, NativeVault},
};
use std::path::{Path, PathBuf};
#[derive(Debug, Subcommand)]
pub enum ProviderCommand {
    /// Connect a provider; prompts for a hidden API key and discovers account models.
    Add {
        name: String,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        key_env: Option<String>,
        #[arg(long)]
        python: Option<PathBuf>,
        #[arg(long)]
        skip_discovery: bool,
    },
    List,
    Remove {
        name: String,
    },
}
#[derive(Debug, Subcommand)]
pub enum ModelCommand {
    List,
    Sync {
        provider: String,
    },
    /// Supply verified metadata for a model absent from the bundled catalog.
    Configure {
        provider: String,
        model: String,
        #[arg(long)]
        context: u32,
        #[arg(long)]
        max_output: u32,
        #[arg(long)]
        input_price: f64,
        #[arg(long)]
        output_price: f64,
        #[arg(long)]
        structured_output: bool,
        #[arg(long, default_value = "medium")]
        tier: String,
    },
}
fn discover(
    profile: &ProviderProfile,
    key: &str,
    python: &Path,
) -> Result<Vec<SavedModel>, String> {
    tokio::runtime::Runtime::new()
        .map_err(|_| "cannot start discovery runtime")?
        .block_on(harness_runtime::catalog::discover(profile, key, python))
}
pub fn providers(root: &Path, command: ProviderCommand) -> Result<String, String> {
    let store = ProviderStore::new(root);
    match command {
        ProviderCommand::List => serde_json::to_string_pretty(&store.load()?.providers)
            .map_err(|_| "cannot format providers".into()),
        ProviderCommand::Add {
            name,
            kind,
            base_url,
            key_env,
            python,
            skip_discovery,
        } => {
            if store.load()?.providers.iter().any(|p| p.id == name) {
                return Err("provider already exists; remove it before reconnecting".into());
            }
            let kind = match kind.as_deref().unwrap_or(&name) {
                "openai" => ProviderKind::Openai,
                "anthropic" | "claude" => ProviderKind::Anthropic,
                "compatible" | "openrouter" => ProviderKind::Compatible,
                _ => return Err("use --kind openai, anthropic or compatible".into()),
            };
            let base_url = base_url.unwrap_or_else(|| {
                match kind {
                    ProviderKind::Openai => "https://api.openai.com/v1",
                    ProviderKind::Anthropic => "https://api.anthropic.com/v1",
                    ProviderKind::Compatible if name == "openrouter" => {
                        "https://openrouter.ai/api/v1"
                    }
                    _ => "",
                }
                .into()
            });
            let credential = key_env
                .map(|variable| CredentialRef::Environment { variable })
                .unwrap_or_else(|| CredentialRef::Keychain {
                    account: uuid::Uuid::new_v4().to_string(),
                });
            let profile = ProviderProfile {
                id: name.clone(),
                kind,
                base_url,
                credential,
            };
            profile.validate()?;
            let key = match &profile.credential {
                CredentialRef::Environment { variable } => std::env::var(variable)
                    .map_err(|_| "credential environment variable is not set")?,
                _ => rpassword::prompt_password(
                    "API key (hidden, saved in your OS credential vault): ",
                )
                .map_err(|_| "cannot read hidden API key from terminal")?,
            };
            validate_key(&key)?;
            harness_core::redaction::register_secret(&key);
            let python = if skip_discovery {
                python
            } else {
                Some(store.python(python)?)
            };
            let models = if skip_discovery {
                vec![]
            } else {
                discover(&profile, &key, python.as_deref().unwrap())?
            };
            if let CredentialRef::Keychain { account } = &profile.credential {
                NativeVault.put(account, &key)?;
            }
            let result = store.update(|s| {
                if s.providers.iter().any(|p| p.id == name) {
                    return Err("provider already exists".into());
                }
                s.providers.push(profile.clone());
                s.models.extend(models);
                if python.is_some() {
                    s.python = python.clone();
                    s.record_audit("python.set", serde_json::json!({"path":python}));
                }
                Ok(())
            });
            if result.is_err()
                && let CredentialRef::Keychain { account } = &profile.credential
            {
                let _ = NativeVault.delete(account);
            }
            let settings = result?;
            Ok(format!(
                "Connected {name}. {} models discovered, {} ready. Use `nova models list`.",
                settings
                    .models
                    .iter()
                    .filter(|m| m.provider == name)
                    .count(),
                settings
                    .models
                    .iter()
                    .filter(|m| m.provider == name && m.ready())
                    .count()
            ))
        }
        ProviderCommand::Remove { name } => {
            // Commit removal first so a vault failure cannot leave an active broken profile.
            let mut removed = None;
            store.update(|s| {
                removed = s.providers.iter().find(|p| p.id == name).cloned();
                if removed.is_none() {
                    return Err("unknown provider".into());
                }
                s.providers.retain(|p| p.id != name);
                s.models.retain(|m| m.provider != name);
                s.routes
                    .retain(|_, id| !id.starts_with(&format!("{name}/")));
                Ok(())
            })?;
            if let Some(ProviderProfile {
                credential: CredentialRef::Keychain { account },
                ..
            }) = removed
            {
                NativeVault.delete(&account)?;
            }
            Ok(format!("Removed {name}."))
        }
    }
}
pub fn models(root: &Path, command: ModelCommand) -> Result<String, String> {
    let store = ProviderStore::new(root);
    match command {
        ModelCommand::List => {
            let models = store.load()?.models;
            Ok(models
                .iter()
                .map(|m| {
                    format!(
                        "{}  {}  [{}]",
                        m.id,
                        if m.ready() { "ready" } else { "needs metadata" },
                        m.metadata_source
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        ModelCommand::Sync { provider } => {
            let profile = store
                .load()?
                .providers
                .into_iter()
                .find(|p| p.id == provider)
                .ok_or("unknown provider")?;
            let key = profile.resolve(&NativeVault)?;
            let rows = discover(&profile, &key, &store.python(None)?)?;
            store.update(|s| {
                s.models.retain(|m| m.provider != provider);
                s.models.extend(rows);
                s.routes
                    .retain(|_, id| s.models.iter().any(|m| &m.id == id));
                Ok(())
            })?;
            Ok("Model catalog refreshed.".into())
        }
        ModelCommand::Configure {
            provider,
            model,
            context,
            max_output,
            input_price,
            output_price,
            structured_output,
            tier,
        } => {
            if !["low", "medium", "high"].contains(&tier.as_str()) {
                return Err("tier must be low, medium or high".into());
            }
            let row = SavedModel {
                id: format!("{provider}/{model}"),
                provider,
                model_id: model.clone(),
                display_name: model,
                context_window: Some(context),
                max_output_tokens: Some(max_output),
                input_usd_per_million_tokens: Some(input_price),
                output_usd_per_million_tokens: Some(output_price),
                structured_output,
                tier,
                metadata_source: "operator configured".into(),
            };
            if !row.ready() {
                return Err("model requires structured output, context >=8192, positive output cap and valid prices".into());
            }
            store.update(|s| {
                s.models.retain(|m| m.id != row.id);
                s.models.push(row);
                Ok(())
            })?;
            Ok("Model configured.".into())
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum RouteCommand {
    List,
    Set { use_case: String, model: String },
    Reset,
}
pub fn routes(root: &Path, command: RouteCommand) -> Result<String, String> {
    let store = ProviderStore::new(root);
    match command {
        RouteCommand::List => {
            let config = store.load()?.live_config()?;
            serde_json::to_string_pretty(&config).map_err(|_| "cannot format routing".into())
        }
        RouteCommand::Set { use_case, model } => {
            let _: harness_core::domain::UseCase = serde_json::from_value(
                serde_json::Value::String(use_case.clone()),
            )
            .map_err(
                |_| "use case must be coding, planning, review, debugging, architecture or fast",
            )?;
            store.update(|s| {
                if !s.models.iter().any(|m| m.id == model && m.ready()) {
                    return Err("route requires a ready model".into());
                }
                s.routes.insert(use_case, model);
                Ok(())
            })?;
            Ok("Route saved.".into())
        }
        RouteCommand::Reset => {
            store.update(|s| {
                s.routes.clear();
                Ok(())
            })?;
            Ok("Routes reset to complexity/risk defaults.".into())
        }
    }
}
