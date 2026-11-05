//! Loopback-only authenticated control server.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
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
    let app = router(token.clone());
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
        format!("Bearer {}", self.token)
    }
}

pub fn router(token: String) -> Router {
    Router::new().route("/healthz", get(healthz)).nest(
        "/v1",
        Router::new()
            .route("/control", get(control))
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
    axum::extract::State(token): axum::extract::State<String>,
    request: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let expected = format!("Bearer {token}");
    match request.headers().get(header::AUTHORIZATION) {
        Some(value) if value.as_bytes() == expected.as_bytes() => next.run(request).await,
        _ => StatusCode::UNAUTHORIZED.into_response(),
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
