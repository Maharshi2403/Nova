use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use harness_core::domain::{Project, ProjectId, Run, RunId};
use harness_daemon::api_router;
use harness_runtime::git::git;
use harness_store::Store;
use harness_store::repository::Repository;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

const TOKEN: &str = "test-token";

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
