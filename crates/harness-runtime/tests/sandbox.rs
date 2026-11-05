use harness_runtime::sandbox::*;
#[test]
fn untrusted_execution_denied_without_real_backend() {
    assert!(authorize(Trust::Untrusted, None).is_err());
    assert!(authorize(Trust::TrustedLocal, None).is_ok());
}
struct Incomplete;
impl SandboxAdapter for Incomplete {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            filesystem: true,
            network: false,
            process_tree: true,
        }
    }
    fn configure(&self, _: &mut tokio::process::Command) -> Result<(), String> {
        Err("not implemented".into())
    }
}
#[test]
fn incomplete_isolation_capability_rejected() {
    assert!(authorize(Trust::Untrusted, Some(&Incomplete)).is_err());
}
