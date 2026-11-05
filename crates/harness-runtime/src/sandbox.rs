//! Execution trust boundary. Worktrees do not isolate host resources.
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    Untrusted,
    TrustedLocal,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct Capabilities {
    pub filesystem: bool,
    pub network: bool,
    pub process_tree: bool,
}

/// A backend must actually configure the command, not merely advertise a label.
pub trait SandboxAdapter {
    fn capabilities(&self) -> Capabilities;
    fn configure(&self, command: &mut Command) -> Result<(), String>;
}

pub fn authorize(trust: Trust, backend: Option<&dyn SandboxAdapter>) -> Result<(), &'static str> {
    if trust == Trust::TrustedLocal {
        return Ok(());
    }
    let capabilities = backend
        .ok_or("untrusted execution requires an OS sandbox backend")?
        .capabilities();
    if !capabilities.filesystem || !capabilities.network || !capabilities.process_tree {
        return Err("sandbox does not provide required isolation capabilities");
    }
    Ok(())
}
