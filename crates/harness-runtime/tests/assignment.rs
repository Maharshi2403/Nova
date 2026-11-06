use std::path::PathBuf;
use std::time::Duration;

use harness_core::messages::*;
use harness_runtime::worker::{Worker, WorkerSpec};
use serde_json::json;

const T: Duration = Duration::from_secs(30);

fn python() -> PathBuf {
    if let Some(p) = std::env::var_os("HARNESS_PYTHON") {
        return p.into();
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
    exe
}

fn start(provider: &str) -> StartTask {
    StartTask {
        role: "coder".into(),
        worktree_id: "wt-1".into(),
        model: ModelRef {
            provider: provider.into(),
            model_id: "mock-coder".into(),
        },
        context_manifest_hash: "sha256:abc".into(),
        allowed_tools: vec!["repo.read".into()],
        max_tool_calls: 5,
        max_model_turns: None,
        max_output_tokens: 1000,
        timeout_seconds: 60,
        cost_cap_usd: 0.0,
    }
}

async fn next(w: &mut Worker, s: &mut AssignmentSession) -> WorkerMessage {
    let env = w.recv(T).await.unwrap().expect("worker closed early");
    s.accept(&env).unwrap()
}

#[tokio::test]
async fn mock_worker_full_assignment() {
    let mut w = Worker::spawn(&WorkerSpec::python_worker(python())).unwrap();
    w.handshake(T).await.unwrap();
    let mut s = AssignmentSession::new("run-1", "task-1", 1, 5);
    w.send(&s.start_task(&start("mock"), "2026-01-01T00:00:00Z".into()))
        .await
        .unwrap();

    assert!(matches!(
        next(&mut w, &mut s).await,
        WorkerMessage::Progress(_)
    ));
    let WorkerMessage::ToolRequest {
        request_id,
        request,
    } = next(&mut w, &mut s).await
    else {
        panic!("expected tool request");
    };
    assert_eq!(request.tool, "repo.read");
    let resp = ToolResponse {
        ok: true,
        result: Some(json!({"content": "hello", "truncated": false})),
        error: None,
    };
    let env = s
        .tool_response(request_id, &resp, "2026-01-01T00:00:01Z".into())
        .unwrap();
    w.send(&env).await.unwrap();

    let WorkerMessage::Usage(u) = next(&mut w, &mut s).await else {
        panic!("expected usage");
    };
    assert_eq!(u.input_tokens, None);
    assert!(matches!(
        next(&mut w, &mut s).await,
        WorkerMessage::Completed(_)
    ));
    assert!(s.is_finished());
    let report = w.shutdown(T).await;
    assert_eq!(report.code, Some(0), "{}", report.stderr);
}

#[tokio::test]
async fn non_mock_provider_fails_the_attempt() {
    let mut w = Worker::spawn(&WorkerSpec::python_worker(python())).unwrap();
    w.handshake(T).await.unwrap();
    let mut s = AssignmentSession::new("run-1", "task-1", 1, 5);
    w.send(&s.start_task(&start("openai"), "2026-01-01T00:00:00Z".into()))
        .await
        .unwrap();
    let WorkerMessage::Failed(f) = next(&mut w, &mut s).await else {
        panic!("expected failed");
    };
    assert!(!f.retryable);
    w.shutdown(T).await;
}
