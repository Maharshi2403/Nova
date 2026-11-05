use std::path::PathBuf;
use std::time::Duration;

use harness_core::messages::*;
use harness_runtime::supervisor::{Limits, Outcome, SuperviseError, ToolBroker, supervise};
use harness_runtime::worker::{Worker, WorkerSpec};
use serde_json::json;
use tokio::sync::watch;

const T: Duration = Duration::from_secs(30);

#[tokio::test]
async fn cancellation_interrupts_pending_broker_call() {
    struct Hung;
    impl ToolBroker for Hung {
        async fn handle(&mut self, _: &ToolRequest) -> ToolResponse {
            std::future::pending().await
        }
    }
    let mut worker = Worker::spawn(&scripted("write_frame(out, frame('tool_request', {'tool':'repo.read','arguments':{'path':'README.md'}}))\nread_frame(sys.stdin.buffer)")).unwrap();
    worker.handshake(T).await.unwrap();
    let mut session = AssignmentSession::new("run-1", "task-1", 1, 3);
    let (tx, rx) = watch::channel(false);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        tx.send(true).unwrap();
    });
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        supervise(&mut worker, &mut session, &start(), limits(), &mut Hung, rx),
    )
    .await
    .unwrap();
    assert!(matches!(result, Err(SuperviseError::Cancelled)));
    worker.shutdown(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn closed_cancellation_channel_does_not_starve_worker() {
    let (tx, rx) = watch::channel(false);
    drop(tx);
    let (result, worker) = run("write_frame(out, frame('completed', {'summary':'done', 'changed_paths':[], 'claimed_tests':[], 'subtask_requests':[]}))", limits(), rx).await;
    assert!(matches!(result, Ok(Outcome::Completed(_))));
    worker.shutdown(Duration::from_millis(200)).await;
}

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

/// A scripted worker: real handshake, then `body` runs after `start_task` arrives.
fn scripted(body: &str) -> WorkerSpec {
    let code = format!(
        "import sys, time, uuid, datetime\n\
         from harness_ai.framing import read_frame, write_frame\n\
         from harness_ai.handshake import build_ack\n\
         from harness_ai.protocol import Envelope\n\
         out = sys.stdout.buffer\n\
         write_frame(out, build_ack(read_frame(sys.stdin.buffer)))\n\
         start = read_frame(sys.stdin.buffer)\n\
         def frame(kind, payload):\n\
         \x20   return Envelope(protocol_version='1.0', kind=kind, message_id=uuid.uuid4(),\n\
         \x20       correlation_id=None, run_id='run-1', task_id='task-1', attempt=1,\n\
         \x20       timestamp=datetime.datetime.now(datetime.UTC), payload=payload)\n\
         {body}\n"
    );
    let mut spec = WorkerSpec::python_worker(python());
    spec.args = vec!["-c".into(), code.into()];
    spec
}

fn start() -> StartTask {
    StartTask {
        role: "coder".into(),
        worktree_id: "wt-1".into(),
        model: ModelRef {
            provider: "mock".into(),
            model_id: "m".into(),
        },
        context_manifest_hash: "sha256:x".into(),
        allowed_tools: vec![],
        max_tool_calls: 3,
        max_output_tokens: 100,
        timeout_seconds: 30,
        cost_cap_usd: 0.0,
    }
}

fn limits() -> Limits {
    Limits {
        idle_timeout: Duration::from_millis(800),
        deadline: Duration::from_secs(20),
        max_messages: 50,
    }
}

struct EchoBroker;

impl ToolBroker for EchoBroker {
    async fn handle(&mut self, _r: &ToolRequest) -> ToolResponse {
        ToolResponse {
            ok: true,
            result: Some(json!({"content": ""})),
            error: None,
        }
    }
}

async fn run(
    body: &str,
    limits: Limits,
    cancel: watch::Receiver<bool>,
) -> (Result<Outcome, SuperviseError>, Worker) {
    let mut w = Worker::spawn(&scripted(body)).unwrap();
    w.handshake(T).await.unwrap();
    let mut s = AssignmentSession::new("run-1", "task-1", 1, 3);
    let r = supervise(&mut w, &mut s, &start(), limits, &mut EchoBroker, cancel).await;
    (r, w)
}

#[tokio::test]
async fn hung_worker_times_out() {
    let (_tx, rx) = watch::channel(false);
    let (r, w) = run("time.sleep(60)", limits(), rx).await;
    assert!(matches!(r, Err(SuperviseError::IdleTimeout(_))), "{r:?}");
    let report = w.shutdown(T).await;
    assert!(report.killed_by_supervisor);
}

#[tokio::test]
async fn heartbeats_keep_a_slow_worker_alive() {
    let (_tx, rx) = watch::channel(false);
    let body = "for _ in range(4):\n    time.sleep(0.4)\n    write_frame(out, frame('heartbeat', {}))\n\
                write_frame(out, frame('completed', {'summary': 'ok', 'changed_paths': [], 'claimed_tests': []}))";
    let (r, w) = run(body, limits(), rx).await;
    assert!(matches!(r, Ok(Outcome::Completed(_))), "{r:?}");
    w.shutdown(T).await;
}

#[tokio::test]
async fn stdout_log_breaks_protocol() {
    let (_tx, rx) = watch::channel(false);
    let (r, w) = run(
        "print('debug: starting up', flush=True)\ntime.sleep(5)",
        limits(),
        rx,
    )
    .await;
    assert!(matches!(r, Err(SuperviseError::Protocol(_))), "{r:?}");
    assert!(w.shutdown(T).await.killed_by_supervisor);
}

#[tokio::test]
async fn message_flood_bounded() {
    let (_tx, rx) = watch::channel(false);
    let body = "for _ in range(100000):\n    write_frame(out, frame('progress', {'message': 'x'}))";
    let (r, w) = run(body, limits(), rx).await;
    assert!(matches!(r, Err(SuperviseError::MessageFlood(50))), "{r:?}");
    assert!(w.shutdown(T).await.killed_by_supervisor);
}

#[tokio::test]
async fn deadline_is_enforced_even_with_heartbeats() {
    let (_tx, rx) = watch::channel(false);
    let mut l = limits();
    l.deadline = Duration::from_millis(1200);
    l.max_messages = 10_000;
    let body = "while True:\n    time.sleep(0.2)\n    write_frame(out, frame('heartbeat', {}))";
    let (r, w) = run(body, l, rx).await;
    assert!(
        matches!(r, Err(SuperviseError::DeadlineExceeded(_))),
        "{r:?}"
    );
    w.shutdown(T).await;
}

#[tokio::test]
async fn cancellation_is_cooperative() {
    let (tx, rx) = watch::channel(false);
    let body = "m = read_frame(sys.stdin.buffer)\nsys.exit(0 if m and m.kind == 'cancel' else 3)";
    let mut l = limits();
    l.idle_timeout = Duration::from_secs(10);
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        tx.send(true).unwrap();
        tx
    });
    let (r, w) = run(body, l, rx).await;
    assert!(matches!(r, Err(SuperviseError::Cancelled)), "{r:?}");
    let report = w.shutdown(T).await;
    assert_eq!(report.code, Some(0), "{}", report.stderr);
    assert!(!report.killed_by_supervisor);
    canceller.await.unwrap();
}

