//! Authenticated loopback control server.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{Path as RoutePath, Query, State};
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use harness_core::domain::{Priority, Project, ProjectId, Run, RunId, RunState, Task};
use harness_core::plan::PlanProposal;
use harness_runtime::git::inspect_repo;
use harness_runtime::scheduler::tick;
use harness_store::Store;
use harness_store::plans::{PlanRecord, PlanRepository, PlanStatus};
use harness_store::repository::{PriorityUpdate, Repository};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const PORT_FILE: &str = "daemon.json";
const TOKEN_FILE: &str = "daemon.token";

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("daemon may only bind a loopback address: {0}")]
    NonLoopback(SocketAddr),
    #[error("could not access daemon data directory: {0}")]
    DataDirectory(#[source] std::io::Error),
    #[error("could not access daemon token: {0}")]
    TokenFile(#[source] std::io::Error),
    #[error("could not write daemon discovery metadata: {0}")]
    DiscoveryFile(#[source] std::io::Error),
    #[error("could not bind daemon listener: {0}")]
    Bind(#[source] std::io::Error),
    #[error("could not open daemon state database: {0}")]
    Store(#[source] harness_store::StoreError),
}

#[derive(Debug)]
pub struct Daemon {
    pub address: SocketAddr,
    token: String,
    pub token_path: PathBuf,
    pub discovery_path: PathBuf,
    shutdown: tokio::sync::watch::Sender<bool>,
    server: tokio::task::JoinHandle<()>,
}

#[derive(Serialize)]
struct Discovery {
    port: u16,
}

pub async fn start(data_dir: impl AsRef<Path>, bind: SocketAddr) -> Result<Daemon, DaemonError> {
    if !bind.ip().is_loopback() {
        return Err(DaemonError::NonLoopback(bind));
    }
    let data_dir = data_dir.as_ref();
    std::fs::create_dir_all(data_dir).map_err(DaemonError::DataDirectory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(data_dir, std::fs::Permissions::from_mode(0o700))
            .map_err(DaemonError::DataDirectory)?;
    }
    let daemon_lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(data_dir.join("daemon.lock"))
        .map_err(DaemonError::DataDirectory)?;
    daemon_lock
        .try_lock()
        .map_err(|e| DaemonError::DataDirectory(std::io::Error::other(e)))?;
    let token_path = data_dir.join(TOKEN_FILE);
    let token = load_or_create_token(&token_path)?;
    harness_core::redaction::register_secret(&token);
    // Reuse a durable port on restart: binding it establishes single-daemon ownership
    // before touching leases. A second daemon with the same data directory fails closed.
    let bind = if bind.port() == 0 {
        std::fs::read(data_dir.join(PORT_FILE))
            .ok()
            .and_then(|v| serde_json::from_slice::<serde_json::Value>(&v).ok())
            .and_then(|v| v["port"].as_u64())
            .and_then(|p| u16::try_from(p).ok())
            .filter(|p| *p > 0)
            .map_or(bind, |p| SocketAddr::new(bind.ip(), p))
    } else {
        bind
    };
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(DaemonError::Bind)?;
    let db_path = data_dir.join("harness.db");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&db_path)
            .map_err(DaemonError::DataDirectory)?;
        std::fs::set_permissions(&db_path, std::fs::Permissions::from_mode(0o600))
            .map_err(DaemonError::DataDirectory)?;
    }
    let store = Store::open(db_path).await.map_err(DaemonError::Store)?;
    harness_runtime::recovery::reconcile_interrupted(&store, unix_seconds())
        .await
        .map_err(DaemonError::Store)?;
    let address = listener.local_addr().map_err(DaemonError::Bind)?;
    let discovery_path = data_dir.join(PORT_FILE);
    let discovery = serde_json::to_vec(&Discovery {
        port: address.port(),
    })
    .expect("discovery is serializable");
    std::fs::write(&discovery_path, discovery).map_err(DaemonError::DiscoveryFile)?;
    let (shutdown, mut stopping) = tokio::sync::watch::channel(false);
    let app = api_router_with_lifecycle(
        token.clone(),
        address.port(),
        store,
        Some(harness_runtime::engine::Engine::new(
            data_dir.join("execution"),
        )),
        Some(shutdown.clone()),
        Some(data_dir.to_path_buf()),
    );
    let server = tokio::spawn(async move {
        let _daemon_lock = daemon_lock;
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                if !*stopping.borrow() {
                    let _ = stopping.changed().await;
                }
            })
            .await;
    });
    Ok(Daemon {
        address,
        token,
        token_path,
        discovery_path,
        shutdown,
        server,
    })
}

