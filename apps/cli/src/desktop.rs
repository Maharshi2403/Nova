//! Native UI transport. No daemon bearer or provider secret is returned to the UI.
use super::{CliError, Client, providers};
use harness_runtime::provider_config::*;
use harness_runtime::vault::{CredentialVault, NativeVault};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Start,
    Snapshot {
        #[serde(default)]
        offset: u32,
    },
    Api {
        method: String,
        path: String,
        #[serde(default)]
        body: Option<Value>,
    },
    Settings,
    Connect {
        name: String,
        kind: ProviderKind,
        base_url: String,
        key: String,
        #[serde(default)]
        python: Option<PathBuf>,
    },
    Remove {
        name: String,
    },
    UpdateKey {
        name: String,
        key: String,
    },
    Route {
        use_case: String,
        model: String,
    },
    CreatePlan {
        plan: harness_core::plan::PlanProposal,
    },
    Execute {
        run_id: String,
        version: u32,
        live: bool,
        #[serde(default)]
        committed_snapshot: bool,
    },
}
fn failure(message: impl Into<String>) -> CliError {
    CliError::Desktop(message.into())
}
fn api(client: &Client, method: &str, path: &str, body: Option<Value>) -> Result<Value, CliError> {
    serde_json::from_str(&client.request(method, path, body)?).map_err(CliError::DiscoveryJson)
}
fn safe_path(path: &str) -> bool {
    path.starts_with("/v1/")
        && path.len() <= 2048
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_-?.=&%".contains(&b))
        && !path.contains("..")
        && !path.contains('%')
}
pub(super) fn request(root: &Path) -> Result<String, CliError> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| failure("cannot read desktop request"))?;
    if bytes.len() > 1024 * 1024 {
        return Err(failure("desktop request exceeds byte limit"));
    }
    let request: Request =
        serde_json::from_slice(&bytes).map_err(|_| failure("invalid desktop request"))?;
    let value = handle(root, request)?;
    serde_json::to_string(&value).map_err(CliError::DiscoveryJson)
}
fn handle(root: &Path, request: Request) -> Result<Value, CliError> {
    let store = ProviderStore::new(root);
    match request {
        Request::Start => {
            let value = super::startup::start(root, true)?;
            serde_json::from_str(&value).map_err(CliError::DiscoveryJson)
        }
        Request::Settings => {
            let settings = store.load().map_err(failure)?;
            let models:Vec<_>=settings.models.iter().map(|m|json!({"id":m.id,"name":m.display_name,"ready":m.ready(),"provider":m.provider,"source":m.metadata_source})).collect();
            Ok(
                json!({"providers":settings.providers.iter().map(|p|json!({"id":p.id,"kind":p.kind,"base_url":p.base_url,"key_editable":matches!(p.credential, CredentialRef::Keychain { .. })})).collect::<Vec<_>>(),"models":models,"routes":settings.routes,"run_cap_usd":settings.run_cap_microdollars as f64/1_000_000.,"daily_cap_usd":settings.daily_cap_microdollars as f64/1_000_000.}),
            )
        }
        Request::Connect {
            name,
            kind,
            base_url,
            key,
            python,
        } => {
            validate_key(&key).map_err(failure)?;
            harness_core::redaction::register_secret(&key);
            let profile = ProviderProfile {
                id: name.clone(),
                kind,
                base_url,
                credential: CredentialRef::Keychain {
                    account: uuid::Uuid::new_v4().to_string(),
                },
            };
            profile.validate().map_err(failure)?;
            if store
                .load()
                .map_err(failure)?
                .providers
                .iter()
                .any(|p| p.id == name)
            {
                return Err(failure("provider name already connected"));
            }
            let python = store.python(python).map_err(failure)?;
            let runtime = tokio::runtime::Runtime::new()
                .map_err(|_| failure("cannot start provider setup"))?;
            let models = runtime
                .block_on(harness_runtime::catalog::discover(&profile, &key, &python))
                .map_err(failure)?;
            let ready = models.iter().filter(|m| m.ready()).count();
            let count = models.len();
            let CredentialRef::Keychain { account } = &profile.credential else {
                unreachable!()
            };
            NativeVault.put(account, &key).map_err(failure)?;
            let result = store.update(|s| {
                if s.providers.iter().any(|p| p.id == name) {
                    return Err("provider already exists".into());
                }
                s.providers.push(profile.clone());
                s.models.extend(models);
                s.python = Some(python);
                Ok(())
            });
            if result.is_err() {
                let _ = NativeVault.delete(account);
            }
            result.map_err(failure)?;
            Ok(
                json!({"message":format!("Connected {name}: {count} models discovered, {ready} ready.")}),
            )
        }
        Request::Remove { name } => Ok(
            json!({"message":providers::providers(root,providers::ProviderCommand::Remove{name}).map_err(failure)?}),
        ),
        Request::UpdateKey { name, key } => {
            validate_key(&key).map_err(failure)?;
            harness_core::redaction::register_secret(&key);
            let settings = store.load().map_err(failure)?;
            let profile = settings
                .providers
                .iter()
                .find(|p| p.id == name)
                .ok_or_else(|| failure("unknown provider"))?;
            let CredentialRef::Keychain { account } = &profile.credential else {
                return Err(failure(
                    "this provider uses an environment variable for its key",
                ));
            };
            NativeVault.put(account, &key).map_err(failure)?;
            Ok(json!({"message":format!("Updated API key for {name}.")}))
        }
        Request::Route { use_case, model } if model.is_empty() => {
            let _: harness_core::domain::UseCase =
                serde_json::from_value(Value::String(use_case.clone()))
                    .map_err(|_| failure("unknown use case"))?;
            store
                .update(|settings| {
                    settings.routes.remove(&use_case);
                    Ok(())
                })
                .map_err(failure)?;
            Ok(json!({"message":"Using complexity and risk defaults."}))
        }
        Request::Route { use_case, model } => Ok(
            json!({"message":providers::routes(root,providers::RouteCommand::Set{use_case,model}).map_err(failure)?}),
        ),
        Request::Snapshot { offset } => api(
            &Client::from_data_dir(root)?,
            "GET",
            &format!("/v1/dashboard?offset={offset}&limit=100"),
            None,
        ),
        Request::Api { method, path, body } => {
            if !["GET", "POST", "PATCH", "DELETE"].contains(&method.as_str())
                || !safe_path(&path)
                || path.starts_with("/v1/shutdown")
            {
                return Err(failure("unsupported desktop API request"));
            }
            api(&Client::from_data_dir(root)?, &method, &path, body)
        }
        Request::CreatePlan { plan } => {
            plan.validate().map_err(|e| failure(e.to_string()))?;
            let client = Client::from_data_dir(root)?;
            let run = api(
                &client,
                "POST",
                "/v1/runs",
                Some(json!({"project_id":plan.project_id})),
            )?;
            let id = run["id"]
                .as_str()
                .ok_or_else(|| failure("missing created run identity"))?;
            let proposal = api(
                &client,
                "POST",
                "/v1/plans",
                Some(json!({"run_id":id,"plan":plan})),
            )
            .map_err(|_| {
                failure(format!(
                    "Run {id} was created but plan submission failed. Reload runs and inspect it."
                ))
            })?;
            Ok(json!({"run":run,"proposal":proposal}))
        }
        Request::Execute {
            run_id,
            version,
            live,
            committed_snapshot,
        } => {
            let _ = harness_core::domain::RunId::new(run_id.clone())
                .map_err(|_| failure("invalid run"))?;
            if !safe_path(&format!("/v1/runs/{run_id}")) {
                return Err(failure("unsupported run identity"));
            }
            let client = Client::from_data_dir(root)?;
            let record = api(
                &client,
                "GET",
                &format!("/v1/plans/{run_id}/{version}"),
                None,
            )?;
            if record["status"] != "approved" {
                return Err(failure("approve the reviewed plan before execution"));
            }
            let plan: harness_core::plan::PlanProposal =
                serde_json::from_value(record["plan"].clone()).map_err(CliError::DiscoveryJson)?;
            let mut commands = Vec::new();
            for task in plan.tasks {
                for acceptance in task.acceptance {
                    if let harness_core::domain::Acceptance::Test { command } = acceptance
                        && !commands.contains(&command)
                    {
                        commands.push(command);
                    }
                }
            }
            let python = store.python(None).map_err(failure)?;
            let config = if live {
                Some(
                    store
                        .load()
                        .map_err(failure)?
                        .live_config()
                        .map_err(failure)?,
                )
            } else {
                None
            };
            api(
                &client,
                "POST",
                &format!("/v1/runs/{run_id}/execute"),
                Some(
                    json!({"python":python,"allowed_test_commands":commands,"concurrency":2,"live":live,"model_config":config,"committed_snapshot":committed_snapshot}),
                ),
            )
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_paths_cannot_inject_headers_or_escape() {
        assert!(safe_path("/v1/events?run_id=abc-123&after=0"));
        for p in [
            "/healthz",
            "/v1/runs/../shutdown",
            "/v1/runs/a\r\nAuthorization:x",
            "/v1/runs/%0d",
        ] {
            assert!(!safe_path(p));
        }
    }
}
