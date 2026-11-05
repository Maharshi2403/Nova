use std::path::PathBuf;
use std::time::{Duration, Instant};

use harness_core::messages::{ToolRequest, ToolResponse};
use harness_runtime::broker::command::*;
use harness_runtime::broker::write::RepoWriteBroker;
use harness_runtime::supervisor::ToolBroker;
use serde_json::{Value, json};
use tempfile::TempDir;

fn python() -> String {
    if let Some(p) = std::env::var_os("HARNESS_PYTHON") {
        return p.to_string_lossy().into_owned();
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../intelligence/.venv");
    let exe = if cfg!(windows) {
        root.join("Scripts/python.exe")
    } else {
        root.join("bin/python")
    };
    assert!(
        exe.exists(),
        "python venv not found; run `uv sync` in intelligence/"
    );
    exe.canonicalize()
        .unwrap()
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned()
}

fn broker(dir: &TempDir, allow_python: bool) -> RepoCommandBroker {
    let tools: Vec<String> = ["repo.read", "repo.patch", "cmd.run"]
        .map(String::from)
        .to_vec();
    let write = RepoWriteBroker::new(dir.path(), &tools, &["src".to_string()]).unwrap();
    let policy = if allow_python {
        CommandPolicy::new(vec![vec![python()]])
    } else {
        CommandPolicy::default()
    };
    RepoCommandBroker::new(write, dir.path(), &tools, policy).unwrap()
}

async fn cmd(b: &mut RepoCommandBroker, args: Value) -> ToolResponse {
    b.handle(&ToolRequest {
        tool: "cmd.run".into(),
        arguments: args.as_object().unwrap().clone(),
    })
    .await
}

fn argv(code: &str, extra: &[&str]) -> Vec<String> {
    let mut v = vec![python(), "-c".to_string(), code.to_string()];
    v.extend(extra.iter().map(|s| s.to_string()));
    v
}

#[tokio::test]
async fn unapproved_command_denied() {
    let dir = TempDir::new().unwrap();
    let marker = dir.path().join("pwned.txt");
    let mut b = broker(&dir, false);
    let code = format!("open(r'{}', 'w').write('x')", marker.display());
    let a = argv(&code, &[]);
    let r = cmd(&mut b, json!({"argv": a})).await;
    assert!(
        r.error.as_deref().unwrap().starts_with("approval_required"),
        "{r:?}"
    );
    assert!(!marker.exists(), "denied command must not run");
    assert_eq!(b.pending_approvals(), std::slice::from_ref(&a));

    // Allowlisting is by argv prefix, so a different program is still denied.
    let mut b2 = broker(&dir, true);
    let r = cmd(&mut b2, json!({"argv": ["definitely-not-allowed"]})).await;
    assert!(r.error.as_deref().unwrap().starts_with("approval_required"));

    // An explicit approval of the exact argv lets it run once approved.
    b.approve(a.clone());
    assert!(b.pending_approvals().is_empty());
    let r = cmd(&mut b, json!({"argv": a})).await;
    assert!(r.ok, "{r:?}");
    assert!(marker.exists());
}

#[tokio::test]
async fn literal_arg_not_shell_expanded() {
    let dir = TempDir::new().unwrap();
    let mut b = broker(&dir, true);
    let nasty = "$HOME `whoami` && echo pwned; *";
    let r = cmd(
        &mut b,
        json!({"argv": argv("import sys; print(sys.argv[1])", &[nasty])}),
    )
    .await;
    assert!(r.ok, "{r:?}");
    let v = r.result.unwrap();
    assert_eq!(v["stdout"].as_str().unwrap().trim(), nasty);
    assert_eq!(v["exit_code"], 0);
}

#[tokio::test]
async fn timeout_kills_child() {
    let dir = TempDir::new().unwrap();
    let mut b = broker(&dir, true);
    let started = Instant::now();
    let r = cmd(
        &mut b,
        json!({"argv": argv("import time; time.sleep(60)", &[]), "timeout_seconds": 1}),
    )
    .await;
    let v = r.result.expect("timeout is reported, not an error");
    assert_eq!(v["timed_out"], true);
    assert!(v["exit_code"].is_null());
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[tokio::test]
async fn timeout_kills_grandchildren_too() {
    let dir = TempDir::new().unwrap();
    let marker = dir.path().join("grandchild.txt");
    let child_code = format!(
        "import time; time.sleep(4); open(r'{}', 'w').write('alive')",
        marker.display()
    );
    let parent_code = format!(
        "import subprocess, sys, time; subprocess.Popen([sys.executable, '-c', {child_code:?}]); time.sleep(60)"
    );
    let mut b = broker(&dir, true);
    let r = cmd(
        &mut b,
        json!({"argv": argv(&parent_code, &[]), "timeout_seconds": 1}),
    )
    .await;
    assert_eq!(r.result.unwrap()["timed_out"], true);
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert!(!marker.exists(), "grandchild survived the timeout");
}

#[tokio::test]
async fn output_is_capped_and_nonzero_exit_reported() {
    let dir = TempDir::new().unwrap();
    let mut b = broker(&dir, true);
    let r = cmd(
        &mut b,
        json!({"argv": argv("import sys; sys.stdout.write('a' * 500000); sys.stderr.write('boom'); sys.exit(3)", &[])}),
    )
    .await;
    let v = r.result.unwrap();
    assert_eq!(v["exit_code"], 3);
    assert_eq!(v["truncated"], true);
    assert!(v["stdout"].as_str().unwrap().len() <= MAX_COMMAND_OUTPUT_BYTES);
    assert_eq!(v["stderr"], "boom");
}

#[tokio::test]
async fn cwd_must_stay_inside_worktree_and_env_is_scrubbed() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    let mut b = broker(&dir, true);
    for cwd in ["..", "/", "../x"] {
        let r = cmd(&mut b, json!({"argv": argv("print(1)", &[]), "cwd": cwd})).await;
        assert!(!r.ok, "{cwd} should be rejected");
    }
    // SAFETY-free check: a secret in our environment must not reach the child.
    unsafe { std::env::set_var("HARNESS_TEST_SECRET", "s3cret") };
    let r = cmd(
        &mut b,
        json!({"argv": argv("import os; print(os.environ.get('HARNESS_TEST_SECRET'))", &[]), "cwd": "src"}),
    )
    .await;
    assert_eq!(r.result.unwrap()["stdout"].as_str().unwrap().trim(), "None");
}

#[tokio::test]
async fn bad_requests_and_tool_gating() {
    let dir = TempDir::new().unwrap();
    let mut b = broker(&dir, true);
    for args in [
        json!({"argv": []}),
        json!({"argv": [""]}),
        json!({"argv": ["a"], "x": 1}),
        json!({}),
    ] {
        let r = cmd(&mut b, args).await;
        assert!(
            r.error.as_deref().unwrap().starts_with("bad_arguments"),
            "{r:?}"
        );
    }
    let tools = vec!["repo.read".to_string()];
    let write = RepoWriteBroker::new(dir.path(), &tools, &[]).unwrap();
    let mut no_cmd =
        RepoCommandBroker::new(write, dir.path(), &tools, CommandPolicy::default()).unwrap();
    let r = cmd(&mut no_cmd, json!({"argv": ["x"]})).await;
    assert!(r.error.as_deref().unwrap().starts_with("tool_not_allowed"));
}
