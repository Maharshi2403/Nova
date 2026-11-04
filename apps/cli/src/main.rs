use clap::Parser;

/// AI Harness: local-first coding-agent runtime (CLI entrypoint).
#[derive(Parser, Debug)]
#[command(name = "harness", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    println!("harness: no commands implemented yet (see `harness --help`)");
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
    fn help_mentions_harness() {
        let help = Cli::command().render_help().to_string();
        assert!(help.contains("AI Harness"));
    }
}
