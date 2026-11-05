//! Public provider configuration with exclusive/atomic updates. Secrets live in a vault.
use crate::vault::CredentialVault;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Openai,
    Anthropic,
    Compatible,
}
impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Compatible => "compatible",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum CredentialRef {
    Keychain { account: String },
    Environment { variable: String },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfile {
    pub id: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub credential: CredentialRef,
}
impl ProviderProfile {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        {
            return Err(
                "provider name must be 1..64 letters, digits, hyphens or underscores".into(),
            );
        }
        let url = url::Url::parse(&self.base_url).map_err(|_| "invalid provider base URL")?;
        let local = url.host_str().is_some_and(|h| {
            h == "localhost"
                || h.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || (url.scheme() != "https" && !(url.scheme() == "http" && local))
        {
            return Err(
                "provider endpoint must be HTTPS or loopback HTTP without URL credentials".into(),
            );
        }
        if self.kind == ProviderKind::Openai && self.base_url != "https://api.openai.com/v1" {
            return Err("OpenAI native connections use the official API endpoint".into());
        }
        if self.kind == ProviderKind::Anthropic && self.base_url != "https://api.anthropic.com/v1" {
            return Err("Anthropic native connections use the official API endpoint".into());
        }
        match &self.credential {
            CredentialRef::Keychain { account } if uuid::Uuid::parse_str(account).is_ok() => {}
            CredentialRef::Environment { variable }
                if !variable.is_empty()
                    && variable.len() <= 128
                    && variable
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') => {}
            _ => return Err("invalid credential reference".into()),
        }
        Ok(())
    }
    pub fn resolve(&self, vault: &impl CredentialVault) -> Result<String, String> {
        self.validate()?;
        let key = match &self.credential {
            CredentialRef::Keychain { account } => vault.get(account)?,
            CredentialRef::Environment { variable } => std::env::var(variable)
                .map_err(|_| "provider credential environment variable is not set")?,
        };
        validate_key(&key)?;
        harness_core::redaction::register_secret(&key);
        Ok(key)
    }
    pub fn adapter_env(&self, key: &str) -> Vec<(String, String)> {
        vec![
            ("HARNESS_ENABLE_LIVE".into(), "1".into()),
            ("NOVA_PROVIDER_KIND".into(), self.kind.as_str().into()),
            ("NOVA_PROVIDER_NAME".into(), self.id.clone()),
            ("NOVA_PROVIDER_BASE_URL".into(), self.base_url.clone()),
            ("NOVA_PROVIDER_API_KEY".into(), key.into()),
        ]
    }
}
pub fn validate_key(key: &str) -> Result<(), String> {
    if key.trim().is_empty() || key.len() > 4096 || key.chars().any(char::is_control) {
        return Err("API key must be nonempty and contain no control characters".into());
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedModel {
    pub id: String,
    pub provider: String,
    pub model_id: String,
    pub display_name: String,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub input_usd_per_million_tokens: Option<f64>,
    pub output_usd_per_million_tokens: Option<f64>,
    pub structured_output: bool,
    pub tier: String,
    pub metadata_source: String,
}
impl SavedModel {
    pub fn ready(&self) -> bool {
        self.structured_output
            && self.context_window.is_some_and(|n| n >= 8192)
            && self.max_output_tokens.is_some_and(|n| n > 0)
            && self
                .input_usd_per_million_tokens
                .is_some_and(|v| v.is_finite() && v >= 0.0)
            && self
                .output_usd_per_million_tokens
                .is_some_and(|v| v.is_finite() && v >= 0.0)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSettings {
    pub version: u32,
    pub providers: Vec<ProviderProfile>,
    pub models: Vec<SavedModel>,
    pub routes: BTreeMap<String, String>,
    pub python: Option<PathBuf>,
    pub run_cap_microdollars: u64,
    pub daily_cap_microdollars: u64,
}
impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            version: 1,
            providers: vec![],
            models: vec![],
            routes: BTreeMap::new(),
            python: None,
            run_cap_microdollars: 5_000_000,
            daily_cap_microdollars: 10_000_000,
        }
    }
}
impl ProviderSettings {
    fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.providers.len() > 64 || self.models.len() > 5000 {
            return Err("unsupported or oversized provider settings".into());
        }
        let mut providers = HashSet::new();
        for profile in &self.providers {
            profile.validate()?;
            if !providers.insert(&profile.id) {
                return Err("duplicate provider".into());
            }
        }
        let mut models = HashSet::new();
        for model in &self.models {
            if !providers.contains(&model.provider)
                || !models.insert(&model.id)
                || model.id != format!("{}/{}", model.provider, model.model_id)
                || model.id.len() > 200
                || model.model_id.is_empty()
            {
                return Err("invalid model catalog reference".into());
            }
            for value in [
                model.input_usd_per_million_tokens,
                model.output_usd_per_million_tokens,
            ]
            .into_iter()
            .flatten()
            {
                if !value.is_finite() || value < 0.0 {
                    return Err("invalid model price".into());
                }
            }
            if matches!((model.context_window,model.max_output_tokens),(Some(c),Some(o)) if o>c) {
                return Err("output cap exceeds context window".into());
            }
        }
        if self.routes.values().any(|id| !models.contains(id)) {
            return Err("route refers to missing model".into());
        }
        Ok(())
    }
}
pub struct ProviderStore {
    root: PathBuf,
}
impl ProviderStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn load(&self) -> Result<ProviderSettings, String> {
        let file = self.root.join("providers.json");
        if !file.exists() {
            return Ok(ProviderSettings::default());
        }
        if std::fs::metadata(&file)
            .map_err(|_| "cannot read provider settings")?
            .len()
            > 4 * 1024 * 1024
        {
            return Err("provider settings exceed byte limit".into());
        }
        let settings: ProviderSettings = serde_json::from_slice(
            &std::fs::read(file).map_err(|_| "cannot read provider settings")?,
        )
        .map_err(|_| "invalid provider settings JSON")?;
        settings.validate()?;
        Ok(settings)
    }
    pub fn update(
        &self,
        edit: impl FnOnce(&mut ProviderSettings) -> Result<(), String>,
    ) -> Result<ProviderSettings, String> {
        std::fs::create_dir_all(&self.root).map_err(|_| "cannot create Nova settings directory")?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("providers.lock"))
            .map_err(|_| "cannot lock provider settings")?;
        let deadline = Instant::now() + Duration::from_secs(2);
        while lock.try_lock().is_err() {
            if Instant::now() >= deadline {
                return Err("provider settings are busy; retry the command".into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let mut settings = self.load()?;
        edit(&mut settings)?;
        settings.validate()?;
        let bytes = serde_json::to_vec_pretty(&settings)
            .map_err(|_| "cannot serialize provider settings")?;
        let temporary = self
            .root
            .join(format!(".providers-{}.tmp", uuid::Uuid::new_v4()));
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|_| "cannot write provider settings")?;
        let result = (|| {
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, self.root.join("providers.json"))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
            return Err("cannot save provider settings".into());
        }
        Ok(settings)
    }
    pub fn python(&self, explicit: Option<PathBuf>) -> Result<PathBuf, String> {
        let saved = self.load()?.python;
        let bundle = std::env::current_exe().ok().and_then(|p| {
            let parent = p.parent()?;
            [Some(parent), parent.parent()]
                .into_iter()
                .flatten()
                .map(|root| crate::engine::default_python(&root.join("runtime")))
                .find(|python| python.is_file())
        });
        let source = crate::engine::default_python(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../intelligence/.venv"),
        );
        let candidate = explicit
            .or(saved)
            .or_else(|| std::env::var_os("NOVA_PYTHON").map(PathBuf::from))
            .or_else(|| bundle.filter(|p| p.is_file()))
            .or_else(|| source.is_file().then_some(source))
            .ok_or("intelligence Python not found; pass --python once")?;
        if !candidate.is_absolute() || !candidate.is_file() {
            return Err("configured intelligence Python must be an existing absolute path".into());
        }
        // Keep the venv entry path: canonicalizing its symlink selects the system
        // interpreter and drops the virtual environment's site-packages.
        Ok(candidate)
    }
}

/// Resolved credentials are never serializable or printable.
pub struct Connection {
    pub profile: ProviderProfile,
    pub key: String,
}
pub type Connections = std::collections::BTreeMap<String, Connection>;
impl ProviderStore {
    pub fn connections(&self, config: &crate::live::LiveConfig) -> Result<Connections, String> {
        let settings = self.load()?;
        let mut connections = Connections::new();
        for model in config.registry.models.iter().filter(|m| m.available) {
            if connections.contains_key(&model.provider) {
                continue;
            }
            let profile = settings
                .providers
                .iter()
                .find(|p| p.id == model.provider)
                .cloned()
                .or_else(|| {
                    let (kind, base, variable) = match model.provider.as_str() {
                        "openai" => (
                            ProviderKind::Openai,
                            "https://api.openai.com/v1",
                            "OPENAI_API_KEY",
                        ),
                        "anthropic" => (
                            ProviderKind::Anthropic,
                            "https://api.anthropic.com/v1",
                            "ANTHROPIC_API_KEY",
                        ),
                        _ => return None,
                    };
                    Some(ProviderProfile {
                        id: model.provider.clone(),
                        kind,
                        base_url: base.into(),
                        credential: CredentialRef::Environment {
                            variable: variable.into(),
                        },
                    })
                })
                .ok_or("model provider is not configured; use nova providers add")?;
            if config
                .providers
                .get(&model.provider)
                .is_some_and(|kind| *kind != profile.kind)
            {
                return Err("model config disagrees with saved provider protocol".into());
            }
            let key = profile.resolve(&crate::vault::NativeVault)?;
            connections.insert(model.provider.clone(), Connection { profile, key });
        }
        Ok(connections)
    }
}
impl ProviderSettings {
    pub fn live_config(&self) -> Result<crate::live::LiveConfig, String> {
        use harness_core::{
            registry::{ModelDefinition, ModelRegistry, PrivacyClass},
            routing::RoutingPolicy,
        };
        let mut ready: Vec<_> = self.models.iter().filter(|m| m.ready()).collect();
        ready.sort_by(|a, b| {
            a.input_usd_per_million_tokens
                .unwrap()
                .total_cmp(&b.input_usd_per_million_tokens.unwrap())
                .then(a.id.cmp(&b.id))
        });
        let cheapest = ready
            .first()
            .ok_or("no ready models; connect a provider or configure model metadata")?;
        let select = |tier: &str| {
            ready
                .iter()
                .find(|m| m.tier == tier)
                .copied()
                .unwrap_or(cheapest)
                .id
                .clone()
        };
        let routing = RoutingPolicy {
            low_model: select("low"),
            medium_model: select("medium"),
            high_model: select("high"),
        };
        let mut roles = BTreeMap::new();
        for (role, id) in &self.routes {
            if !ready.iter().any(|m| &m.id == id) {
                return Err("route selects a model without verified metadata".into());
            }
            let role = serde_json::from_value(serde_json::Value::String(role.clone()))
                .map_err(|_| "unknown route use case")?;
            roles.insert(
                role,
                RoutingPolicy {
                    low_model: id.clone(),
                    medium_model: id.clone(),
                    high_model: id.clone(),
                },
            );
        }
        Ok(crate::live::LiveConfig {
            registry: ModelRegistry {
                models: ready
                    .into_iter()
                    .map(|m| ModelDefinition {
                        id: m.id.clone(),
                        provider: m.provider.clone(),
                        model_id: m.model_id.clone(),
                        supported_tools: vec!["repo.patch".into(), "repo.read".into()],
                        structured_output: true,
                        context_window: m.context_window.unwrap(),
                        max_output_tokens: m.max_output_tokens.unwrap().min(8192),
                        input_usd_per_million_tokens: m.input_usd_per_million_tokens.unwrap(),
                        output_usd_per_million_tokens: m.output_usd_per_million_tokens.unwrap(),
                        available: true,
                        privacy: PrivacyClass::Private,
                    })
                    .collect(),
            },
            routing,
            providers: self
                .providers
                .iter()
                .map(|p| (p.id.clone(), p.kind))
                .collect(),
            roles,
            run_cap_microdollars: self.run_cap_microdollars,
            daily_cap_microdollars: self.daily_cap_microdollars,
        })
    }
}
