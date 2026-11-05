use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use harness_core::domain::{Project, ProjectId, Run, RunId};
use harness_core::plan::PlanProposal;
use harness_daemon::api_router;
use harness_runtime::git::git;
use harness_store::Store;
use harness_store::plans::PlanRepository;
use harness_store::repository::Repository;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

const TOKEN: &str = "test-token";

#[tokio::test]
async fn shutdown_requires_authentication() {
    let (_data, store, _repo) = fixture().await;
    let response = api_router(TOKEN.into(), store)
        .oneshot(Request::post("/v1/shutdown").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn subscriber_resumes_after_disconnect_without_duplicates() {
    let (_data, store, _repo) = fixture().await;
    let p = Project::new(ProjectId::new("p").unwrap(), "fixture", "/repo").unwrap();
    store.create_project(&p).await.unwrap();
    let run = Run::new(RunId::new("r").unwrap(), p.id);
    store.create_run(&run).await.unwrap();
    let cursor = store
        .list_events(&run.id, 0, 100)
        .await
        .unwrap()
        .last()
        .map_or(0, |e| e.sequence);
    let write = store.clone();
    let producer = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        write
            .transition_run(&run.id, 0, harness_core::domain::RunState::Cancelled, None)
            .await
            .unwrap();
    });
    let app = api_router(TOKEN.into(), store);
    let response = app
        .clone()
        .oneshot(
            auth(Request::get(format!(
                "/v1/events?run_id=r&after={cursor}&wait_seconds=1"
            )))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let events: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 10000).await.unwrap()).unwrap();
    assert_eq!(events.as_array().unwrap().len(), 1);
    let next = events[0]["sequence"].as_i64().unwrap();
    assert!(next > cursor);
    let response = app
        .oneshot(
            auth(Request::get(format!("/v1/events?run_id=r&after={next}")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        to_bytes(response.into_body(), 10000)
            .await
            .unwrap()
            .as_ref(),
        b"[]"
    );
    producer.await.unwrap();
}

#[tokio::test]
async fn event_long_poll_rejects_unbounded_wait() {
    let (_data, store, _repo) = fixture().await;
    let p = Project::new(ProjectId::new("p").unwrap(), "fixture", "/repo").unwrap();
    store.create_project(&p).await.unwrap();
    store
        .create_run(&Run::new(RunId::new("r").unwrap(), p.id))
        .await
        .unwrap();
    let response = api_router(TOKEN.into(), store)
        .oneshot(
            auth(Request::get("/v1/events?run_id=r&wait_seconds=26"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

fn auth(request: axum::http::request::Builder) -> axum::http::request::Builder {
    request.header(header::AUTHORIZATION, ["Bearer", TOKEN].join(" "))
}

async fn fixture() -> (TempDir, Store, TempDir) {
    let data = tempfile::tempdir().unwrap();
    let repository = tempfile::tempdir().unwrap();
    git(repository.path(), &["init", "-q", "-b", "main"])
        .await
        .unwrap();
    std::fs::write(repository.path().join("README.md"), "demo\n").unwrap();
    git(repository.path(), &["add", "README.md"]).await.unwrap();
    git(
        repository.path(),
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "init",
        ],
    )
    .await
    .unwrap();
    let store = Store::open(data.path().join("harness.db")).await.unwrap();
    (data, store, repository)
}

fn valid_plan(project_id: &str) -> Value {
    json!({
        "schema_version": "1.0",
        "project_id": project_id,
        "idea": "Build a minimal API response.",
        "assumptions": [],
        "requirements": ["It returns a JSON response."],
        "architecture": {
            "recommended": "Implement a small isolated response module.",
            "alternatives": [],
            "risks": []
        },
        "tasks": [{
            "id": "task-1",
            "title": "Implement response",
            "description": "Implement the response module.",
            "complexity": "low",
            "risk": "low",
            "agent_priority": 50,
            "manual_priority": null,
            "dependencies": [],
            "candidate_paths": ["src/**"],
            "acceptance": [{"kind": "test", "command": ["git", "--version"]}],
            "limits": {
                "max_tool_calls": 1,
                "max_output_tokens": 1,
                "timeout_seconds": 30,
                "cost_cap_usd": 0.0
            }
        }]
    })
}

#[tokio::test]
async fn create_project_api() {
    let (_data, store, repository) = fixture().await;
    let app = api_router(TOKEN.into(), store);
    let body = json!({
        "name": "Demo",
        "repo_path": repository.path(),
    });
    let response = app
        .oneshot(
            auth(Request::post("/v1/projects"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let response: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(response["name"], "Demo");
    assert!(response["id"].as_str().is_some_and(|id| !id.is_empty()));
}

#[tokio::test]
async fn invalid_plan_422() {
    let (_data, store, repository) = fixture().await;
    let project_id = ProjectId::new("project-1").unwrap();
    let run_id = RunId::new("run-1").unwrap();
    store
        .create_project(
            &Project::new(
                project_id.clone(),
                "Demo",
                repository.path().display().to_string(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    store
        .create_run(&Run::new(run_id.clone(), project_id))
        .await
        .unwrap();
    let body = json!({
        "run_id": run_id.as_str(),
        "plan": {
            "schema_version": "1.0",
            "project_id": "project-1",
            "idea": "bad",
            "assumptions": [],
            "requirements": ["required"],
            "architecture": {"recommended": "Long enough architecture.", "alternatives": [], "risks": []},
            "tasks": []
        }
    });
    let response = api_router(TOKEN.into(), store)
        .oneshot(
            auth(Request::post("/v1/plans"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn plan_approval_required() {
    let (_data, store, repository) = fixture().await;
    let project_id = ProjectId::new("project-1").unwrap();
    let run_id = RunId::new("run-1").unwrap();
    store
        .create_project(
            &Project::new(
                project_id.clone(),
                "Demo",
                repository.path().display().to_string(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    store
        .create_run(&Run::new(run_id.clone(), project_id))
        .await
        .unwrap();
    let response = api_router(TOKEN.into(), store.clone())
        .oneshot(
            auth(Request::post("/v1/plans"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"run_id": run_id.as_str(), "plan": valid_plan("project-1")}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), 16_384).await.unwrap();
    let plan: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(plan["status"], "proposed");
    assert_eq!(
        store.get_run(&run_id).await.unwrap().state,
        harness_core::domain::RunState::AwaitingApproval
    );
}

async fn running_fixture() -> (TempDir, Store, TempDir, RunId) {
    let (data, store, repository) = fixture().await;
    let project_id = ProjectId::new("project-1").unwrap();
    let run_id = RunId::new("run-1").unwrap();
    store
        .create_project(
            &Project::new(
                project_id.clone(),
                "Demo",
                repository.path().display().to_string(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    store
        .create_run(&Run::new(run_id.clone(), project_id))
        .await
        .unwrap();
    store
        .propose_plan(
            &run_id,
            &serde_json::from_value::<PlanProposal>(valid_plan("project-1")).unwrap(),
        )
        .await
        .unwrap();
    store.approve_plan(&run_id, 1).await.unwrap();
    (data, store, repository, run_id)
}

#[tokio::test]
async fn repeated_control_cycles_and_concurrent_ticks_remain_valid() {
    let (_data, store, _repo, run) = running_fixture().await;
    let app = api_router(TOKEN.into(), store.clone());
    for _ in 0..3 {
        for action in ["pause", "resume"] {
            let response = app
                .clone()
                .oneshot(
                    auth(Request::post(format!("/v1/runs/{run}/{action}")))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
    }
    let (a, b) = tokio::join!(
        harness_runtime::scheduler::tick(&store, &run, 0),
        harness_runtime::scheduler::tick(&store, &run, 0)
    );
    assert!(a.is_ok() && b.is_ok());
    assert_eq!(
        store.list_tasks(&run).await.unwrap()[0].state,
        harness_core::domain::TaskState::Ready
    );
}

#[tokio::test]
async fn pause_prevents_new_dispatch() {
    let (_data, store, _repository, run_id) = running_fixture().await;
    let app = api_router(TOKEN.into(), store.clone());
    let pause = app
        .clone()
        .oneshot(
            auth(Request::post(format!("/v1/runs/{run_id}/pause")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(pause.status(), StatusCode::OK);
    let tasks = app
        .oneshot(
            auth(Request::get(format!("/v1/runs/{run_id}/tasks")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(tasks.into_body(), 16_384).await.unwrap();
    let tasks: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(tasks[0]["state"], "pending");
    assert_eq!(
        store.get_run(&run_id).await.unwrap().state,
        harness_core::domain::RunState::Paused
    );
}

#[tokio::test]
async fn resume_releases_ready() {
    let (_data, store, _repository, run_id) = running_fixture().await;
    let app = api_router(TOKEN.into(), store.clone());
    app.clone()
        .oneshot(
            auth(Request::post(format!("/v1/runs/{run_id}/pause")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let response = app
        .oneshot(
            auth(Request::post(format!("/v1/runs/{run_id}/resume")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        store
            .list_tasks(&run_id)
            .await
            .unwrap()
            .first()
            .unwrap()
            .state,
        harness_core::domain::TaskState::Ready
    );
}

#[tokio::test]
async fn status_matches_db() {
    let (_data, store, _repository, run_id) = running_fixture().await;
    let response = api_router(TOKEN.into(), store.clone())
        .oneshot(
            auth(Request::get(format!("/v1/runs/{run_id}")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let response: Value = serde_json::from_slice(&body).unwrap();
    let run = store.get_run(&run_id).await.unwrap();
    assert_eq!(response["id"], run.id.as_str());
    assert_eq!(response["state"], "running");
    assert_eq!(response["revision"], run.revision);
}

#[tokio::test]
async fn stale_priority_http_409() {
    let (_data, store, _repository, run_id) = running_fixture().await;
    let body = json!({"revision": 0, "manual_priority": 90}).to_string();
    let response = api_router(TOKEN.into(), store)
        .oneshot(
            auth(Request::patch(format!(
                "/v1/tasks/{run_id}/task-1/priority"
            )))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn unauthorized_approval_denied() {
    let (_data, store, _repository) = fixture().await;
    let response = api_router(TOKEN.into(), store)
        .oneshot(
            Request::post("/v1/approvals/not-real/decision")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn events_cursor_replay() {
    let (_data, store, _repository, run_id) = running_fixture().await;
    let app = api_router(TOKEN.into(), store);
    let first = app
        .clone()
        .oneshot(
            auth(Request::get(format!(
                "/v1/events?run_id={run_id}&after=0&limit=1"
            )))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let body = to_bytes(first.into_body(), 4096).await.unwrap();
    let first: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(first.as_array().unwrap().len(), 1);
    let sequence = first[0]["sequence"].as_i64().unwrap();
    let next = app
        .oneshot(
            auth(Request::get(format!(
                "/v1/events?run_id={run_id}&after={sequence}"
            )))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(next.into_body(), 4096).await.unwrap();
    let next: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        next.as_array()
            .unwrap()
            .iter()
            .all(|event| event["sequence"].as_i64().unwrap() > sequence)
    );
}

#[tokio::test]
async fn dashboard_inventory_is_authenticated_bounded_and_durable() {
    let (_data, store, _repo) = fixture().await;
    for n in 0..3 {
        let p = Project::new(
            ProjectId::new(format!("p{n}")).unwrap(),
            format!("Project {n}"),
            "/repo",
        )
        .unwrap();
        store.create_project(&p).await.unwrap();
        store
            .create_run(&Run::new(RunId::new(format!("r{n}")).unwrap(), p.id))
            .await
            .unwrap();
    }
    let app = api_router(TOKEN.into(), store);
    let response = app
        .clone()
        .oneshot(Request::get("/v1/dashboard").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(
            auth(Request::get("/v1/dashboard?offset=1&limit=1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let data: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 10000).await.unwrap()).unwrap();
    assert_eq!(data["project_count"], 3);
    assert_eq!(data["run_count"], 3);
    assert_eq!(data["projects"][0]["id"], "p1");
    assert_eq!(data["runs"].as_array().unwrap().len(), 1);
    assert_eq!(data["projects"][0]["trusted_local"], false);
    let response = app
        .clone()
        .oneshot(
            auth(Request::get("/v1/dashboard?limit=201"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let response = app
        .clone()
        .oneshot(
            auth(Request::get("/v1/dashboard"))
                .header("origin", "http://localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app
        .oneshot(
            auth(Request::get("/v1/tasks/r1/missing/evidence"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn trusted_project_import_preserves_dirty_files_but_execution_gate_stays_clean() {
    let (_data, store, repo) = fixture().await;
    std::fs::write(repo.path().join("unfinished.txt"), "user work\n").unwrap();
    let before = git(repo.path(), &["status", "--porcelain=v1"])
        .await
        .unwrap();
    let response = api_router(TOKEN.into(), store)
        .oneshot(
            auth(Request::post("/v1/projects"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"name":"Work in progress","repo_path":repo.path(),"trusted_local":true})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        std::fs::read_to_string(repo.path().join("unfinished.txt")).unwrap(),
        "user work\n"
    );
    assert_eq!(
        git(repo.path(), &["status", "--porcelain=v1"])
            .await
            .unwrap(),
        before
    );
    assert!(matches!(
        harness_runtime::git::register_repo(repo.path().to_str().unwrap()).await,
        Err(harness_runtime::git::RepoError::Dirty { .. })
    ));
}

#[tokio::test]
async fn project_trust_is_explicit_and_removal_never_deletes_files_or_runs() {
    let (_data, store, repo) = fixture().await;
    let project = Project::new(
        ProjectId::new("trust-project").unwrap(),
        "Trust fixture",
        repo.path().to_str().unwrap(),
    )
    .unwrap();
    store.create_project(&project).await.unwrap();
    let app = api_router(TOKEN.into(), store.clone());
    let path = "/v1/projects/trust-project/trust";
    let response = app
        .clone()
        .oneshot(
            Request::post(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"trusted_local":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(!store.project_is_trusted(&project.id).await.unwrap());
    std::fs::write(repo.path().join("unfinished.txt"), "keep").unwrap();
    let response = app
        .clone()
        .oneshot(
            auth(Request::post(path))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"trusted_local":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(store.project_is_trusted(&project.id).await.unwrap());
    let run = Run::new(RunId::new("keep-run").unwrap(), project.id.clone());
    store.create_run(&run).await.unwrap();
    let response = app
        .clone()
        .oneshot(
            auth(Request::delete("/v1/projects/trust-project"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(store.get_run(&run.id).await.is_ok());
    assert!(store.project_is_trusted(&project.id).await.unwrap());
    let empty = Project::new(
        ProjectId::new("empty").unwrap(),
        "Empty",
        repo.path().to_str().unwrap(),
    )
    .unwrap();
    store.create_project(&empty).await.unwrap();
    store.set_project_trust(&empty.id, true).await.unwrap();
    let response = app
        .oneshot(
            auth(Request::delete("/v1/projects/empty"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(store.get_project(&empty.id).await.is_err());
    assert_eq!(
        std::fs::read_to_string(repo.path().join("unfinished.txt")).unwrap(),
        "keep"
    );
}
