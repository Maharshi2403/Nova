use harness_core::registry::{ModelRegistry, RegistryError};

const VALID: &str = r#"{
  "models": [{
    "id": "mock-coder",
    "provider": "mock",
    "model_id": "mock-coder-v1",
    "supported_tools": ["repo.read", "repo.patch"],
    "structured_output": true,
    "context_window": 8192,
    "max_output_tokens": 1024,
    "input_usd_per_million_tokens": 0.0,
    "output_usd_per_million_tokens": 0.0,
    "available": true,
    "privacy": "local"
  }]
}"#;

#[test]
fn invalid_registry_rejected() {
    let invalid = VALID.replace("\"context_window\": 8192", "\"context_window\": 0");
    assert!(matches!(
        ModelRegistry::from_json(&invalid),
        Err(RegistryError::Invalid(_))
    ));
}

#[test]
fn unsupported_capability_rejected() {
    let registry = ModelRegistry::from_json(VALID).unwrap();
    assert!(matches!(
        registry.require("mock-coder", Some("cmd.run"), true),
        Err(RegistryError::UnsupportedCapability { .. })
    ));
    assert!(
        registry
            .require("mock-coder", Some("repo.patch"), true)
            .is_ok()
    );
}
