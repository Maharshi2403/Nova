use clap::Parser;
use harness_daemon::{default_data_dir, loopback_ephemeral, start};

/// AI Harness supervisor daemon.
#[derive(Parser, Debug)]
#[command(name = "harnessd", version, about)]
struct Args {
    #[arg(long)]
    data_dir: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let data_dir = match args.data_dir.map(Ok).unwrap_or_else(default_data_dir) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("harnessd: {error}");
            std::process::exit(1);
        }
    };
    match start(data_dir, loopback_ephemeral()).await {
        Ok(daemon) => {
            eprintln!("harnessd listening on {}", daemon.address);
            daemon.run_until_shutdown().await;
        }
        Err(error) => {
            eprintln!("harnessd: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn args_definition_is_valid() {
        Args::command().debug_assert();
    }
}
