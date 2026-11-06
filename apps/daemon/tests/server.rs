use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::body::to_bytes;
use axum::http::{Request, StatusCode, header};
use harness_daemon::{DaemonError, loopback_ephemeral, router, start};
use tempfile::tempdir;
use tower::ServiceExt;

const TOKEN: &str = "test-token";

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
async fn missing_token_401() {
    let response = router(TOKEN.into())
        .oneshot(
            Request::get("/v1/control")
                .header(header::HOST, "localhost:8080")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_token_401() {
    let response = router(TOKEN.into())
        .oneshot(
            Request::get("/v1/control")
                .header(header::HOST, "localhost:8080")
                .header(header::AUTHORIZATION, "Bearer wrong")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn foreign_host_header_403() {
    let response = router(TOKEN.into())
        .oneshot(
            Request::get("/v1/control")
                .header(header::HOST, "evil.example:8080")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn hostile_browser_origin_denied_even_with_bearer() {
    let response = router(TOKEN.into())
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
async fn healthz_needs_no_token() {
    let response = router(TOKEN.into())
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
async fn ephemeral_port_is_persisted_with_token() {
    let dir = tempdir().unwrap();
    let daemon = start(dir.path(), loopback_ephemeral()).await.unwrap();
    assert!(
        start(dir.path(), loopback_ephemeral()).await.is_err(),
        "second daemon must not acquire the same state directory"
    );
    assert_ne!(daemon.address.port(), 0);
    assert!(daemon.token_path.exists());
    assert!(daemon.discovery_path.exists());
    let request = Request::get("/v1/control")
        .header(header::HOST, format!("localhost:{}", daemon.address.port()))
        .header(header::AUTHORIZATION, daemon.authorization_header())
        .body(axum::body::Body::empty())
        .unwrap();
    let response = router(std::fs::read_to_string(&daemon.token_path).unwrap())
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[cfg(unix)]
#[tokio::test]
async fn data_dir_is_0700() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir().unwrap();
    let daemon = start(dir.path(), loopback_ephemeral()).await.unwrap();
    assert_eq!(
        std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(daemon.token_path.exists());
    assert_eq!(
        std::fs::metadata(dir.path().join("harness.db"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[tokio::test]
async fn token_file_is_0600() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir().unwrap();
    let daemon = start(dir.path(), loopback_ephemeral()).await.unwrap();
    assert_eq!(
        std::fs::metadata(&daemon.token_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[tokio::test]
async fn bound_port_host_and_bearer_enforced() {
    use std::io::{Read, Write};
    let dir = tempdir().unwrap();
    let daemon = start(dir.path(), loopback_ephemeral()).await.unwrap();
    let address = daemon.address;
    let bearer = daemon.authorization_header();
    let request = move |host_port: u16, authorization: Option<&str>| {
        let authorization =
            authorization.map_or(String::new(), |value| format!("Authorization: {value}\r\n"));
        let wire = format!(
            "GET /v1/control HTTP/1.1\r\nHost: localhost:{host_port}\r\n{authorization}Connection: close\r\n\r\n"
        );
        tokio::task::spawn_blocking(move || {
            let mut stream = std::net::TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            stream.write_all(wire.as_bytes()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        })
    };
    let valid = request(address.port(), Some(&bearer)).await.unwrap();
    assert!(valid.starts_with("HTTP/1.1 200"), "{valid}");
    let wrong_port = request(address.port().saturating_sub(1), Some(&bearer))
        .await
        .unwrap();
    assert!(wrong_port.starts_with("HTTP/1.1 403"), "{wrong_port}");
    let no_token = request(address.port(), None).await.unwrap();
    assert!(no_token.starts_with("HTTP/1.1 401"), "{no_token}");
}
