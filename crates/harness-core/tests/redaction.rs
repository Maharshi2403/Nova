use harness_core::redaction::*;

#[test]
fn secret_redacted_in_error_and_log_capped() {
    register_secret("opaque-runtime-credential-12345");
    let result = redact(
        "Authorization: Bearer abc123xyz\napi_key='private-value' password=another-secret sk-proj-abcdefgh12345 opaque-runtime-credential-12345",
    );
    for secret in [
        "abc123xyz",
        "private-value",
        "another-secret",
        "abcdefgh12345",
        "opaque-runtime-credential-12345",
    ] {
        assert!(!result.contains(secret));
    }
    assert!(redact(&"🔒".repeat(50000)).len() <= MAX_LOG_BYTES);
}

#[test]
fn private_prompt_never_in_artifacts() {
    let mut artifact = serde_json::json!({"prompt":"private full document", "stdout":"api_key=private-123456", "nested":[{"authorization":"Bearer other-secret"}], "exit_code":0});
    redact_json(&mut artifact);
    let text = artifact.to_string();
    assert!(!text.contains("private"));
    assert!(!text.contains("other-secret"));
    assert_eq!(artifact["exit_code"], 0);
}
