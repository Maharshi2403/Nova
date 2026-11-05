//! Typed risky operations. Broad command prefixes never authorize these implicitly.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Command,
    Destructive,
    Network,
    Push,
    Deploy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub operation: Operation,
    pub argv: Vec<String>,
    pub cwd: String,
}

pub fn classify(argv: &[String]) -> Option<Operation> {
    let program = argv
        .first()?
        .rsplit(['/', '\\'])
        .next()?
        .to_ascii_lowercase();
    let program = program.trim_end_matches(".exe");
    if program == "git" && argv.iter().any(|v| v == "push") {
        return Some(Operation::Push);
    }
    if matches!(program, "rm" | "rmdir" | "del" | "format" | "remove-item")
        || (program == "git" && argv.iter().any(|v| matches!(v.as_str(), "reset" | "clean")))
    {
        return Some(Operation::Destructive);
    }
    if matches!(program, "curl" | "wget" | "ssh" | "scp" | "pip" | "pip3")
        || argv
            .iter()
            .any(|v| matches!(v.as_str(), "install" | "fetch" | "pull" | "clone"))
    {
        return Some(Operation::Network);
    }
    if matches!(
        program,
        "terraform" | "kubectl" | "vercel" | "netlify" | "aws" | "az"
    ) {
        return Some(Operation::Deploy);
    }
    if matches!(
        program,
        "sh" | "bash" | "zsh" | "cmd" | "powershell" | "pwsh"
    ) {
        return Some(Operation::Command);
    }
    None
}
