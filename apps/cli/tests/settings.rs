use harness_runtime::provider_config::ProviderStore;
use std::process::Command;

fn nova(root: &std::path::Path, args: &[&str]) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_nova"))
        .arg("--daemon-dir")
        .arg(root)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn config_and_budget_are_persisted_and_audited() {
    let dir = tempfile::tempdir().unwrap();
    let python = std::env::current_exe().unwrap();
    let saved = nova(
        dir.path(),
        &["config", "set", "python", python.to_str().unwrap()],
    );
    assert_eq!(saved["python"], python.to_str().unwrap());
    let budget = nova(dir.path(), &["budget", "set", "--run", "2", "--daily", "3"]);
    assert_eq!(budget["run_cap_usd"].as_f64(), Some(2.0));
    let settings = ProviderStore::new(dir.path()).load().unwrap();
    assert_eq!(settings.python.as_deref(), Some(python.as_path()));
    assert_eq!(settings.run_cap_microdollars, 2_000_000);
    assert_eq!(settings.daily_cap_microdollars, 3_000_000);
    assert_eq!(settings.audit.len(), 2);
    assert_eq!(settings.audit[0].action, "python.set");
    assert_eq!(settings.audit[1].action, "budget.set");
}

#[test]
fn budget_rejects_invalid_or_inverted_ceilings() {
    let dir = tempfile::tempdir().unwrap();
    for values in [["0", "1"], ["2", "1"], ["NaN", "1"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_nova"))
            .arg("--daemon-dir")
            .arg(dir.path())
            .args(["budget", "set", "--run", values[0], "--daily", values[1]])
            .output()
            .unwrap();
        assert!(!output.status.success());
    }
    assert!(!dir.path().join("providers.json").exists());
}
