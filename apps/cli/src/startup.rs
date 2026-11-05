//! Single-command, detached service lifecycle. Startup never approves or executes a task.
use super::{CliError, Client};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::{
    fs::OpenOptions,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn running(directory: &Path) -> Result<Option<Client>, CliError> {
    let client = match Client::from_data_dir(directory) {
        Ok(client) => client,
        Err(CliError::Discovery(e) | CliError::Token(e))
            if e.kind() == std::io::ErrorKind::NotFound =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    match client.request_with_timeout("GET", "/v1/control", None, Duration::from_secs(1)) {
        Ok(response) => {
            let status: serde_json::Value =
                serde_json::from_str(&response).map_err(CliError::DiscoveryJson)?;
            if status["status"] != "authenticated" {
                return Err(CliError::Startup(
                    "unexpected local service response".into(),
                ));
            }
            Ok(Some(client))
        }
        Err(CliError::Connect(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

fn ready(directory: &Path, json: bool, started: bool) -> String {
    if json {
        return serde_json::json!({"name":"Nova","status":"running","started":started,"data_dir":directory}).to_string();
    }
    format!(
        "Nova is running{}.\n\nUse `nova --help` to see commands.\nUse `nova stop` to stop the service.\n\nNo agents start until you approve and execute a plan.",
        if started {
            " in the background"
        } else {
            " (already started)"
        }
    )
}

pub(super) fn start(directory: &Path, json: bool) -> Result<String, CliError> {
    if running(directory)?.is_some() {
        return Ok(ready(directory, json, false));
    }
    std::fs::create_dir_all(directory).map_err(|e| CliError::Startup(e.to_string()))?;
    let log_path = directory.join("nova-service.log");
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let log = options
        .open(&log_path)
        .map_err(|e| CliError::Startup(e.to_string()))?;
    let executable = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|e| CliError::Startup(e.to_string()))?;
    let mut command = Command::new(executable);
    command
        .arg("--daemon-dir")
        .arg(directory)
        .arg("__serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x00000008 | 0x00000200); // DETACHED_PROCESS | NEW_PROCESS_GROUP
    }
    let mut child = command
        .spawn()
        .map_err(|e| CliError::Startup(e.to_string()))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match running(directory) {
            Ok(Some(_)) => return Ok(ready(directory, json, true)),
            Ok(None) | Err(CliError::DiscoveryJson(_)) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|e| CliError::Startup(e.to_string()))?
        {
            return Err(CliError::Startup(format!(
                "service exited with {status}; see {}",
                log_path.display()
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err(CliError::Startup(format!(
        "service did not become ready; see {}",
        log_path.display()
    )))
}

pub(super) fn stop(directory: &Path, json: bool) -> Result<String, CliError> {
    if let Some(client) = running(directory)? {
        client.request("POST", "/v1/shutdown", None)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if running(directory)?.is_none() {
                return Ok(stopped(json));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        return Err(CliError::Startup(
            "service is still draining connections".into(),
        ));
    }
    Ok(stopped(json))
}
fn stopped(json: bool) -> String {
    if json {
        "{\"name\":\"Nova\",\"status\":\"stopped\"}".into()
    } else {
        "Nova stopped.".into()
    }
}