impl Daemon {
    pub async fn run_until_shutdown(self) {
        let mut server = self.server;
        let interrupt = async {
            if tokio::signal::ctrl_c().await.is_err() {
                // Detached Windows services may not have a console. API shutdown
                // still works; lack of a signal handler must not stop the service.
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            _ = interrupt => {
                let _ = self.shutdown.send(true);
                let _ = server.await;
            },
            _ = &mut server => {},
        }
    }
    pub fn authorization_header(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

#[derive(Clone)]
struct ApiState {
    store: Store,
    event_waiters: Arc<tokio::sync::Semaphore>,
    engine: Option<harness_runtime::engine::Engine>,
    shutdown: Option<tokio::sync::watch::Sender<bool>>,
    /// Directory holding `providers.json`; required for the main planning agent.
    settings_root: Option<PathBuf>,
}

#[derive(Clone)]
struct AuthGuard {
    token: String,
    port: u16,
}

pub fn router(token: String) -> Router {
    Router::new().route("/healthz", get(healthz)).nest(
        "/v1",
        Router::new()
            .route("/control", get(control))
            .layer(axum::middleware::from_fn_with_state(
                AuthGuard { token, port: 0 },
                require_local_bearer,
            )),
    )
}

pub fn api_router(token: String, store: Store) -> Router {
    api_router_with_runtime(token, store, None)
}

pub fn api_router_with_runtime(
    token: String,
    store: Store,
    engine: Option<harness_runtime::engine::Engine>,
) -> Router {
    api_router_with_lifecycle(token, 0, store, engine, None, None)
}

/// Router whose main planning agent reads provider settings from `settings_root`.
pub fn api_router_with_settings(
    token: String,
    store: Store,
    engine: Option<harness_runtime::engine::Engine>,
    settings_root: PathBuf,
) -> Router {
    api_router_with_lifecycle(token, 0, store, engine, None, Some(settings_root))
}
fn api_router_with_lifecycle(
    token: String,
    port: u16,
    store: Store,
    engine: Option<harness_runtime::engine::Engine>,
    shutdown: Option<tokio::sync::watch::Sender<bool>>,
    settings_root: Option<PathBuf>,
) -> Router {
    Router::new().route("/healthz", get(healthz)).nest(
        "/v1",
        Router::new()
            .route("/control", get(control))
            .route("/dashboard", get(dashboard))
            .route("/tasks/{run_id}/{task_id}/evidence", get(task_evidence))
            .route("/shutdown", post(shutdown_service))
            .route("/projects", post(create_project))
            .route("/projects/{project_id}/trust", post(update_project_trust))
            .route(
                "/projects/{project_id}",
                axum::routing::delete(remove_project),
            )
            .route("/goals", post(submit_goal))
            .route("/goals/{run_id}", get(goal_status))
            .route("/plans", post(propose_plan))
            .route("/plans/{run_id}/{version}", get(get_plan))
            .route("/plans/{run_id}/{version}/approve", post(approve_plan))
            .route("/runs", post(create_run))
            .route("/runs/{run_id}", get(get_run))
            .route("/runs/{run_id}/source", get(execution_source))
            .route("/runs/{run_id}/pause", post(pause_run))
            .route("/runs/{run_id}/resume", post(resume_run))
            .route("/runs/{run_id}/cancel", post(cancel_run))
            .route("/runs/{run_id}/execute", post(execute_run))
            .route("/runs/{run_id}/tasks", get(list_tasks))
            .route(
                "/tasks/{run_id}/{task_id}/priority",
                patch(update_task_priority),
            )
            .route("/events", get(list_events))
            .route("/usage", get(list_usage))
            .route("/approvals", get(list_approvals))
            .route("/approvals/{approval_id}/decision", post(decide_approval))
            .with_state(ApiState {
                store,
                event_waiters: Arc::new(tokio::sync::Semaphore::new(64)),
                engine,
                shutdown,
                settings_root,
            })
            .layer(axum::middleware::from_fn_with_state(
                AuthGuard { token, port },
                require_local_bearer,
            )),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DashboardQuery {
    offset: Option<u32>,
    limit: Option<u32>,
}
async fn dashboard(
    State(state): State<ApiState>,
    Query(query): Query<DashboardQuery>,
) -> ApiResult<harness_store::dashboard::Dashboard> {
    let limit = query.limit.unwrap_or(100);
    if limit == 0 || limit > 200 {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_page",
            "limit must be 1..200",
        ));
    }
    let data = state
        .store
        .dashboard(query.offset.unwrap_or(0), limit)
        .await
        .map_err(store_error)?;
    Ok((StatusCode::OK, Json(data)))
}
async fn task_evidence(
    State(state): State<ApiState>,
    RoutePath((run, task)): RoutePath<(String, String)>,
) -> ApiResult<serde_json::Value> {
    use harness_store::{
        git_artifacts::GitArtifactRepository, verification::VerificationRepository,
    };
    let run = RunId::new(run).map_err(|e| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_run",
            e.to_string(),
        )
    })?;
    let task = harness_core::domain::TaskId::new(task).map_err(|e| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_task",
            e.to_string(),
        )
    })?;
    state
        .store
        .get_task(&run, &task)
        .await
        .map_err(store_error)?;
    let commit = state
        .store
        .get_task_commit(&run, &task)
        .await
        .map_err(store_error)?;
    let tests = state
        .store
        .list_test_results(&run, &task)
        .await
        .map_err(store_error)?;
    let failure = state
        .store
        .latest_task_failure(&run, &task)
        .await
        .map_err(store_error)?;
    let quarantine = state
        .store
        .latest_task_quarantine(&run, &task)
        .await
        .map_err(store_error)?;
    Ok((
        StatusCode::OK,
        Json(
            serde_json::json!({"commit":commit.map(|c|serde_json::json!({"task_commit_sha":c.task_commit_sha,"integration_commit_sha":c.integration_commit_sha,"diff_paths":c.diff_paths})),"tests":tests.into_iter().map(|t|serde_json::json!({"id":t.id,"command":t.command,"exit_code":t.exit_code,"duration_ms":t.duration_ms})).collect::<Vec<_>>(),"failure":failure,"quarantine":quarantine}),
        ),
    ))
}

