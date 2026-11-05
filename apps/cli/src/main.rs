use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
mod desktop;
mod providers;
mod startup;

/// Nova — your local coding-agent harness. Run `nova` to get started.
#[derive(Parser, Debug)]
#[command(name = "nova", version, about)]
struct Cli {
    /// Print the API response as stable JSON.
    #[arg(long)]
    json: bool,
    /// Nova's state directory (defaults to the per-user Nova directory).
    #[arg(long, global = true)]
    daemon_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    #[command(name = "__desktop-request", hide = true)]
    DesktopRequest,
    Providers {
        #[command(subcommand)]
        command: providers::ProviderCommand,
    },
    Routes {
        #[command(subcommand)]
        command: providers::RouteCommand,
    },
    Models {
        #[command(subcommand)]
        command: providers::ModelCommand,
    },
    /// Start Nova in the background, or connect to the running service.
    Start,
    /// Stop Nova safely when no runs are active.
    Stop,
    #[command(name = "__serve", hide = true)]
    Serve,
    Approvals {
        run_id: String,
    },
    Decision {
        approval_id: String,
        revision: i64,
        #[arg(long)]
        approve: bool,
    },
    /// Execute an approved run (offline mock by default; explicit live opt-in).
    Execute {
        run_id: String,
        #[arg(long)]
        python: Option<PathBuf>,
        /// JSON array of exact argv arrays approved for verifier execution.
        #[arg(long)]
        test_commands: PathBuf,
        #[arg(long, default_value_t = 2)]
        concurrency: usize,
        #[arg(long)]
        live: bool,
        #[arg(long, requires = "live")]
        model_config: Option<PathBuf>,
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
    Cancel {
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
    #[error("could not start Nova: {0}")]
    Startup(String),
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
    #[error("request rejected: {0}")]
    Rejected(String),
    #[error("desktop request failed: {0}")]
    Desktop(String),
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
    if matches!(cli.command, Some(Command::Serve)) {
        let result = cli
            .daemon_dir
            .map(Ok)
            .unwrap_or_else(default_data_dir)
            .and_then(|directory| {
                let runtime =
                    tokio::runtime::Runtime::new().map_err(|e| CliError::Startup(e.to_string()))?;
                runtime.block_on(async {
                    let daemon =
                        harness_daemon::start(directory, harness_daemon::loopback_ephemeral())
                            .await
                            .map_err(|e| CliError::Startup(e.to_string()))?;
                    daemon.run_until_shutdown().await;
                    Ok(())
                })
            });
        if let Err(error) = result {
            eprintln!("nova: {error}");
            std::process::exit(1);
        }
        return;
    }
    match execute(cli) {
        Ok(body) => println!("{body}"),
        Err(error) => {
            eprintln!("nova: {error}");
            std::process::exit(1);
        }
    }
}

fn execute(cli: Cli) -> Result<String, CliError> {
    let data_dir = cli.daemon_dir.map(Ok).unwrap_or_else(default_data_dir)?;
    let command = cli.command.unwrap_or(Command::Start);
    if matches!(command, Command::DesktopRequest) {
        return desktop::request(&data_dir);
    }
    if matches!(command, Command::Start) {
        return startup::start(&data_dir, cli.json);
    }
    if matches!(command, Command::Stop) {
        return startup::stop(&data_dir, cli.json);
    }
    let command = match command {
        Command::Providers { command } => {
            return providers::providers(&data_dir, command).map_err(CliError::Startup);
        }
        Command::Routes { command } => {
            return providers::routes(&data_dir, command).map_err(CliError::Startup);
        }
        Command::Models { command } => {
            return providers::models(&data_dir, command).map_err(CliError::Startup);
        }
        other => other,
    };
    let client = Client::from_data_dir(&data_dir)?;
    let response = match command {
        Command::DesktopRequest
        | Command::Routes { .. }
        | Command::Providers { .. }
        | Command::Models { .. }
        | Command::Start
        | Command::Stop
        | Command::Serve => {
            unreachable!("lifecycle handled before API commands")
        }
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
            live,
            model_config,
        } => {
            let python = harness_runtime::provider_config::ProviderStore::new(&data_dir)
                .python(python)
                .map_err(CliError::Startup)?;
            let commands = std::fs::read_to_string(test_commands).map_err(CliError::Input)?;
            let commands: Vec<Vec<String>> =
                serde_json::from_str(&commands).map_err(CliError::DiscoveryJson)?;
            let config = model_config
                .map(|p| {
                    std::fs::read_to_string(p)
                        .map_err(CliError::Input)
                        .and_then(|s| {
                            serde_json::from_str::<serde_json::Value>(&s)
                                .map_err(CliError::DiscoveryJson)
                        })
                })
                .transpose()?;
            let config = if live && config.is_none() {
                Some(
                    serde_json::to_value(
                        harness_runtime::provider_config::ProviderStore::new(&data_dir)
                            .load()
                            .map_err(CliError::Startup)?
                            .live_config()
                            .map_err(CliError::Startup)?,
                    )
                    .map_err(CliError::DiscoveryJson)?,
                )
            } else {
                config
            };
            client.request("POST", &format!("/v1/runs/{run_id}/execute"), Some(serde_json::json!({"python":python, "allowed_test_commands":commands, "concurrency":concurrency,"live":live,"model_config":config})))?
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
        Command::Cancel { run_id } => {
            client.request("POST", &format!("/v1/runs/{run_id}/cancel"), None)?
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
        self.request_with_timeout(method, path, body, std::time::Duration::from_secs(30))
    }
    fn request_with_timeout(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
        timeout: std::time::Duration,
    ) -> Result<String, CliError> {
        let body = body.map(|body| body.to_string()).unwrap_or_default();
        let mut stream =
            TcpStream::connect_timeout(&self.address, std::time::Duration::from_secs(1))
                .map_err(CliError::Connect)?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(CliError::Connect)?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(CliError::Connect)?;
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
            let message = serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|value| value["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("HTTP {status}"));
            return Err(CliError::Rejected(message));
        }
        Ok(body.to_owned())
    }
}

fn default_data_dir() -> Result<PathBuf, CliError> {
    harness_daemon::default_data_dir().map_err(|e| CliError::Startup(e.to_string()))
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
