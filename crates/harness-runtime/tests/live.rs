use harness_core::{
    domain::{RunId, Task},
    messages::ToolRequest,
};
use harness_runtime::live::{LiveBroker, LiveConfig};
fn fixture() -> (Task, LiveConfig) {
    let mut plan: harness_core::plan::PlanProposal = serde_json::from_str(include_str!(
        "../../../tests/fixtures/plans/valid_plan.json"
    ))
    .unwrap();
    let mut task = Task::new(RunId::new("r").unwrap(), plan.tasks.remove(0)).unwrap();
    task.attempt = 1;
    let config = serde_json::from_value(serde_json::json!({"registry":{"models":[{"id":"configured","provider":"openai","model_id":"user-configured-model","supported_tools":["repo.patch","repo.read"],"structured_output":true,"context_window":64000,"max_output_tokens":1000,"input_usd_per_million_tokens":1,"output_usd_per_million_tokens":2,"available":true,"privacy":"private"}]},"routing":{"low_model":"configured","medium_model":"configured","high_model":"configured"},"run_cap_microdollars":1000000,"daily_cap_microdollars":2000000})).unwrap();
    (task, config)
}
fn request(model: &str, input: usize, output: u32) -> ToolRequest {
    ToolRequest {tool:"model.authorize".into(), arguments:serde_json::json!({"model_id":model,"input_byte_bound":input,"max_output_tokens":output,"retry_attempt":1}).as_object().unwrap().clone()}
}
#[tokio::test]
async fn live_grants_validate_route_budget_and_complete_request_without_network() {
    let (task, config) = fixture();
    let model = config.select(&task).unwrap();
    let mut broker = LiveBroker::new(
        model,
        &task,
        "missing-python".into(),
        "fixture-credential".into(),
    );
    assert!(!broker.authorize(&request("different-model", 4000, 1000)).ok);
    assert!(
        !broker
            .authorize(&request("user-configured-model", 1000000, 1000))
            .ok
    );
    assert!(
        broker
            .authorize(&request("user-configured-model", 4000, 1000))
            .ok
    );
    assert!(
        !broker
            .authorize(&request("user-configured-model", 4000, 1000))
            .ok
    );
    let (response, usage) = broker.complete(&request("other", 4000, 1000)).await;
    assert!(!response.ok);
    assert!(usage.is_none());
}
#[test]
fn live_registry_cannot_route_without_nonzero_budget_and_capability() {
    let (task, mut config) = fixture();
    config.run_cap_microdollars = 0;
    assert!(config.select(&task).is_err());
    config.run_cap_microdollars = 1;
    config.registry.models[0].provider = "unexpected-provider".into();
    assert!(config.select(&task).is_err());
}

#[tokio::test]
async fn credential_scoped_adapter_fails_closed_without_key_over_real_ipc() {
    let (task, config) = fixture();
    let python = harness_runtime::engine::default_python(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../intelligence/.venv"),
    );
    let mut broker = LiveBroker::new(config.select(&task).unwrap(), &task, python, String::new());
    assert!(
        broker
            .authorize(&request("user-configured-model", 4000, 1000))
            .ok
    );
    let response = broker.complete(&ToolRequest {tool:"model.complete".into(),arguments:serde_json::json!({"messages":[],"schema":{"type":"object"},"model_id":"user-configured-model","max_output_tokens":1000,"timeout_seconds":1}).as_object().unwrap().clone()}).await;
    assert!(!response.0.ok);
    assert_eq!(response.0.error.as_deref(), Some("permanent_provider"));
    assert!(response.1.unwrap().cost_usd.is_none());
}