async fn shutdown_service(State(state): State<ApiState>) -> ApiResult<serde_json::Value> {
    let shutdown = state.shutdown.ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "shutdown_unavailable",
            "service lifecycle is not configured",
        )
    })?;
    if state
        .engine
        .as_ref()
        .is_some_and(|engine| !engine.prepare_shutdown())
    {
        return Err(api_error(
            StatusCode::CONFLICT,
            "work_in_progress",
            "Nova has active runs; pause or cancel them and wait before stopping",
        ));
    }
    shutdown.send(true).map_err(|_| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "shutdown_failed",
            "service is already stopping",
        )
    })?;
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"status":"stopped"})),
    ))
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecuteRequest {
    #[serde(default)]
    committed_snapshot: bool,
    allowed_test_commands: Vec<Vec<String>>,
    concurrency: usize,
    #[serde(default)]
    live: bool,
    #[serde(default)]
    run_cap_microdollars: Option<u64>,
    #[serde(default)]
    daily_cap_microdollars: Option<u64>,
}

async fn execute_run(
    State(state): State<ApiState>,
    RoutePath(run_id): RoutePath<String>,
    Json(request): Json<ExecuteRequest>,
) -> ApiResult<serde_json::Value> {
    let settings_root = state.settings_root.as_ref().ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "settings_unavailable",
            "daemon settings are not configured",
        )
    })?;
    let settings_store = harness_runtime::provider_config::ProviderStore::new(settings_root);
    let settings = settings_store
        .load()
        .map_err(|error| api_error(StatusCode::CONFLICT, "invalid_settings", error))?;
    let run_cap = request
        .run_cap_microdollars
        .unwrap_or(settings.run_cap_microdollars);
    let daily_cap = request
        .daily_cap_microdollars
        .unwrap_or(settings.daily_cap_microdollars);
    if run_cap == 0
        || daily_cap == 0
        || run_cap > daily_cap
        || run_cap > settings.run_cap_microdollars
        || daily_cap > settings.daily_cap_microdollars
    {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "spend_cap_exceeds_saved_default",
            "request spend caps must be positive and no greater than saved defaults",
        ));
    }
    let run = load_run(&state.store, run_id).await?;
    let python = settings_store
        .python_for_daemon()
        .map_err(|error| api_error(StatusCode::SERVICE_UNAVAILABLE, "python_unavailable", error))?;
    let live = if request.live {
        let mut config = settings.live_config().map_err(|error| {
            api_error(StatusCode::UNPROCESSABLE_ENTITY, "live_unavailable", error)
        })?;
        config.run_cap_microdollars = run_cap;
        config.daily_cap_microdollars = daily_cap;
        Some(config)
    } else {
        None
    };
    let engine = state.engine.ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_unavailable",
            "execution runtime is not configured",
        )
    })?;
    engine.launch_configured(state.store, run.id.clone(), python, request.allowed_test_commands, request.concurrency, harness_runtime::engine::LaunchMode{live,committed_snapshot:request.committed_snapshot}).await.map_err(|error| api_error(StatusCode::CONFLICT, "execution_denied", if error.starts_with("working tree has uncommitted changes") { "Repository has uncommitted changes. Use committed snapshot in Execute, or commit/stash changes. Your files are untouched." } else { error.as_str() }))?;
    Ok((
        StatusCode::ACCEPTED,
        Json(
            serde_json::json!({"run_id":run.id.to_string(), "execution":"started", "committed_snapshot":request.committed_snapshot, "provider":if request.live {"configured"} else {"mock"}}),
        ),
    ))
}

