use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output};

struct Service(tempfile::TempDir);
impl Service {
    fn call(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_nova"))
            .arg("--daemon-dir")
            .arg(self.0.path())
            .arg("--json")
            .args(args)
            .output()
            .unwrap()
    }
    fn json(&self, args: &[&str]) -> serde_json::Value {
        let result = self.call(args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice(&result.stdout).unwrap()
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.call(&["stop"]);
    }
}

#[test]
fn bare_nova_starts_attaches_and_stops_without_losing_state() {
    let service = Service(tempfile::tempdir().unwrap());
    let started = service.json(&[]);
    assert_eq!(started["name"], "Nova");
    assert_eq!(started["status"], "running");
    assert_eq!(started["started"], true);
    assert!(service.0.path().join("daemon.token").exists());
    let discovery = std::fs::read(service.0.path().join("daemon.json")).unwrap();
    assert_eq!(service.json(&["start"])["started"], false);
    assert_eq!(
        std::fs::read(service.0.path().join("daemon.json")).unwrap(),
        discovery
    );
    assert_eq!(service.json(&["stop"])["status"], "stopped");
    assert!(service.0.path().join("harness.db").exists());
    assert_eq!(service.json(&["stop"])["status"], "stopped");
    assert_eq!(service.json(&[])["started"], true);
    assert_eq!(
        std::fs::read(service.0.path().join("daemon.json")).unwrap(),
        discovery
    );
}

#[test]
fn nova_help_does_not_start_a_service() {
    let service = Service(tempfile::tempdir().unwrap());
    let result = service.call(&["--help"]);
    assert!(result.status.success());
    let help = String::from_utf8(result.stdout).unwrap();
    assert!(help.contains("nova"));
    assert!(help.contains("start"));
    assert!(help.contains("stop"));
    assert!(!help.contains("__serve"));
    assert!(!service.0.path().join("harness.db").exists());
}

#[test]
fn startup_rejects_impersonating_listener() {
    let service = Service(tempfile::tempdir().unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::fs::write(
        service.0.path().join("daemon.json"),
        serde_json::json!({"port": port}).to_string(),
    )
    .unwrap();
    std::fs::write(service.0.path().join("daemon.token"), "a".repeat(64)).unwrap();
    let fake = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let _ = stream.read(&mut request).unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"status\":\"ok\"}")
            .unwrap();
    });
    let response = service.call(&["start"]);
    fake.join().unwrap();
    assert!(!response.status.success());
    assert!(String::from_utf8_lossy(&response.stderr).contains("another process owns port"));
}
