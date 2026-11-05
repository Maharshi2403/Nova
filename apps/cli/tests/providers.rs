use std::process::Command;
fn run(root: &std::path::Path, args: &[&str]) -> String {
    let result = Command::new(env!("CARGO_BIN_EXE_nova"))
        .arg("--daemon-dir")
        .arg(root)
        .args(args)
        .env("NOVA_FIXTURE_KEY", "not-a-real-key")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}
#[test]
fn offline_provider_setup_routes_and_removal_need_no_daemon() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run(
        root,
        &[
            "providers",
            "add",
            "fixture",
            "--kind",
            "compatible",
            "--base-url",
            "http://127.0.0.1:8080/v1",
            "--key-env",
            "NOVA_FIXTURE_KEY",
            "--skip-discovery",
        ],
    );
    assert!(
        !std::fs::read_to_string(root.join("providers.json"))
            .unwrap()
            .contains("not-a-real-key")
    );
    assert!(run(root, &["models", "list"]).trim().is_empty());
    run(
        root,
        &[
            "models",
            "configure",
            "fixture",
            "sample-model",
            "--context",
            "32000",
            "--max-output",
            "4096",
            "--input-price",
            "1",
            "--output-price",
            "2",
            "--structured-output",
        ],
    );
    run(root, &["routes", "set", "review", "fixture/sample-model"]);
    let routes = run(root, &["routes", "list"]);
    assert!(routes.contains("review"));
    assert!(routes.contains("fixture/sample-model"));
    run(root, &["providers", "remove", "fixture"]);
    assert_eq!(run(root, &["providers", "list"]).trim(), "[]");
    assert!(!root.join("daemon.json").exists());
}