async fn execution_source(
    State(state): State<ApiState>,
    RoutePath(run): RoutePath<String>,
) -> ApiResult<Option<harness_store::execution::ExecutionSource>> {
    let run = load_run(&state.store, run).await?;
    let source = state
        .store
        .execution_source(&run.id)
        .await
        .map_err(store_error)?;
    Ok((StatusCode::OK, Json(source)))
}

async fn control() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "authenticated"}))
}

async fn require_local_bearer(
    State(guard): State<AuthGuard>,
    request: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    if request.headers().contains_key(header::ORIGIN)
        || request
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|v| v != "none" && v != "same-origin")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let allowed_host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|host| {
            ["127.0.0.1", "localhost", "[::1]"]
                .iter()
                .find_map(|name| host.strip_prefix(name)?.strip_prefix(':'))
        })
        .and_then(|port| port.parse::<u16>().ok())
        .is_some_and(|port| port > 0 && (guard.port == 0 || port == guard.port));
    if !allowed_host {
        return StatusCode::FORBIDDEN.into_response();
    }
    let expected = format!("Bearer {}", guard.token);
    let authenticated = request
        .headers()
        .get(header::AUTHORIZATION)
        .is_some_and(|value| {
            value.as_bytes().len() == expected.len()
                && value
                    .as_bytes()
                    .iter()
                    .zip(expected.as_bytes())
                    .fold(0u8, |diff, (actual, expected)| diff | (actual ^ expected))
                    == 0
        });
    if !authenticated {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateProjectRequest {
    name: String,
    repo_path: String,
    #[serde(default)]
    trusted_local: bool,
}

#[derive(Serialize)]
struct ProjectResponse {
    id: String,
    name: String,
    repo_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposePlanRequest {
    run_id: String,
    plan: PlanProposal,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRunRequest {
    project_id: String,
}

#[derive(Serialize)]
struct RunResponse {
    reason: Option<String>,
    id: String,
    project_id: String,
    state: RunState,
    revision: u64,
}

#[derive(Serialize)]
struct TaskResponse {
    id: String,
    state: harness_core::domain::TaskState,
    revision: u64,
    attempt: u32,
    spec: harness_core::domain::TaskSpec,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PriorityRequest {
    revision: u64,
    manual_priority: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventsQuery {
    run_id: String,
    after: Option<i64>,
    limit: Option<u32>,
    wait_seconds: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageQuery {
    run_id: String,
}

#[derive(Serialize)]
struct EventResponse {
    sequence: i64,
    run_id: String,
    task_id: Option<String>,
    kind: String,
    from_state: Option<String>,
    to_state: Option<String>,
    attempt: Option<i64>,
}

#[derive(Serialize)]
struct PlanResponse {
    run_id: String,
    version: u32,
    status: String,
    plan: PlanProposal,
    requires_attention: Vec<String>,
}

#[derive(Serialize)]
struct ErrorResponse {
    code: &'static str,
    message: String,
}

type ApiResult<T> = Result<(StatusCode, Json<T>), (StatusCode, Json<ErrorResponse>)>;

fn store_error(error: harness_store::StoreError) -> (StatusCode, Json<ErrorResponse>) {
    if matches!(error, harness_store::StoreError::NotFound(_)) {
        api_error(StatusCode::NOT_FOUND, "not_found", error.to_string())
    } else {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "read_failed",
            "could not read dashboard data",
        )
    }
}

fn api_error(
    status: StatusCode,
    code: &'static str,
    message: impl Into<String>,
) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            code,
            message: harness_core::redaction::redact(&message.into()),
        }),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustRequest {
    trusted_local: bool,
}
async fn update_project_trust(
    State(state): State<ApiState>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<TrustRequest>,
) -> ApiResult<serde_json::Value> {
    let id = ProjectId::new(id).map_err(|e| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_project",
            e.to_string(),
        )
    })?;
    let project = state.store.get_project(&id).await.map_err(store_error)?;
    if request.trusted_local {
        inspect_repo(&project.repo_path).await.map_err(|e| {
            api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_repository",
                e.to_string(),
            )
        })?;
    }
    state
        .store
        .set_project_trust(&id, request.trusted_local)
        .await
        .map_err(store_error)?;
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"trusted_local":request.trusted_local})),
    ))
}
async fn remove_project(
    State(state): State<ApiState>,
    RoutePath(id): RoutePath<String>,
) -> ApiResult<serde_json::Value> {
    let id = ProjectId::new(id).map_err(|e| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_project",
            e.to_string(),
        )
    })?;
    state.store.remove_empty_project(&id).await.map_err(|e| {
        api_error(
            if matches!(e, harness_store::StoreError::NotFound(_)) {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::CONFLICT
            },
            "project_not_removed",
            e.to_string(),
        )
    })?;
    Ok((StatusCode::OK, Json(serde_json::json!({"removed":id}))))
}

