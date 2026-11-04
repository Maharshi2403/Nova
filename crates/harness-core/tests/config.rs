use std::fs;
use std::path::Path;

use harness_core::config::{CliOverrides, Config, ConfigError, Provider};

fn none_env() -> Vec<(&'static str, &'static str)> {
    Vec::new()
}

#[test]
fn config_default_mock() {
    let c = Config::resolve(None, none_env(), &CliOverrides::default()).unwrap();
    assert_eq!(c.provider, Provider::Mock);
    assert_eq!(c.worker.default_concurrency, 2);
    assert!(!c.execution.allow_network);
    assert!(!c.execution.allow_auto_push);
    assert!(c.security.loopback_only);
}

#[test]
fn example_file_equals_defaults() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/defaults.example.yaml");
    let text = fs::read_to_string(path).unwrap();
    let c = Config::resolve(Some(&text), none_env(), &CliOverrides::default()).unwrap();
    assert_eq!(c, Config::default());
}

#[test]
fn config_precedence() {
    let file = "provider: openai\nworker:\n  default_concurrency: 3\n";
    let from_file = Config::resolve(Some(file), none_env(), &CliOverrides::default()).unwrap();
    assert_eq!(from_file.provider, Provider::Openai);
    assert_eq!(from_file.worker.default_concurrency, 3);

    let env = [("HARNESS_PROVIDER", "mock"), ("HARNESS_CONCURRENCY", "4")];
    let from_env = Config::resolve(Some(file), env, &CliOverrides::default()).unwrap();
    assert_eq!(from_env.provider, Provider::Mock);
    assert_eq!(from_env.worker.default_concurrency, 4);

    let cli = CliOverrides {
        provider: Some("openai".into()),
        concurrency: Some(1),
    };
    let from_cli = Config::resolve(Some(file), env, &cli).unwrap();
    assert_eq!(from_cli.provider, Provider::Openai);
    assert_eq!(from_cli.worker.default_concurrency, 1);
}

#[test]
fn config_invalid_concurrency() {
    for n in [0u32, 5, 100] {
        let cli = CliOverrides {
            provider: None,
            concurrency: Some(n),
        };
        let err = Config::resolve(None, none_env(), &cli).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)), "n={n}");
    }
    let bad_env = [("HARNESS_CONCURRENCY", "many")];
    assert!(Config::resolve(None, bad_env, &CliOverrides::default()).is_err());
}

#[test]
fn unsafe_or_unknown_settings_rejected() {
    for yaml in [
        "execution:\n  allow_auto_push: true\n",
        "security:\n  log_raw_secrets: true\n",
        "mystery_key: 1\n",
        "provider: carrier-pigeon\n",
        "worker:\n  maximum_concurrency: 9\n",
        "budget:\n  max_task_usd: 50\n",
    ] {
        assert!(
            Config::resolve(Some(yaml), none_env(), &CliOverrides::default()).is_err(),
            "accepted: {yaml}"
        );
    }
}

#[test]
fn unrelated_env_is_ignored() {
    let env = [("PATH", "/bin"), ("OPENAI_API_KEY", "sk-test")];
    let c = Config::resolve(None, env, &CliOverrides::default()).unwrap();
    assert_eq!(c, Config::default());
}
