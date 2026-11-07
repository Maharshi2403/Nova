//! Live coder path end to end against a scripted, loopback OpenAI-compatible server.
//! No network, no paid calls: the "model" is a fixed script of HTTP replies.
use harness_core::domain::*;
use harness_runtime::{
    engine::{Engine, LaunchMode, default_python},
    git::git,
};
use harness_store::{
    Store, git_artifacts::GitArtifactRepository, plans::PlanRepository, repository::Repository,
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

/// One scripted HTTP reply: status and JSON body.
type Reply = (u16, Value);

fn action(value: Value) -> Reply {
    (
        200,
        json!({
            "choices":[{"finish_reason":"stop","message":{"role":"assistant","content":value.to_string()}}],
            "usage":{"prompt_tokens":100,"completion_tokens":50}
        }),
    )
}
fn patch(path: &str, content: &str) -> Reply {
    action(json!({"action":"tool","tool":"repo.patch",
        "arguments_json":json!({"path":path,"content":content}).to_string(),"summary":"write file"}))
}
fn done() -> Reply {
    action(json!({"action":"completed","tool":null,"arguments_json":null,"summary":"done"}))
}

/// Serves replies in order (one per request) and counts requests.
async fn fake_provider(script: Vec<Reply>) -> (u16, Arc<Mutex<usize>>) {
    let (port, count, _) = fake_provider_captured(script).await;
    (port, count)
}

type Requests = Arc<Mutex<Vec<Value>>>;
async fn fake_provider_captured(script: Vec<Reply>) -> (u16, Arc<Mutex<usize>>, Requests) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let count = Arc::new(Mutex::new(0usize));
    let seen = count.clone();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                let n = socket.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf);
                if let Some(split) = text.find("\r\n\r\n") {
                    let length = text[..split]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if buf.len() >= split + 4 + length {
                        break;
                    }
                }
            }
            let split = buf
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
                .unwrap();
            let request: Value = serde_json::from_slice(&buf[split + 4..]).unwrap();
            {
                let mut requests = captured.lock().unwrap();
                if requests.len() < 48 {
                    requests.push(request);
                }
            }
            let index = {
                let mut n = seen.lock().unwrap();
                *n += 1;
                *n - 1
            };
            let (status, body) = script.get(index).cloned().unwrap_or_else(|| {
                action(json!({"action":"failed","tool":null,"arguments_json":null,"summary":"script exhausted"}))
            });
            let body = body.to_string();
            let reply = format!(
                "HTTP/1.1 {status} Scripted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
            socket.shutdown().await.ok();
        }
    });
    (port, count, requests)
}

struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    run: RunId,
    repo: PathBuf,
}

fn python() -> PathBuf {
    std::path::absolute(default_python(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../intelligence/.venv"),
    ))
    .unwrap()
}