async fn create_project(
    State(state): State<ApiState>,
    Json(request): Json<CreateProjectRequest>,
) -> ApiResult<ProjectResponse> {
    let root = if request.trusted_local {
        inspect_repo(&request.repo_path).await.map(|r| r.root)
    } else {
        // Do not invoke Git or repository helpers before the user's trust decision.
        harness_runtime::git::validate_repo_path(&request.repo_path)
    }
    .map_err(|error| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_repository",
            error.to_string(),
        )
    })?;
    let project = Project::new(
        ProjectId::new(Uuid::new_v4().to_string()).expect("UUID is a valid project id"),
        request.name,
        root.display().to_string(),
    )
    .map_err(|error| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_project",
            error.to_string(),
        )
    })?;
    state
        .store
        .create_project(&project)
        .await
        .map_err(|error| api_error(StatusCode::CONFLICT, "project_conflict", error.to_string()))?;
    state
        .store
        .set_project_trust(&project.id, request.trusted_local)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "trust_write_failed",
                "could not record trust decision",
            )
        })?;
    Ok((
        StatusCode::CREATED,
        Json(ProjectResponse {
            id: project.id.to_string(),
            name: project.name,
            repo_path: project.repo_path,
        }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalRequest {
    project_id: String,
    goal: String,
}

/// Hand a user goal to the main agent. Planning runs in the background; the run moves
/// to `awaiting_approval` with a proposed subtask plan, or to `failed` with a reason.
async fn submit_goal(
    State(state): State<ApiState>,
    Json(request): Json<GoalRequest>,
) -> ApiResult<serde_json::Value> {
    let goal = request.goal.trim().to_owned();
    if goal.chars().count() < 5 || goal.len() > 16_000 {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_goal",
            "describe the goal in 5..16000 characters",
        ));
    }
    let project_id = ProjectId::new(request.project_id).map_err(|error| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_project_id",
            error.to_string(),
        )
    })?;
    let project = state
        .store
        .get_project(&project_id)
        .await
        .map_err(|error| {
            api_error(
                StatusCode::NOT_FOUND,
                "project_not_found",
                error.to_string(),
            )
        })?;
    let root = state.settings_root.clone().ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "planner_unavailable",
            "the main planning agent is not configured",
        )
    })?;
    let run = Run::new(
        RunId::new(Uuid::new_v4().to_string()).expect("UUID is a valid run id"),
        project_id,
    );
    state
        .store
        .create_run(&run)
        .await
        .map_err(|error| api_error(StatusCode::CONFLICT, "run_conflict", error.to_string()))?;
    state
        .store
        .record_run_event(&run.id, "planning.started", &serde_json::json!({}))
        .await
        .map_err(store_error)?;
    let store = state.store.clone();
    let run_id = run.id.clone();
    tokio::spawn(async move {
        let outcome =
            match harness_runtime::orchestrator::plan_goal(&root, &project, &goal, run_id.as_str())
                .await
            {
                Ok(planned) => match store.propose_plan(&run_id, &planned.plan).await {
                    Ok(_) => Ok(planned),
                    Err(error) => Err(format!("main agent plan rejected: {error}")),
                },
                Err(reason) => Err(reason),
            };
        match outcome {
            Ok(planned) => {
                let _ = store
                    .record_run_event(
                        &run_id,
                        "planning.completed",
                        &serde_json::json!({
                            "model": planned.model,
                            "subtasks": planned.plan.tasks.len(),
                            "input_tokens": planned.input_tokens,
                            "output_tokens": planned.output_tokens,
                            "cost_usd": planned.cost_usd,
                        }),
                    )
                    .await;
            }
            Err(reason) => {
                let reason: String = harness_core::redaction::redact(&reason)
                    .chars()
                    .take(300)
                    .collect();
                if let Ok(current) = store.get_run(&run_id).await
                    && current.state == RunState::Planning
                {
                    let _ = store
                        .transition_run(&run_id, current.revision, RunState::Failed, None)
                        .await;
                }
                let _ = store
                    .record_run_event(
                        &run_id,
                        "planning.failed",
                        &serde_json::json!({ "reason": reason }),
                    )
                    .await;
            }
        }
    });
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "run": run_response(run), "planning": "started" })),
    ))
}

