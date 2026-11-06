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
async fn control_without_authentication_succeeds() {
    let response = router()
        .oneshot(
            Request::get("/v1/control")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn hostile_browser_origin_denied_even_with_bearer() {
    let response = router()
        .oneshot(
            Request::get("/v1/control")
                .header(header::AUTHORIZATION, "Bearer test-token")
                .header(header::ORIGIN, "https://hostile.example")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn healthz_minimal() {
    let response = router()
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
async fn ephemeral_port_is_persisted_without_token() {
    let dir = tempdir().unwrap();
    let daemon = start(dir.path(), loopback_ephemeral()).await.unwrap();
    assert!(
        start(dir.path(), loopback_ephemeral()).await.is_err(),
        "second daemon must not acquire the same state directory"
    );
    assert_ne!(daemon.address.port(), 0);
    assert!(!dir.path().join("api-token").exists());
    assert!(daemon.discovery_path.exists());
    let request = Request::get("/v1/control")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
