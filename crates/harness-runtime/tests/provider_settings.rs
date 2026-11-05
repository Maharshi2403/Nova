use harness_runtime::provider_config::*;
#[test]
fn profiles_are_public_atomic_and_validated() {
    let dir = tempfile::tempdir().unwrap();
    let store = ProviderStore::new(dir.path());
    let profile = ProviderProfile {
        id: "claude".into(),
        kind: ProviderKind::Anthropic,
        base_url: "https://api.anthropic.com/v1".into(),
        credential: CredentialRef::Environment {
            variable: "NOVA_TEST_KEY".into(),
        },
    };
    store
        .update(|s| {
            s.providers.push(profile);
            Ok(())
        })
        .unwrap();
    assert_eq!(store.load().unwrap().providers.len(), 1);
    assert!(
        !std::fs::read_to_string(dir.path().join("providers.json"))
            .unwrap()
            .contains("secret-value")
    );
    assert!(
        store
            .update(|s| {
                s.providers[0].base_url = "http://evil.test".into();
                Ok(())
            })
            .is_err()
    );
    assert_eq!(
        store.load().unwrap().providers[0].base_url,
        "https://api.anthropic.com/v1"
    );
}
#[test]
fn unknown_catalog_metadata_is_never_free_or_ready() {
    let p = ProviderProfile {
        id: "custom".into(),
        kind: ProviderKind::Compatible,
        base_url: "https://example.com/v1".into(),
        credential: CredentialRef::Environment {
            variable: "CUSTOM_KEY".into(),
        },
    };
    let model =
        harness_runtime::catalog::normalize(&p, &serde_json::json!({"id":"unknown"})).unwrap();
    assert!(!model.ready());
    assert_eq!(model.input_usd_per_million_tokens, None);
    let model=harness_runtime::catalog::normalize(&p,&serde_json::json!({"id":"known","context_length":32000,"top_provider":{"max_completion_tokens":4096},"pricing":{"prompt":"0.000002","completion":"0.000010"},"supported_parameters":["structured_outputs"]})).unwrap();
    assert!(model.ready());
    assert_eq!(model.input_usd_per_million_tokens, Some(2.));
}