/// Main-agent planning status for a run: `planning`, `proposed` or `failed` (+ details).
async fn goal_status(
    State(state): State<ApiState>,
    RoutePath(run_id): RoutePath<String>,
) -> ApiResult<serde_json::Value> {
    let run = load_run(&state.store, run_id).await?;
    let event = state
        .store
        .latest_run_event(&run.id, "planning.")
        .await
        .map_err(store_error)?;
    let Some((kind, mut detail)) = event else {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "goal_not_found",
            "this run was not planned by the main agent",
        ));
    };
    let status = match kind.as_str() {
        "planning.completed" => "proposed",
        "planning.failed" => "failed",
        _ => "planning",
    };
    if !detail.is_object() {
        detail = serde_json::json!({});
    }
    detail["status"] = status.into();
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "run": run_response(run), "planning": detail })),
    ))
}

async fn propose_plan(
    State(state): State<ApiState>,
    Json(request): Json<ProposePlanRequest>,
) -> ApiResult<PlanResponse> {
    let run_id = RunId::new(request.run_id).map_err(|error| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_run_id",
            error.to_string(),
        )
    })?;
    let run =
        state.store.get_run(&run_id).await.map_err(|error| {
            api_error(StatusCode::NOT_FOUND, "run_not_found", error.to_string())
        })?;
    if request.plan.project_id != run.project_id.as_str() {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "project_mismatch",
            "plan project_id does not match the run project",
        ));
    }
    let record = state
        .store
        .propose_plan(&run_id, &request.plan)
        .await
        .map_err(|error| {
            api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_plan",
                error.to_string(),
            )
        })?;
    Ok((StatusCode::CREATED, Json(plan_response(record))))
}

async fn get_plan(
    State(state): State<ApiState>,
    RoutePath((run_id, version)): RoutePath<(String, u32)>,
) -> ApiResult<PlanResponse> {
    let run_id = RunId::new(run_id).map_err(|error| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_run_id",
            error.to_string(),
        )
    })?;
    let record = state
        .store
        .get_plan(&run_id, version)
        .await
        .map_err(|error| api_error(StatusCode::NOT_FOUND, "plan_not_found", error.to_string()))?;
    Ok((StatusCode::OK, Json(plan_response(record))))
}

async fn approve_plan(
    State(state): State<ApiState>,
    RoutePath((run_id, version)): RoutePath<(String, u32)>,
) -> ApiResult<PlanResponse> {
    let run_id = RunId::new(run_id).map_err(|error| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_run_id",
            error.to_string(),
        )
    })?;
    let record = state
        .store
        .approve_plan(&run_id, version)
        .await
        .map_err(|error| {
            api_error(
                StatusCode::CONFLICT,
                "plan_not_approvable",
                error.to_string(),
            )
        })?;
    Ok((StatusCode::OK, Json(plan_response(record))))
}

fn plan_response(record: PlanRecord) -> PlanResponse {
    let status = match record.status {
        PlanStatus::Proposed => "proposed",
        PlanStatus::Approved => "approved",
        PlanStatus::Rejected => "rejected",
    };
    let requires_attention = harness_runtime::orchestrator::attention_tasks(&record.plan);
    PlanResponse {
        run_id: record.run_id.to_string(),
        version: record.version,
        status: status.to_owned(),
        plan: record.plan,
        requires_attention,
    }
}

async fn create_run(
    State(state): State<ApiState>,
    Json(request): Json<CreateRunRequest>,
) -> ApiResult<RunResponse> {
    let project_id = ProjectId::new(request.project_id).map_err(|error| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_project_id",
            error.to_string(),
        )
    })?;
    state
        .store
        .get_project(&project_id)
        .await
        .map_err(|error| {
            api_error(
                StatusCode::NOT_FOUND,
                "project_not_found",
                error.to_string(),
            )
        })?;
    let run = Run::new(
        RunId::new(Uuid::new_v4().to_string()).expect("UUID is a valid run id"),
        project_id,
    );
    state
        .store
        .create_run(&run)
        .await
        .map_err(|error| api_error(StatusCode::CONFLICT, "run_conflict", error.to_string()))?;
    Ok((StatusCode::CREATED, Json(run_response(run))))
}

async fn get_run(
    State(state): State<ApiState>,
    RoutePath(run_id): RoutePath<String>,
) -> ApiResult<RunResponse> {
    let run = load_run(&state.store, run_id).await?;
    let diagnostic = state
        .store
        .latest_run_event(&run.id, "run.")
        .await
        .map_err(|_| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "run_diagnostic",
                "run diagnostic unavailable",
            )
        })?;
    let mut response = run_response(run);
    response.reason =
        diagnostic.and_then(|(_, payload)| payload["reason"].as_str().map(str::to_owned));
    Ok((StatusCode::OK, Json(response)))
}

async fn pause_run(
    State(state): State<ApiState>,
    RoutePath(run_id): RoutePath<String>,
) -> ApiResult<RunResponse> {
    transition_run(&state.store, run_id, RunState::Paused).await
}

