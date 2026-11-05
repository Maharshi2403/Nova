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
