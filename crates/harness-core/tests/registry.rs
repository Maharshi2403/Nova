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

#[test]
fn optional_cache_prices_roundtrip_and_validate() {
    let original = ModelRegistry::from_json(VALID).unwrap();
    assert_eq!(original.models[0].cached_input_usd_per_million_tokens, None);
    let mut value: serde_json::Value = serde_json::from_str(VALID).unwrap();
    value["models"][0]["cached_input_usd_per_million_tokens"] = 0.1.into();
    value["models"][0]["cache_write_usd_per_million_tokens"] = 1.25.into();
    let parsed = ModelRegistry::from_json(&value.to_string()).unwrap();
    assert_eq!(
        parsed.models[0].cache_write_usd_per_million_tokens,
        Some(1.25)
    );
    assert_eq!(
        serde_json::to_value(parsed).unwrap()["models"][0]["cached_input_usd_per_million_tokens"],
        0.1
    );
    value["models"][0]["cache_write_usd_per_million_tokens"] = (-1.0).into();
    assert!(ModelRegistry::from_json(&value.to_string()).is_err());
}
