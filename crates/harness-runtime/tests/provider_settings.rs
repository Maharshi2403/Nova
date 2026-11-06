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
#[test]
fn saved_routes_generate_real_registry_and_credentials_stay_scoped() {
    let mut settings = ProviderSettings::default();
    for (id, kind, base) in [
        ("code", ProviderKind::Openai, "https://api.openai.com/v1"),
        (
            "review",
            ProviderKind::Anthropic,
            "https://api.anthropic.com/v1",
        ),
    ] {
        let profile = ProviderProfile {
            id: id.into(),
            kind,
            base_url: base.into(),
            credential: CredentialRef::Environment {
                variable: "NO_KEY_READ_IN_THIS_TEST".into(),
            },
        };
        let model_id = if kind == ProviderKind::Openai {
            "gpt-6.1-sol"
        } else {
            "claude-sonnet-5-5"
        };
        settings.models.push(
            harness_runtime::catalog::normalize(&profile, &serde_json::json!({"id":model_id}))
                .unwrap(),
        );
        settings.providers.push(profile);
    }
    settings
        .routes
        .insert("review".into(), "review/claude-sonnet-5-5".into());
    let config = settings.live_config().unwrap();
    assert_eq!(
        config.roles[&harness_core::domain::UseCase::Review].high_model,
        "review/claude-sonnet-5-5"
    );
    let env = settings.providers[1].adapter_env("fixture-only");
    assert!(
        env.iter()
            .any(|(k, v)| k == "NOVA_PROVIDER_NAME" && v == "review")
    );
    assert!(!env.iter().any(|(k, _)| k == "OPENAI_API_KEY"));
    settings.models[1].input_usd_per_million_tokens = None;
    assert!(settings.live_config().is_err());
}
#[cfg(unix)]
#[test]
fn python_keeps_absolute_virtualenv_symlink_path() {
    let dir = tempfile::tempdir().unwrap();
    let venv = dir.path().join("runtime").join("bin");
    std::fs::create_dir_all(&venv).unwrap();
    let link = venv.join("python");
    std::os::unix::fs::symlink(std::env::current_exe().unwrap(), &link).unwrap();
    let resolved = ProviderStore::new(dir.path())
        .python(Some(link.clone()))
        .unwrap();
    assert_eq!(resolved, link);
}

#[test]
fn automatic_tiers_select_workers_across_providers_and_escalate_risk() {
    use harness_core::domain::{Complexity, Risk, RunId, Task, UseCase};
    let mut settings = ProviderSettings::default();
    for (provider, tier, price) in [
        ("small", "low", 0.1),
        ("general", "medium", 1.0),
        ("reasoning", "high", 5.0),
    ] {
        settings.providers.push(ProviderProfile {
            id: provider.into(),
            kind: ProviderKind::Compatible,
            base_url: "https://example.test/v1".into(),
            credential: CredentialRef::Environment {
                variable: "UNUSED_TEST_CREDENTIAL".into(),
            },
        });
        settings.models.push(SavedModel {
            id: format!("{provider}/worker"),
            provider: provider.into(),
            model_id: "worker".into(),
            display_name: provider.into(),
            context_window: Some(32000),
            max_output_tokens: Some(4096),
            input_usd_per_million_tokens: Some(price),
            output_usd_per_million_tokens: Some(price * 2.0),
            structured_output: true,
            tier: tier.into(),
            metadata_source: "test fixture".into(),
        });
    }
    let mut plan: harness_core::plan::PlanProposal = serde_json::from_str(include_str!(
        "../../../tests/fixtures/plans/valid_plan.json"
    ))
    .unwrap();
    let mut task = Task::new(RunId::new("routing-test").unwrap(), plan.tasks.remove(0)).unwrap();
    task.spec.limits.cost_cap_usd = 1.0;
    let config = settings.live_config().unwrap();
    for (complexity, risk, expected) in [
        (Complexity::Low, Risk::Low, "small"),
        (Complexity::Medium, Risk::Low, "general"),
        (Complexity::High, Risk::Low, "reasoning"),
        (Complexity::Low, Risk::High, "reasoning"),
    ] {
        task.spec.complexity = complexity;
        task.spec.risk = risk;
        assert_eq!(config.select(&task).unwrap().provider, expected);
    }
    settings
        .routes
        .insert("review".into(), "general/worker".into());
    task.spec.use_case = UseCase::Review;
    assert_eq!(
        settings
            .live_config()
            .unwrap()
            .select(&task)
            .unwrap()
            .provider,
        "general"
    );
    // Incomplete metadata must never silently route a worker to this account.
    settings.models[1].structured_output = false;
    assert!(settings.live_config().is_err());
}