/// Disposable repo + store + approved plan where task `i` must create `files[i]` and pass
/// `checks[i]`; each task depends on the previous one.
async fn launch(port: u16, key_var: &str, files: &[&str], checks: &[&str]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.name", "Harness Test"],
        &["config", "user.email", "test@example.test"],
    ] {
        git(&repo, args).await.unwrap();
    }
    std::fs::write(repo.join("README.md"), "fixture\n").unwrap();
    git(&repo, &["add", "README.md"]).await.unwrap();
    git(&repo, &["commit", "-qm", "init"]).await.unwrap();

    // SAFETY: test-only environment variable, unique per test.
    unsafe { std::env::set_var(key_var, "fixture-key") };
    std::fs::write(
        dir.path().join("providers.json"),
        json!({
            "version":1,
            "providers":[{"id":"fake","kind":"compatible","base_url":format!("http://127.0.0.1:{port}/v1"),
                "credential":{"source":"environment","variable":key_var}}],
            "models":[],"routes":{},"python":null,
            "run_cap_microdollars":5000000,"daily_cap_microdollars":10000000
        })
        .to_string(),
    )
    .unwrap();
    let live: harness_runtime::live::LiveConfig = serde_json::from_value(json!({
        "providers":{"fake":"compatible"},
        "registry":{"models":[{"id":"fake/coder","provider":"fake","model_id":"coder",
            "supported_tools":["repo.patch","repo.read"],"structured_output":true,"context_window":64000,
            "max_output_tokens":4000,"input_usd_per_million_tokens":1,"output_usd_per_million_tokens":2,
            "available":true,"privacy":"private"}]},
        "routing":{"low_model":"fake/coder","medium_model":"fake/coder","high_model":"fake/coder"},
        "run_cap_microdollars":5000000,"daily_cap_microdollars":10000000
    }))
    .unwrap();

    let store = Store::open(dir.path().join("db")).await.unwrap();
    let p = Project::new(
        ProjectId::new("proj-demo").unwrap(),
        "fixture",
        repo.to_str().unwrap(),
    )
    .unwrap();
    store.create_project(&p).await.unwrap();
    store.set_project_trust(&p.id, true).await.unwrap();
    let run = Run::new(RunId::new("r").unwrap(), p.id.clone());
    store.create_run(&run).await.unwrap();
    let commands: Vec<Vec<String>> = checks
        .iter()
        .map(|c| vec![python().to_str().unwrap().into(), "-c".into(), (*c).into()])
        .collect();
    let mut plan: harness_core::plan::PlanProposal = serde_json::from_str(include_str!(
        "../../../tests/fixtures/plans/valid_plan.json"
    ))
    .unwrap();
    let template = plan.tasks.remove(0);
    plan.tasks = files
        .iter()
        .enumerate()
        .map(|(i, file)| {
            let mut task = template.clone();
            task.id = TaskId::new(format!("task-{i}")).unwrap();
            task.dependencies = if i == 0 {
                vec![]
            } else {
                vec![TaskId::new(format!("task-{}", i - 1)).unwrap()]
            };
            task.candidate_paths = vec![(*file).into()];
            task.acceptance = vec![Acceptance::Test {
                command: commands[i].clone(),
            }];
            task
        })
        .collect();
    store.propose_plan(&run.id, &plan).await.unwrap();
    store.approve_plan(&run.id, 1).await.unwrap();
    Engine::new(dir.path().join("execution"))
        .launch_configured(
            store.clone(),
            run.id.clone(),
            python(),
            commands,
            1,
            LaunchMode {
                live: Some(live),
                committed_snapshot: false,
            },
        )
        .await
        .unwrap();
    Fixture {
        _dir: dir,
        store,
        run: run.id,
        repo,
    }
}