async fn resume_run(
    State(state): State<ApiState>,
    RoutePath(run_id): RoutePath<String>,
) -> ApiResult<RunResponse> {
    let run = transition_run_internal(&state.store, run_id, RunState::Running).await?;
    tick(&state.store, &run.id, unix_seconds())
        .await
        .map_err(|error| {
            api_error(
                StatusCode::CONFLICT,
                "scheduler_tick_failed",
                error.to_string(),
            )
        })?;
    let run =
        state.store.get_run(&run.id).await.map_err(|error| {
            api_error(StatusCode::NOT_FOUND, "run_not_found", error.to_string())
        })?;
    Ok((StatusCode::OK, Json(run_response(run))))
}

async fn cancel_run(
    State(state): State<ApiState>,
    RoutePath(run_id): RoutePath<String>,
) -> ApiResult<RunResponse> {
    let run = transition_run_internal(&state.store, run_id, RunState::Cancelled).await?;
    for task in state.store.list_tasks(&run.id).await.map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "cancel_failed",
            "could not enumerate tasks",
        )
    })? {
        if harness_core::state::is_allowed(task.state, harness_core::domain::TaskState::Cancelled) {
            state
                .store
                .transition_task(
                    &run.id,
                    task.id(),
                    task.revision,
                    harness_core::domain::TaskState::Cancelled,
                    None,
                )
                .await
                .map_err(|_| {
                    api_error(
                        StatusCode::CONFLICT,
                        "cancel_race",
                        "task moved during cancellation; inspect state",
                    )
                })?;
        }
    }
    Ok((StatusCode::OK, Json(run_response(run))))
}

async fn list_tasks(
    State(state): State<ApiState>,
    RoutePath(run_id): RoutePath<String>,
) -> ApiResult<Vec<TaskResponse>> {
    let run = load_run(&state.store, run_id).await?;
    let tasks =
        state.store.list_tasks(&run.id).await.map_err(|error| {
            api_error(StatusCode::NOT_FOUND, "run_not_found", error.to_string())
        })?;
    Ok((
        StatusCode::OK,
        Json(tasks.into_iter().map(task_response).collect()),
    ))
}

async fn transition_run(store: &Store, run_id: String, target: RunState) -> ApiResult<RunResponse> {
    let run = transition_run_internal(store, run_id, target).await?;
    Ok((StatusCode::OK, Json(run_response(run))))
}

async fn transition_run_internal(
    store: &Store,
    run_id: String,
    target: RunState,
) -> Result<Run, (StatusCode, Json<ErrorResponse>)> {
    let run = load_run(store, run_id).await?;
    store
        .transition_run(
            &run.id,
            run.revision,
            target,
            Some(&format!("api:run:{target:?}:{}:{}", run.id, run.revision)),
        )
        .await
        .map_err(|error| {
            api_error(
                StatusCode::CONFLICT,
                "run_not_transitionable",
                error.to_string(),
            )
        })
}

async fn load_run(store: &Store, run_id: String) -> Result<Run, (StatusCode, Json<ErrorResponse>)> {
    let run_id = RunId::new(run_id).map_err(|error| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_run_id",
            error.to_string(),
        )
    })?;
    store
        .get_run(&run_id)
        .await
        .map_err(|error| api_error(StatusCode::NOT_FOUND, "run_not_found", error.to_string()))
}

fn run_response(run: Run) -> RunResponse {
    RunResponse {
        reason: None,
        id: run.id.to_string(),
        project_id: run.project_id.to_string(),
        state: run.state,
        revision: run.revision,
    }
}

fn task_response(task: Task) -> TaskResponse {
    TaskResponse {
        id: task.id().to_string(),
        state: task.state,
        revision: task.revision,
        attempt: task.attempt,
        spec: task.spec,
    }
}

fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

fn load_or_create_token(path: &Path) -> Result<String, DaemonError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() {
                return Err(DaemonError::TokenFile(std::io::Error::other(
                    "token path must be a regular file",
                )));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                    .map_err(DaemonError::TokenFile)?;
            }
            let token = std::fs::read_to_string(path).map_err(DaemonError::TokenFile)?;
            let token = token.trim();
            if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(DaemonError::TokenFile(std::io::Error::other(
                    "daemon token is invalid",
                )));
            }
            Ok(token.to_owned())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).map_err(|error| {
                DaemonError::TokenFile(std::io::Error::other(error.to_string()))
            })?;
            let token = bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            write_token(path, &token)?;
            Ok(token)
        }
        Err(error) => Err(DaemonError::TokenFile(error)),
    }
}

#[cfg(unix)]
fn write_token(path: &Path, token: &str) -> Result<(), DaemonError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(DaemonError::TokenFile)?;
    file.write_all(token.as_bytes())
        .map_err(DaemonError::TokenFile)
}

