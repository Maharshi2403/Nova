use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// AI Harness: local-first coding-agent runtime.
#[derive(Parser, Debug)]
#[command(name = "harness", version, about)]
struct Cli {
    /// Print the API response as stable JSON.
    #[arg(long)]
    json: bool,
    /// Daemon state directory. Defaults to the per-user AIHarness data directory.
    #[arg(long, global = true)]
    daemon_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Approvals {
        run_id: String,
    },
    Decision {
        approval_id: String,
        revision: i64,
        #[arg(long)]
        approve: bool,
    },
    /// Execute an approved run using the deterministic offline fixture worker.
    Execute {
        run_id: String,
        #[arg(long)]
        python: PathBuf,
        /// JSON array of exact argv arrays approved for verifier execution.
        #[arg(long)]
        test_commands: PathBuf,
        #[arg(long, default_value_t = 2)]
        concurrency: usize,
    },
    Init {
        repo: PathBuf,
        #[arg(long)]
        name: String,
        /// Authorize unsandboxed execution of this trusted local repository.
        #[arg(long)]
        trust_local: bool,
    },
    Plan {
        run_id: String,
        file: PathBuf,
    },
    Approve {
        run_id: String,
        version: u32,
    },
    Run {
        project_id: String,
    },
    Status {
        run_id: String,
    },
    Task {
        run_id: String,
    },
    Priority {
        run_id: String,
        task_id: String,
        revision: u64,
        #[arg(long)]
        manual_priority: Option<i64>,
    },
    Pause {
        run_id: String,
    },
    Resume {
        run_id: String,
    },
    Logs {
        run_id: String,
        #[arg(long, default_value_t = 0)]
        after: i64,
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("could not read daemon discovery: {0}")]
    Discovery(#[source] std::io::Error),
    #[error("daemon discovery metadata is invalid: {0}")]
    DiscoveryJson(#[source] serde_json::Error),
    #[error("daemon token is unavailable: {0}")]
    Token(#[source] std::io::Error),
    #[error("daemon token is empty")]
    EmptyToken,
    #[error("could not contact loopback daemon: {0}")]
    Connect(#[source] std::io::Error),
    #[error("daemon returned malformed HTTP: {0}")]
    Protocol(String),
    #[error("request input is invalid: {0}")]
    Input(#[source] std::io::Error),
}

struct Client {
    address: SocketAddr,
    token: String,
}

fn main() {
    let cli = Cli::parse();
    match execute(cli) {
        Ok(body) => println!("{body}"),
        Err(error) => {
            eprintln!("harness: {error}");
            std::process::exit(1);
        }
    }
}

fn execute(cli: Cli) -> Result<String, CliError> {
    let data_dir = cli.daemon_dir.unwrap_or(default_data_dir()?);
    let client = Client::from_data_dir(&data_dir)?;
    let response = match cli.command {
        Command::Approvals { run_id } => {
            client.request("GET", &format!("/v1/approvals?run_id={run_id}"), None)?
        }
        Command::Decision {
            approval_id,
            revision,
            approve,
        } => client.request(
            "POST",
            &format!("/v1/approvals/{approval_id}/decision"),
            Some(serde_json::json!({"revision":revision,"approved":approve})),
        )?,
        Command::Execute {
            run_id,
            python,
            test_commands,
            concurrency,
        } => {
            let commands = std::fs::read_to_string(test_commands).map_err(CliError::Input)?;
            let commands: Vec<Vec<String>> =
                serde_json::from_str(&commands).map_err(CliError::DiscoveryJson)?;
            client.request("POST", &format!("/v1/runs/{run_id}/execute"), Some(serde_json::json!({"python":python, "allowed_test_commands":commands, "concurrency":concurrency})))?
        }
        Command::Init {
            repo,
            name,
            trust_local,
        } => client.request(
            "POST",
            "/v1/projects",
            Some(serde_json::json!({"name": name, "repo_path": repo, "trusted_local":trust_local})),
        )?,
        Command::Plan { run_id, file } => {
            let plan = std::fs::read_to_string(file).map_err(CliError::Input)?;
            let plan: serde_json::Value =
                serde_json::from_str(&plan).map_err(CliError::DiscoveryJson)?;
            client.request(
                "POST",
                "/v1/plans",
                Some(serde_json::json!({"run_id": run_id, "plan": plan})),
            )?
        }
        Command::Approve { run_id, version } => client.request(
            "POST",
            &format!("/v1/plans/{run_id}/{version}/approve"),
            None,
        )?,
        Command::Run { project_id } => client.request(
            "POST",
            "/v1/runs",
            Some(serde_json::json!({"project_id": project_id})),
        )?,
        Command::Status { run_id } => client.request("GET", &format!("/v1/runs/{run_id}"), None)?,
        Command::Task { run_id } => {
            client.request("GET", &format!("/v1/runs/{run_id}/tasks"), None)?
        }
        Command::Priority {
            run_id,
            task_id,
            revision,
            manual_priority,
        } => client.request(
            "PATCH",
            &format!("/v1/tasks/{run_id}/{task_id}/priority"),
            Some(serde_json::json!({
                "revision": revision,
                "manual_priority": manual_priority,
            })),
        )?,
        Command::Pause { run_id } => {
            client.request("POST", &format!("/v1/runs/{run_id}/pause"), None)?
        }
        Command::Resume { run_id } => {
            client.request("POST", &format!("/v1/runs/{run_id}/resume"), None)?
        }
        Command::Logs {
            run_id,
            after,
            limit,
        } => client.request(
            "GET",
            &format!("/v1/events?run_id={run_id}&after={after}&limit={limit}"),
            None,
        )?,
    };
    if cli.json {
        return Ok(response);
    }
    let value: serde_json::Value =
        serde_json::from_str(&response).map_err(CliError::DiscoveryJson)?;
    serde_json::to_string_pretty(&value).map_err(CliError::DiscoveryJson)
}

impl Client {
    fn from_data_dir(data_dir: &std::path::Path) -> Result<Self, CliError> {
        let metadata =
            std::fs::read_to_string(data_dir.join("daemon.json")).map_err(CliError::Discovery)?;
        let metadata: serde_json::Value =
            serde_json::from_str(&metadata).map_err(CliError::DiscoveryJson)?;
        let port = metadata
            .get("port")
            .and_then(serde_json::Value::as_u64)
            .and_then(|port| u16::try_from(port).ok())
            .ok_or_else(|| CliError::Protocol("missing daemon port".into()))?;
        let token = std::fs::read_to_string(data_dir.join("api-token"))
            .map_err(CliError::Token)?
            .trim()
            .to_owned();
        if token.is_empty() {
            return Err(CliError::EmptyToken);
        }
        Ok(Self {
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            token,
        })
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<String, CliError> {
        let body = body.map(|body| body.to_string()).unwrap_or_default();
        let mut stream = TcpStream::connect(self.address).map_err(CliError::Connect)?;
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.token,
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .map_err(CliError::Connect)?;
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .map_err(CliError::Connect)?;
        let (head, body) = response
            .split_once("\r\n\r\n")
            .ok_or_else(|| CliError::Protocol("response lacks headers".into()))?;
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .ok_or_else(|| CliError::Protocol("response lacks status".into()))?;
        if !status.starts_with('2') {
            return Err(CliError::Protocol(format!("HTTP {status}: {body}")));
        }
        Ok(body.to_owned())
    }
}

fn default_data_dir() -> Result<PathBuf, CliError> {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .ok_or_else(|| CliError::Protocol("home directory unavailable".into()))?;
    Ok(PathBuf::from(base).join("AIHarness"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn help_mentions_workflows() {
        let help = Cli::command().render_help().to_string();
        assert!(help.contains("init"));
        assert!(help.contains("priority"));
        assert!(help.contains("logs"));
    }
}
