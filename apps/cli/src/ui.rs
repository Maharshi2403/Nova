//! Launch the installed native UI with public paths only; credentials stay in stdin IPC.
use super::CliError;
use std::path::{Path, PathBuf};
fn candidates(executable: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(parent) = executable.parent() {
        paths.push(parent.join("Nova.app"));
        if let Some(root) = parent.parent() {
            paths.push(root.join("Nova.app"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join("Applications/Nova.app"));
    }
    paths.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist/Nova.app"));
    paths
}
pub(super) fn launch(directory: &Path, print_path: bool) -> Result<String, CliError> {
    if !cfg!(target_os = "macos") {
        return Err(CliError::Startup(
            "the desktop app currently supports macOS; use nova --help for CLI management".into(),
        ));
    }
    let executable = std::env::current_exe().map_err(|e| CliError::Startup(e.to_string()))?;
    let app=candidates(&executable).into_iter().find(|p|p.join("Contents/MacOS/Nova").is_file()).ok_or_else(||CliError::Startup("Nova.app is not installed. Install the desktop bundle in ~/Applications or build it with scripts/build_desktop.py.".into()))?;
    if print_path {
        return Ok(app.display().to_string());
    }
    super::startup::start(directory, true)?;
    let status = std::process::Command::new("/usr/bin/open")
        .args(["-n", "-a"])
        .arg(&app)
        .arg("--args")
        .arg("--daemon-dir")
        .arg(directory)
        .status()
        .map_err(|e| CliError::Startup(e.to_string()))?;
    if !status.success() {
        return Err(CliError::Startup("could not open Nova.app".into()));
    }
    Ok("Opened Nova desktop. Closing the window leaves your service running.".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundle_discovery_covers_extracted_release() {
        let paths = candidates(Path::new("/tmp/nova-release/bin/nova"));
        assert!(paths.contains(&PathBuf::from("/tmp/nova-release/Nova.app")));
    }
}
