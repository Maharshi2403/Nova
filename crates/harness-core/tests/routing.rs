use harness_core::domain::{Complexity, Risk};
use harness_core::registry::ModelRegistry;
use harness_core::routing::{RoutingPolicy, route};

fn registry() -> ModelRegistry {
    ModelRegistry::from_json(
        r#"{"models":[
          {"id":"low","provider":"mock","model_id":"low","supported_tools":[],"structured_output":true,"context_window":10,"max_output_tokens":1,"input_usd_per_million_tokens":0.0,"output_usd_per_million_tokens":0.0,"available":true,"privacy":"local"},
          {"id":"medium","provider":"mock","model_id":"medium","supported_tools":[],"structured_output":true,"context_window":10,"max_output_tokens":1,"input_usd_per_million_tokens":0.0,"output_usd_per_million_tokens":0.0,"available":true,"privacy":"local"},
          {"id":"high","provider":"mock","model_id":"high","supported_tools":[],"structured_output":true,"context_window":10,"max_output_tokens":1,"input_usd_per_million_tokens":0.0,"output_usd_per_million_tokens":0.0,"available":true,"privacy":"local"}
        ]}"#,
    )
    .unwrap()
}

fn policy() -> RoutingPolicy {
    RoutingPolicy {
        low_model: "low".into(),
        medium_model: "medium".into(),
        high_model: "high".into(),
    }
}

#[test]
fn cheap_low_risk_route() {
    let result = route(&registry(), &policy(), Complexity::Low, Risk::Low).unwrap();
    assert_eq!(result.model_id, "low");
    assert_eq!(result.reason, "low_complexity_low_risk");
}

#[test]
fn high_risk_needs_capable_model() {
    let result = route(&registry(), &policy(), Complexity::Low, Risk::High).unwrap();
    assert_eq!(result.model_id, "high");
}

#[test]
fn route_reproducible() {
    let registry = registry();
    assert_eq!(
        route(&registry, &policy(), Complexity::Medium, Risk::Medium),
        route(&registry, &policy(), Complexity::Medium, Risk::Medium)
    );
}