#[tokio::test]
async fn real_mock_worker_still_completes_under_supervision() {
    let (_tx, rx) = watch::channel(false);
    let mut w = Worker::spawn(&WorkerSpec::python_worker(python())).unwrap();
    w.handshake(T).await.unwrap();
    let mut s = AssignmentSession::new("run-1", "task-1", 1, 3);
    let r = supervise(&mut w, &mut s, &start(), limits(), &mut EchoBroker, rx).await;
    assert!(matches!(r, Ok(Outcome::Completed(_))), "{r:?}");
    assert_eq!(w.shutdown(T).await.code, Some(0));
}

#[tokio::test]
async fn output_token_budget_stops_worker() {
    let (_tx, rx) = watch::channel(false);
    let body = "write_frame(out, frame('usage', {'provider': 'mock', 'model_id': 'm', 'input_tokens': None, 'output_tokens': 101, 'cost_usd': None, 'price_source': 'unknown'}))\ntime.sleep(60)";
    let (r, w) = run(body, limits(), rx).await;
    assert!(
        matches!(r, Err(SuperviseError::BudgetExceeded("reported usage"))),
        "{r:?}"
    );
    assert!(w.shutdown(T).await.killed_by_supervisor);
}

#[tokio::test]
async fn reported_cost_budget_stops_worker() {
    let (_tx, rx) = watch::channel(false);
    let body = "write_frame(out, frame('usage', {'provider': 'mock', 'model_id': 'm', 'input_tokens': None, 'output_tokens': None, 'cost_usd': 0.01, 'price_source': 'provider_billed'}))\ntime.sleep(60)";
    let (r, w) = run(body, limits(), rx).await;
    assert!(
        matches!(r, Err(SuperviseError::BudgetExceeded("reported usage"))),
        "{r:?}"
    );
    assert!(w.shutdown(T).await.killed_by_supervisor);
}

#[tokio::test]
async fn tool_budget_checked_before_broker_invocation() {
    struct CountingBroker(u32);
    impl ToolBroker for CountingBroker {
        async fn handle(&mut self, _: &ToolRequest) -> ToolResponse {
            self.0 += 1;
            ToolResponse {
                ok: true,
                result: Some(json!({})),
                error: None,
            }
        }
    }
    let body = "for _ in range(2):\n    write_frame(out, frame('tool_request', {'tool': 'read_file', 'arguments': {}}))\n    read_frame(sys.stdin.buffer)";
    let mut w = Worker::spawn(&scripted(body)).unwrap();
    w.handshake(T).await.unwrap();
    // The assignment cap must apply even when the session has a higher cap.
    let mut session = AssignmentSession::new("run-1", "task-1", 1, 10);
    let mut assignment = start();
    assignment.max_tool_calls = 1;
    let mut broker = CountingBroker(0);
    let (_tx, rx) = watch::channel(false);
    let result = supervise(&mut w, &mut session, &assignment, limits(), &mut broker, rx).await;
    assert!(
        matches!(result, Err(SuperviseError::BudgetExceeded("tool calls"))),
        "{result:?}"
    );
    assert_eq!(broker.0, 1);
    assert!(w.shutdown(T).await.killed_by_supervisor);
}

#[tokio::test]
async fn output_budget_accumulates_across_usage_messages() {
    let (_tx, rx) = watch::channel(false);
    let body = "for _ in range(2):\n    write_frame(out, frame('usage', {'provider': 'mock', 'model_id': 'm', 'input_tokens': None, 'output_tokens': 60, 'cost_usd': None, 'price_source': 'unknown'}))\ntime.sleep(60)";
    let (r, w) = run(body, limits(), rx).await;
    assert!(
        matches!(r, Err(SuperviseError::BudgetExceeded("reported usage"))),
        "{r:?}"
    );
    assert!(w.shutdown(T).await.killed_by_supervisor);
}