/// Wait until the run completes (`Ok`) or any task blocks (`Err(task id)`).
async fn settle(f: &Fixture) -> Result<(), TaskId> {
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let tasks = f.store.list_tasks(&f.run).await.unwrap();
            if let Some(t) = tasks.iter().find(|t| t.state == TaskState::Blocked) {
                return Err(t.id().clone());
            }
            if f.store.get_run(&f.run).await.unwrap().state == RunState::Completed {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("run neither completed nor blocked")
}

async fn failure(f: &Fixture, task: &TaskId) -> String {
    f.store
        .latest_task_failure(&f.run, task)
        .await
        .unwrap()
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn live_coder_completes_dependent_python_tasks_despite_check_artifacts() {
    let (port, requests) = fake_provider(vec![
        action(
            json!({"action":"tool","tool":"repo.read","arguments_json":"{\"path\":\"README.md\"}","summary":"inspect"}),
        ),
        patch("greeting.py", "def greet(name):\n    return f'Hello, {name}!'\n"),
        done(),
        patch(
            "shout.py",
            "from greeting import greet\n\n\ndef shout(name):\n    return greet(name).upper()\n",
        ),
        done(),
    ])
    .await;
    // Importing modules makes Python write __pycache__ in task and integration worktrees.
    let f = launch(
        port,
        "NOVA_FAKE_E2E_KEY_OK",
        &["greeting.py", "shout.py"],
        &[
            "from greeting import greet; assert greet('Ada') == 'Hello, Ada!'",
            "from shout import shout; assert shout('Ada') == 'HELLO, ADA!'",
        ],
    )
    .await;
    if let Err(task) = settle(&f).await {
        panic!("{task} blocked: {}", failure(&f, &task).await);
    }
    assert_eq!(*requests.lock().unwrap(), 5);
    for id in ["task-0", "task-1"] {
        let commit = f
            .store
            .get_task_commit(&f.run, &TaskId::new(id).unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(commit.integration_commit_sha.is_some());
        assert!(commit.diff_paths.iter().all(|p| !p.contains("__pycache__")));
    }
    assert!(
        !f.repo.join("greeting.py").exists(),
        "source checkout untouched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_rejection_reason_is_recorded_without_provider_message() {
    let quota = (
        429,
        json!({"error":{"code":"insufficient_quota","type":"insufficient_quota",
            "message":"PRIVATE-ACCOUNT-DETAIL you exceeded your quota"}}),
    );
    let (port, requests) = fake_provider(vec![quota.clone(), quota.clone(), quota]).await;
    let f = launch(port, "NOVA_FAKE_E2E_KEY_QUOTA", &["a.py"], &["import a"]).await;
    let task = settle(&f).await.expect_err("quota failure must block");
    let reason = failure(&f, &task).await;
    assert_eq!(
        *requests.lock().unwrap(),
        3,
        "transient errors retry 3 times"
    );
    assert!(
        reason.contains("provider HTTP 429 insufficient_quota"),
        "reason: {reason}"
    );
    assert!(
        !reason.contains("PRIVATE-ACCOUNT-DETAIL"),
        "reason: {reason}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_model_output_is_reported_by_kind() {
    let (port, _) = fake_provider(vec![action(json!({"unexpected":"shape"}))]).await;
    let f = launch(port, "NOVA_FAKE_E2E_KEY_SHAPE", &["a.py"], &["import a"]).await;
    let task = settle(&f).await.expect_err("bad output must block");
    let reason = failure(&f, &task).await;
    assert!(
        reason.contains("provider returned invalid or incomplete structured output"),
        "reason: {reason}"
    );
}

/// The whole loop the user sees: one goal -> main agent decomposes it -> human approves
/// -> subagents (one per subtask, routed by complexity) implement, verify and integrate.
#[tokio::test(flavor = "multi_thread")]
async fn user_goal_is_decomposed_by_main_agent_and_completed_by_subagents() {
    let runner = harness_runtime::orchestrator::test_runners()
        .into_iter()
        .find(|r| r.ends_with("/python3"))
        .expect("python3 test runner");
    let decomposition = json!({
        "summary": "Greeting module first, then an uppercase variant that reuses it.",
        "assumptions": [], "risks": [],
        "tasks": [
            {"key":"greeting","title":"Greeting function","use_case":"coding","complexity":"low","risk":"low",
             "instructions":"Create greeting.py with greet(name) returning 'Hello, {name}!'.",
             "depends_on":[],"files":["greeting.py"],
             "test_command":[runner,"-c","from greeting import greet; assert greet('Ada') == 'Hello, Ada!'"]},
            {"key":"shout","title":"Shout function","use_case":"coding","complexity":"medium","risk":"low",
             "instructions":"Create shout.py with shout(name) returning greet(name).upper().",
             "depends_on":["greeting"],"files":["shout.py"],
             "test_command":[runner,"-c","from shout import shout; assert shout('Ada') == 'HELLO, ADA!'"]}
        ]
    });
    let (port, requests) = fake_provider(vec![
        action(decomposition),
        patch(
            "greeting.py",
            "def greet(name):\n    return f'Hello, {name}!'\n",
        ),
        done(),
        patch(
            "shout.py",
            "from greeting import greet\n\n\ndef shout(name):\n    return greet(name).upper()\n",
        ),
        done(),
    ])
    .await;

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.name", "Harness Test"],
        &["config", "user.email", "test@example.test"],
    ] {
        git(&repo, args).await.unwrap();
    }
    std::fs::write(repo.join("README.md"), "greeting demo\n").unwrap();
    git(&repo, &["add", "README.md"]).await.unwrap();
    git(&repo, &["commit", "-qm", "init"]).await.unwrap();
    // SAFETY: test-only environment variable, unique per test.
    unsafe { std::env::set_var("NOVA_FAKE_E2E_KEY_GOAL", "fixture-key") };
    let saved = |id: &str, tier: &str, price: f64| {
        json!({"id":format!("fake/{id}"),"provider":"fake","model_id":id,"display_name":id,
            "context_window":64000,"max_output_tokens":4000,"input_usd_per_million_tokens":price,
            "output_usd_per_million_tokens":price * 2.0,"structured_output":true,"tier":tier,
            "metadata_source":"test"})
    };
    std::fs::write(
        dir.path().join("providers.json"),
        json!({
            "version":1,
            "providers":[{"id":"fake","kind":"compatible","base_url":format!("http://127.0.0.1:{port}/v1"),
                "credential":{"source":"environment","variable":"NOVA_FAKE_E2E_KEY_GOAL"}}],
            "models":[saved("small","low",0.1), saved("mid","medium",1.0), saved("large","high",5.0)],
            "routes":{},"python":python(),
            "run_cap_microdollars":5000000,"daily_cap_microdollars":10000000
        })
        .to_string(),
    )
    .unwrap();

    let store = Store::open(dir.path().join("db")).await.unwrap();
    let project = Project::new(
        ProjectId::new("proj-demo").unwrap(),
        "fixture",
        repo.to_str().unwrap(),
    )
    .unwrap();
    store.create_project(&project).await.unwrap();
    store.set_project_trust(&project.id, true).await.unwrap();
    let run = Run::new(RunId::new("goal-run").unwrap(), project.id.clone());
    store.create_run(&run).await.unwrap();

    let planned = harness_runtime::orchestrator::plan_goal(
        dir.path(),
        &project,
        "Add greeting and shout helpers",
        run.id.as_str(),
    )
    .await
    .unwrap();
    assert_eq!(planned.model, "fake/large", "planning uses the high tier");
    assert_eq!(planned.plan.tasks.len(), 2);
    let settings = harness_runtime::provider_config::ProviderStore::new(dir.path())
        .load()
        .unwrap();
    let live = settings.live_config().unwrap();
    let picks: Vec<_> = store_tasks_models(&live, &planned.plan);
    assert_eq!(
        picks,
        vec!["fake/small", "fake/mid"],
        "subagents routed by complexity"
    );

    store.propose_plan(&run.id, &planned.plan).await.unwrap();
    store.approve_plan(&run.id, 1).await.unwrap();
    let commands: Vec<Vec<String>> = planned
        .plan
        .tasks
        .iter()
        .flat_map(|t| t.acceptance.iter())
        .filter_map(|a| match a {
            Acceptance::Test { command } => Some(command.clone()),
            _ => None,
        })
        .collect();
    Engine::new(dir.path().join("execution"))
        .launch_configured(
            store.clone(),
            run.id.clone(),
            python(),
            commands,
            2,
            LaunchMode {
                live: Some(live),
                committed_snapshot: false,
            },
        )
        .await
        .unwrap();
    let f = Fixture {
        _dir: dir,
        store,
        run: run.id,
        repo,
    };
    if let Err(task) = settle(&f).await {
        panic!("{task} blocked: {}", failure(&f, &task).await);
    }
    assert_eq!(
        *requests.lock().unwrap(),
        5,
        "1 planning + 2x2 subagent calls"
    );
    let usage = f.store.list_usage(&f.run, None).await.unwrap();
    let models: std::collections::BTreeSet<_> =
        usage.iter().map(|u| u.usage.model_id.clone()).collect();
    assert_eq!(
        models,
        ["mid".to_string(), "small".to_string()]
            .into_iter()
            .collect()
    );
}

fn store_tasks_models(
    live: &harness_runtime::live::LiveConfig,
    plan: &harness_core::plan::PlanProposal,
) -> Vec<String> {
    plan.tasks
        .iter()
        .map(|spec| {
            let task = Task::new(RunId::new("x").unwrap(), spec.clone()).unwrap();
            live.select(&task).unwrap().id
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn engine_seeds_exact_acceptance_commands_for_live_coder() {
    let check = "from pathlib import Path; assert Path('out.txt').read_text() == 'ready\\n'; print('checked')";
    let command = vec![
        python().to_str().unwrap().to_owned(),
        "-c".to_owned(),
        check.to_owned(),
    ];
    let (port, _, captured) = fake_provider_captured(vec![
        patch("out.txt", "ready\n"),
        action(json!({"action":"tool", "tool":"cmd.run", "arguments_json":json!({"argv":command}).to_string(), "summary":"run approved check"})),
        done(),
    ]).await;
    let fixture = launch(port, "H089_ENGINE_FIXTURE_KEY", &["out.txt"], &[check]).await;
    settle(&fixture).await.unwrap();
    assert!(
        fixture
            .store
            .list_approvals(&fixture.run)
            .await
            .unwrap()
            .is_empty()
    );
    let requests = captured.lock().unwrap();
    let prompt: Value =
        serde_json::from_str(requests[2]["messages"][1]["content"].as_str().unwrap()).unwrap();
    let memory: Value = serde_json::from_str(prompt["previous_summary"].as_str().unwrap()).unwrap();
    let record = memory["recent"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["tool"] == "cmd.run")
        .unwrap();
    let response: Value = serde_json::from_str(record["result_excerpt"].as_str().unwrap()).unwrap();
    assert_eq!(response["ok"], true);
    assert_eq!(response["result"]["exit_code"], 0);
    assert_eq!(response["result"]["stdout"], "checked\n");
}