#[cfg(windows)]
fn write_token(path: &Path, token: &str) -> Result<(), DaemonError> {
    std::fs::write(path, token).map_err(DaemonError::TokenFile)
}
async fn update_task_priority(
    State(state): State<ApiState>,
    RoutePath((run_id, task_id)): RoutePath<(String, String)>,
    Json(request): Json<PriorityRequest>,
) -> ApiResult<TaskResponse> {
    let run = load_run(&state.store, run_id).await?;
    let task_id = harness_core::domain::TaskId::new(task_id).map_err(|error| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_task_id",
            error.to_string(),
        )
    })?;
    let priority = request
        .manual_priority
        .map(Priority::new)
        .transpose()
        .map_err(|error| {
            api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_priority",
                error.to_string(),
            )
        })?;
    let task = state
        .store
        .set_priority(
            &run.id,
            &task_id,
            request.revision,
            PriorityUpdate::Manual(priority),
        )
        .await
        .map_err(|error| {
            let status = if matches!(error, harness_store::StoreError::RevisionConflict { .. }) {
                StatusCode::CONFLICT
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            api_error(status, "priority_not_updated", error.to_string())
        })?;
    Ok((StatusCode::OK, Json(task_response(task))))
}

async fn list_events(
    State(state): State<ApiState>,
    Query(query): Query<EventsQuery>,
) -> ApiResult<Vec<EventResponse>> {
    let run = load_run(&state.store, query.run_id).await?;
    let wait = query.wait_seconds.unwrap_or(0);
    if wait > 25
        || query.after.is_some_and(|v| v < 0)
        || query.limit.is_some_and(|v| v == 0 || v > 1000)
    {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_cursor",
            "limit must be 1..1000, cursor nonnegative and wait at most 25 seconds",
        ));
    }
    let _permit = if wait > 0 {
        Some(state.event_waiters.try_acquire().map_err(|_| {
            api_error(
                StatusCode::TOO_MANY_REQUESTS,
                "event_capacity",
                "too many event subscribers",
            )
        })?)
    } else {
        None
    };
    let deadline = Instant::now() + Duration::from_secs(u64::from(wait));
    let events = loop {
        let events = state
            .store
            .list_events(
                &run.id,
                query.after.unwrap_or(0),
                query.limit.unwrap_or(100),
            )
            .await
            .map_err(|error| {
                api_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_cursor",
                    error.to_string(),
                )
            })?;
        if !events.is_empty() || Instant::now() >= deadline {
            break events;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    Ok((
        StatusCode::OK,
        Json(
            events
                .into_iter()
                .map(|event| EventResponse {
                    sequence: event.sequence,
                    run_id: event.run_id,
                    task_id: event.task_id,
                    kind: event.kind,
                    from_state: event.from_state,
                    to_state: event.to_state,
                    attempt: event.attempt,
                })
                .collect(),
        ),
    ))
}

async fn list_usage(
    State(state): State<ApiState>,
    Query(query): Query<UsageQuery>,
) -> ApiResult<Vec<serde_json::Value>> {
    let _run = load_run(&state.store, query.run_id).await?;
    let records = state
        .store
        .list_usage(&_run.id, None)
        .await
        .map_err(|error| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "usage_read_failed",
                error.to_string(),
            )
        })?;
    let values = records
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "usage_read_failed",
                error.to_string(),
            )
        })?;
    Ok((StatusCode::OK, Json(values)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalDecision {
    revision: i64,
    approved: bool,
}
async fn list_approvals(
    State(state): State<ApiState>,
    Query(query): Query<UsageQuery>,
) -> ApiResult<Vec<harness_store::approvals::Approval>> {
    let run = load_run(&state.store, query.run_id).await?;
    let approvals = state.store.list_approvals(&run.id).await.map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "approval_read_failed",
            "could not list approvals",
        )
    })?;
    Ok((StatusCode::OK, Json(approvals)))
}
async fn decide_approval(
    State(state): State<ApiState>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<ApprovalDecision>,
) -> ApiResult<harness_store::approvals::Approval> {
    let approval = state
        .store
        .decide_approval(&id, request.revision, request.approved)
        .await
        .map_err(|e| {
            let status = if matches!(e, harness_store::StoreError::NotFound(_)) {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::CONFLICT
            };
            api_error(
                status,
                "approval_not_decidable",
                "approval missing, stale, terminal or already decided",
            )
        })?;
    Ok((StatusCode::OK, Json(approval)))
}

pub fn default_data_dir() -> Result<PathBuf, DaemonError> {
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home).join("Library/Application Support/Nova"));
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .ok_or_else(|| DaemonError::DataDirectory(std::io::Error::other("home unavailable")))?;
    Ok(PathBuf::from(base).join("Nova"))
}

pub fn loopback_ephemeral() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0)
}
