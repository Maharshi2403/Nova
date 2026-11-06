use std::path::PathBuf;
use std::time::Duration;

use harness_runtime::worker::{Worker, WorkerError, WorkerSpec};

const T: Duration = Duration::from_secs(30);

/// The project venv's interpreter, or `HARNESS_PYTHON` when set (CI runs `uv sync` first).
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
        "python venv not found at {}; run `uv sync` in intelligence/ or set HARNESS_PYTHON",
        exe.display()
    );
    exe
}

fn inline(code: &str, extra: &[&str]) -> WorkerSpec {
    let mut spec = WorkerSpec::python_worker(python());
    spec.args = ["-c", code].iter().chain(extra).map(Into::into).collect();
    spec
}

#[tokio::test]
async fn python_worker_handshake() {
    let mut w = Worker::spawn(&WorkerSpec::python_worker(python())).unwrap();
    let accepted = w.handshake(T).await.unwrap();
    assert_eq!(accepted.worker_version, "0.1.0");
    let report = w.shutdown(T).await;
    assert_eq!(report.code, Some(0), "{}", report.stderr);
    assert!(!report.killed_by_supervisor);
}

#[tokio::test]
async fn child_crash_is_reported() {
    let mut w = Worker::spawn(&inline(
        "import sys; sys.stderr.write('boom'); sys.exit(3)",
        &[],
    ))
    .unwrap();
    match w.handshake(T).await.unwrap_err() {
        WorkerError::Exited(r) => {
            assert_eq!(r.code, Some(3));
            assert!(r.stderr.contains("boom"), "{r:?}");
            assert!(!r.killed_by_supervisor);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn hung_worker_handshake_times_out_and_is_killed() {
    let mut w = Worker::spawn(&inline("import time; time.sleep(60)", &[])).unwrap();
    let err = w.handshake(Duration::from_millis(300)).await.unwrap_err();
    assert!(matches!(err, WorkerError::Timeout(_)), "{err:?}");
    let report = w.shutdown(Duration::from_secs(1)).await;
    assert!(report.killed_by_supervisor);
}

#[tokio::test]
async fn arguments_are_not_shell_expanded() {
    let nasty = "a && echo pwned; $HOME %PATH% `x`";
    let mut w = Worker::spawn(&inline(
        "import sys; sys.stderr.write(sys.argv[1]); sys.exit(0)",
        &[nasty],
    ))
    .unwrap();
    let _ = w.handshake(T).await;
    let report = w.shutdown(T).await;
    assert_eq!(report.stderr, nasty);
}

#[tokio::test]
async fn environment_is_sanitized() {
    // SAFETY-free: only sets a variable in this test process before spawning.
    unsafe { std::env::set_var("HARNESS_TEST_SECRET", "s3cret") };
    let mut w = Worker::spawn(&inline(
        "import os, sys; sys.stderr.write(repr(os.environ.get('HARNESS_TEST_SECRET')))",
        &[],
    ))
    .unwrap();
    let _ = w.handshake(T).await;
    assert_eq!(w.shutdown(T).await.stderr, "None");
}

#[tokio::test]
async fn adapter_env_contains_only_scoped_key() {
    use harness_runtime::provider_config::{CredentialRef, ProviderKind, ProviderProfile};
    let profile = ProviderProfile {
        id: "selected".into(),
        kind: ProviderKind::Compatible,
        base_url: "http://127.0.0.1:8080/v1".into(),
        credential: CredentialRef::Environment {
            variable: "SELECTED_KEY".into(),
        },
    };
    let mut spec = WorkerSpec::python_worker(python());
    spec.args = [
        "-c",
        "import os,sys; sys.stderr.write(','.join(sorted(k for k in os.environ if any(s in k.upper() for s in ('KEY','TOKEN','SECRET')))))",
    ]
    .map(Into::into)
    .to_vec();
    spec.env = profile.adapter_env("fixture-only");
    let worker = Worker::spawn(&spec).unwrap();
    let report = worker.shutdown(T).await;
    assert_eq!(report.code, Some(0), "{report:?}");
    assert_eq!(report.stderr, "NOVA_PROVIDER_API_KEY");
}

#[tokio::test]
async fn missing_executable_is_a_spawn_error() {
    let spec = WorkerSpec::python_worker("definitely-not-a-real-binary-xyz");
    assert!(matches!(
        Worker::spawn(&spec).err().unwrap(),
        WorkerError::Spawn(_)
    ));
}

#[tokio::test]
async fn stderr_capture_is_bounded() {
    let mut w = Worker::spawn(&inline(
        "import sys; sys.stderr.write('x' * 300000 + 'END')",
        &[],
    ))
    .unwrap();
    let _ = w.handshake(T).await;
    let report = w.shutdown(T).await;
    assert!(report.stderr.len() <= harness_runtime::worker::STDERR_CAPTURE_BYTES);
    assert!(report.stderr.ends_with("END"));
}
