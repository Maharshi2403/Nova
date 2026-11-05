use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::body::to_bytes;
use axum::http::{Request, StatusCode, header};
use harness_daemon::{DaemonError, loopback_ephemeral, router, start};
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn nonloopback_binding_rejected() {
    let dir = tempdir().unwrap();
    let error = start(
        dir.path(),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, DaemonError::NonLoopback(_)));
}

#[tokio::test]
async fn missing_auth_401() {
    let response = router("test-token".into())
        .oneshot(
            Request::get("/v1/control")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn healthz_minimal() {
    let response = router("test-token".into())
        .oneshot(
            Request::get("/healthz")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(body, r#"{"status":"ok"}"#);
}

#[tokio::test]
async fn ephemeral_port_and_token_are_persisted() {
    let dir = tempdir().unwrap();
    let daemon = start(dir.path(), loopback_ephemeral()).await.unwrap();
    assert_ne!(daemon.address.port(), 0);
    assert!(daemon.token_path.exists());
    assert!(daemon.discovery_path.exists());
    let request = Request::get("/v1/control")
        .header(header::AUTHORIZATION, daemon.authorization_header())
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router(std::fs::read_to_string(&daemon.token_path).unwrap())
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
