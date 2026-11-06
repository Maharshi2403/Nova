use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn request(root: &std::path::Path, value: Value) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_nova"))
        .arg("--daemon-dir")
        .arg(root)
        .arg("__desktop-request")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(value.to_string().as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
fn call(root: &std::path::Path, value: Value) -> Value {
    let result = request(root, value);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}
fn api(root: &std::path::Path, method: &str, path: &str, body: Value) -> Value {
    call(
        root,
        json!({"op":"api","method":method,"path":path,"body":body}),
    )
}
#[test]
fn bridge_rejects_injection_and_never_echoes_keys() {
    let dir = tempfile::tempdir().unwrap();
    let out = request(
        dir.path(),
        json!({"op":"api","method":"GET","path":"/v1/control\r\nInjected: value"}),
    );
    assert!(!out.status.success());
    let out = request(
        dir.path(),
        json!({"op":"connect","name":"bad/name","kind":"openai","base_url":"https://api.openai.com/v1","key":"desktop-fixture-secret"}),
    );
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("desktop-fixture-secret"));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("desktop-fixture-secret"));
    assert!(!dir.path().join("providers.json").exists());
}
#[test]
fn key_update_rejects_environment_credentials_without_exposing_secret() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let added = Command::new(env!("CARGO_BIN_EXE_nova"))
        .arg("--daemon-dir")
        .arg(root)
        .args([
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
        ])
        .env("NOVA_FIXTURE_KEY", "fixture-key")
        .output()
        .unwrap();
    assert!(added.status.success());
    let settings = call(root, json!({"op":"settings"}));
    assert_eq!(settings["providers"][0]["key_editable"], false);
    let secret = "replacement-fixture-secret";
    let out = request(
        root,
        json!({"op":"update_key","name":"fixture","key":secret}),
    );
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stderr).contains(secret));
    assert!(!String::from_utf8_lossy(&out.stdout).contains(secret));
    assert!(
        !std::fs::read_to_string(root.join("providers.json"))
            .unwrap()
            .contains(secret)
    );
}
struct Service(std::path::PathBuf);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = Command::new(env!("CARGO_BIN_EXE_nova"))
            .arg("--daemon-dir")
            .arg(&self.0)
            .arg("stop")
            .output();
    }
}
#[test]
fn native_bridge_review_approve_execute_and_evidence_without_paid_models() {
    verify_workflow(false);
}
#[test]
fn native_bridge_snapshot_executes_with_unfinished_files_preserved() {
    verify_workflow(true);
}
fn verify_workflow(snapshot: bool) {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.name", "Nova Fixture"],
        vec!["config", "user.email", "fixture@example.test"],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    std::fs::write(repo.join("README.md"), "fixture\n").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["add", "README.md"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["commit", "-qm", "fixture"])
            .status()
            .unwrap()
            .success()
    );
    if snapshot {
        std::fs::write(repo.join("README.md"), "unfinished tracked work\n").unwrap();
        std::fs::write(repo.join("draft.txt"), "private unfinished draft\n").unwrap();
    }
    call(&state, json!({"op":"start"}));
    let _service = Service(state.clone());
    let project = api(
        &state,
        "POST",
        "/v1/projects",
        json!({"name":"Desktop fixture","repo_path":repo,"trusted_local":true}),
    );
    let python = harness_runtime::engine::default_python(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../intelligence/.venv"),
    );
    let command = json!([
        python,
        "-c",
        "from pathlib import Path; assert Path('output.txt').read_text().strip() == 'Mock task desktop-task'; assert Path('README.md').read_text().strip() == 'fixture'"
    ]);
    let plan = json!({"schema_version":"1.0","project_id":project["id"],"idea":"Create a disposable desktop fixture","requirements":["Write the expected fixture"],"assumptions":[],"architecture":{"recommended":"A single disposable fixture file","alternatives":[],"risks":[]},"tasks":[{"id":"desktop-task","title":"Create fixture file","description":"Write the expected synthetic fixture file.","use_case":"coding","complexity":"low","risk":"low","agent_priority":50,"manual_priority":null,"dependencies":[],"candidate_paths":["output.txt"],"acceptance":[{"kind":"test","command":command}],"limits":{"max_tool_calls":4,"max_output_tokens":1000,"timeout_seconds":30,"cost_cap_usd":0.01}}]});
    let draft = call(&state, json!({"op":"create_plan","plan":plan}));
    let run = draft["run"]["id"].as_str().unwrap();
    let denied = request(
        &state,
        json!({"op":"execute","run_id":run,"version":1,"live":false,"committed_snapshot":snapshot}),
    );
    assert!(!denied.status.success());
    api(
        &state,
        "POST",
        &format!("/v1/plans/{run}/1/approve"),
        Value::Null,
    );
    if snapshot {
        let denied = request(
            &state,
            json!({"op":"execute","run_id":run,"version":1,"live":false}),
        );
        assert!(!denied.status.success());
        assert_eq!(
            api(
                &state,
                "GET",
                &format!("/v1/runs/{run}/source"),
                Value::Null
            ),
            Value::Null
        );
    }
    let accepted = call(
        &state,
        json!({"op":"execute","run_id":run,"version":1,"live":false,"committed_snapshot":snapshot}),
    );
    assert_eq!(accepted["execution"], "started");
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let status = api(&state, "GET", &format!("/v1/runs/{run}"), Value::Null);
        if status["state"] == "completed" {
            break;
        }
        assert!(Instant::now() < deadline, "run did not complete: {status}");
        std::thread::sleep(Duration::from_millis(100));
    }
    let evidence = api(
        &state,
        "GET",
        &format!("/v1/tasks/{run}/desktop-task/evidence"),
        Value::Null,
    );
    assert_eq!(evidence["tests"][0]["exit_code"], 0);
    assert!(evidence["commit"]["integration_commit_sha"].is_string());
    let inventory = call(&state, json!({"op":"snapshot"}));
    assert_eq!(inventory["runs"][0]["plan_version"], 1);
    assert_eq!(inventory["runs"][0]["state"], "completed");
    assert!(!repo.join("output.txt").exists());
    let source = api(
        &state,
        "GET",
        &format!("/v1/runs/{run}/source"),
        Value::Null,
    );
    assert!(source["base_sha"].is_string());
    assert_eq!(source["committed_snapshot"], snapshot);
    if snapshot {
        assert_eq!(
            std::fs::read_to_string(repo.join("README.md")).unwrap(),
            "unfinished tracked work\n"
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("draft.txt")).unwrap(),
            "private unfinished draft\n"
        );
    }
}
