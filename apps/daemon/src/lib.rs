//! Loopback-only authenticated control server.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::{Path as RoutePath, State};
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use harness_core::domain::{Project, ProjectId, RunId};
use harness_core::plan::PlanProposal;
use harness_runtime::git::register_repo;
use harness_store::Store;
use harness_store::plans::{PlanRecord, PlanRepository, PlanStatus};
use harness_store::repository::Repository;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const TOKEN_FILE: &str = "api-token";
const PORT_FILE: &str = "daemon.json";

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("daemon may only bind a loopback address: {0}")]
    NonLoopback(SocketAddr),
    #[error("could not access daemon data directory: {0}")]
    DataDirectory(#[source] std::io::Error),
    #[error("could not create API token file: {0}")]
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
    let token_path = data_dir.join(TOKEN_FILE);
    let token = load_or_create_token(&token_path)?;
    let store = Store::open(data_dir.join("harness.db"))
        .await
        .map_err(DaemonError::Store)?;
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(DaemonError::Bind)?;
    let address = listener.local_addr().map_err(DaemonError::Bind)?;
    let discovery_path = data_dir.join(PORT_FILE);
    let discovery = serde_json::to_vec(&Discovery {
        port: address.port(),
    })
    .expect("discovery is serializable");
    std::fs::write(&discovery_path, discovery).map_err(DaemonError::DiscoveryFile)?;
    let app = api_router(token.clone(), store);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(Daemon {
        address,
        token,
        token_path,
        discovery_path,
    })
}

impl Daemon {
    pub fn authorization_header(&self) -> String {
        ["Bearer", &self.token].join(" ")
    }
}

#[derive(Clone)]
struct ApiState {
    store: Store,
}

pub fn router(token: String) -> Router {
    Router::new().route("/healthz", get(healthz)).nest(
        "/v1",
        Router::new()
            .route("/control", get(control))
            .layer(axum::middleware::from_fn_with_state(token, require_bearer)),
    )
}

pub fn api_router(token: String, store: Store) -> Router {
    Router::new().route("/healthz", get(healthz)).nest(
        "/v1",
        Router::new()
            .route("/control", get(control))
            .route("/projects", post(create_project))
            .route("/plans", post(propose_plan))
            .route("/plans/{run_id}/{version}", get(get_plan))
            .route("/plans/{run_id}/{version}/approve", post(approve_plan))
            .with_state(ApiState { store })
            .layer(axum::middleware::from_fn_with_state(token, require_bearer)),
    )
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

async fn control() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "authenticated"}))
}

async fn require_bearer(
    State(token): State<String>,
    request: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let expected = ["Bearer", &token].join(" ");
    match request.headers().get(header::AUTHORIZATION) {
        Some(value) if value.as_bytes() == expected.as_bytes() => next.run(request).await,
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateProjectRequest {
    name: String,
    repo_path: String,
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

#[derive(Serialize)]
struct PlanResponse {
    run_id: String,
    version: u32,
    status: String,
    plan: PlanProposal,
}

#[derive(Serialize)]
struct ErrorResponse {
    code: &'static str,
    message: String,
}

type ApiResult<T> = Result<(StatusCode, Json<T>), (StatusCode, Json<ErrorResponse>)>;

fn api_error(
    status: StatusCode,
    code: &'static str,
    message: impl Into<String>,
) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            code,
            message: message.into(),
        }),
    )
}

async fn create_project(
    State(state): State<ApiState>,
    Json(request): Json<CreateProjectRequest>,
) -> ApiResult<ProjectResponse> {
    let trusted = register_repo(&request.repo_path).await.map_err(|error| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_repository",
            error.to_string(),
        )
    })?;
    let project = Project::new(
        ProjectId::new(Uuid::new_v4().to_string()).expect("UUID is a valid project id"),
        request.name,
        trusted.root.display().to_string(),
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
    Ok((
        StatusCode::CREATED,
        Json(ProjectResponse {
            id: project.id.to_string(),
            name: project.name,
            repo_path: project.repo_path,
        }),
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
    PlanResponse {
        run_id: record.run_id.to_string(),
        version: record.version,
        status: status.to_owned(),
        plan: record.plan,
    }
}

fn load_or_create_token(path: &Path) -> Result<String, DaemonError> {
    match std::fs::read_to_string(path) {
        Ok(token) if !token.trim().is_empty() => Ok(token.trim().to_owned()),
        Ok(_) => Err(DaemonError::TokenFile(std::io::Error::other(
            "token file is empty",
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
            write_token(path, &token)?;
            Ok(token)
        }
        Err(error) => Err(DaemonError::TokenFile(error)),
    }
}

#[cfg(unix)]
fn write_token(path: &Path, token: &str) -> Result<(), DaemonError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = std::fs::OpenOptions::new();
    let mut file = options
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(DaemonError::TokenFile)?;
    std::io::Write::write_all(&mut file, token.as_bytes()).map_err(DaemonError::TokenFile)
}

#[cfg(windows)]
fn write_token(path: &Path, token: &str) -> Result<(), DaemonError> {
    // `%LOCALAPPDATA%` is per-user; the default inherited ACL restricts this file to that user.
    std::fs::write(path, token).map_err(DaemonError::TokenFile)
}

pub fn default_data_dir() -> Result<PathBuf, DaemonError> {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .ok_or_else(|| DaemonError::DataDirectory(std::io::Error::other("home unavailable")))?;
    Ok(PathBuf::from(base).join("AIHarness"))
}

pub fn loopback_ephemeral() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 0)
}
