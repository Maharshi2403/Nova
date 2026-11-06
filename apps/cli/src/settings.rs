//! Operator-owned worker interpreter and live spend ceilings.
use clap::Subcommand;
use harness_runtime::provider_config::ProviderStore;
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Set {
        #[command(subcommand)]
        setting: ConfigSetting,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigSetting {
    Python { path: PathBuf },
}

#[derive(Debug, Subcommand)]
pub enum BudgetCommand {
    Set {
        #[arg(long)]
        run: f64,
        #[arg(long)]
        daily: f64,
    },
}

pub fn microdollars(usd: f64) -> Result<u64, String> {
    if !usd.is_finite() || !(0.000_001..=(u64::MAX as f64 / 1_000_000.0)).contains(&usd) {
        return Err("spend cap must be a positive finite USD amount of at least $0.000001".into());
    }
    let amount = (usd * 1_000_000.0).round() as u64;
    if amount == 0 {
        return Err("spend cap rounds to zero microdollars".into());
    }
    Ok(amount)
}

pub fn config(root: &Path, command: ConfigCommand, json_output: bool) -> Result<String, String> {
    let store = ProviderStore::new(root);
    match command {
        ConfigCommand::Set {
            setting: ConfigSetting::Python { path },
        } => {
            let python = store.python(Some(path))?;
            store.update(|settings| {
                settings.python = Some(python.clone());
                settings.record_audit("python.set", json!({"path": python}));
                Ok(())
            })?;
            if json_output {
                Ok(json!({"python": python}).to_string())
            } else {
                Ok(format!("Saved worker Python: {}", python.display()))
            }
        }
    }
}

pub fn budget(root: &Path, command: BudgetCommand, json_output: bool) -> Result<String, String> {
    let store = ProviderStore::new(root);
    match command {
        BudgetCommand::Set { run, daily } => {
            let run_cap = microdollars(run)?;
            let daily_cap = microdollars(daily)?;
            if run_cap > daily_cap {
                return Err("run cap cannot exceed daily cap".into());
            }
            store.update(|settings| {
                settings.run_cap_microdollars = run_cap;
                settings.daily_cap_microdollars = daily_cap;
                settings.record_audit(
                    "budget.set",
                    json!({"run_cap_microdollars":run_cap,"daily_cap_microdollars":daily_cap}),
                );
                Ok(())
            })?;
            if json_output {
                Ok(json!({"run_cap_usd":run,"daily_cap_usd":daily}).to_string())
            } else {
                Ok(format!(
                    "Saved spend ceilings: ${run:.6} per run, ${daily:.6} daily."
                ))
            }
        }
    }
}
