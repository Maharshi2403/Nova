use std::fs;
use std::path::Path;

use harness_core::domain::*;
use jsonschema::Validator;

fn spec_json() -> serde_json::Value {
    serde_json::json!({
        "id": "task-001",
        "title": "Add order endpoint validation",
        "description": "Implement 400 response when quantity <= 0",
        "complexity": "low",
        "risk": "low",
        "agent_priority": 60,
        "manual_priority": null,
        "dependencies": [],
        "candidate_paths": ["api/orders.py", "tests/test_orders.py"],
        "acceptance": [
            {"kind": "test", "command": ["python", "-m", "pytest", "-q", "tests/test_orders.py"]},
            {"kind": "assertion", "description": "quantity <= 0 yields HTTP 400"}
        ],
        "limits": {"max_tool_calls": 20, "max_output_tokens": 3000, "timeout_seconds": 180, "cost_cap_usd": 0.20}
    })
}

fn spec() -> TaskSpec {
    serde_json::from_value(spec_json()).unwrap()
}

fn run_id() -> RunId {
    RunId::new("run-001").unwrap()
}

#[test]
fn task_rejects_empty_title() {
    for title in ["", "   ", "ab"] {
        let mut s = spec();
        s.title = title.into();
        assert!(Task::new(run_id(), s).is_err(), "title {title:?}");
    }
}

#[test]
fn priority_validates_0_to_100() {
    assert!(Priority::new(0).is_ok());
    assert_eq!(Priority::new(100).unwrap().get(), 100);
    for bad in [-1, 101, 256, i64::MAX] {
        assert_eq!(
            Priority::new(bad),
            Err(DomainError::PriorityOutOfRange(bad))
        );
    }
    let mut v = spec_json();
    v["agent_priority"] = 101.into();
    assert!(serde_json::from_value::<TaskSpec>(v).is_err());
}

#[test]
fn task_json_round_trip() {
    let task = Task::new(run_id(), spec()).unwrap();
    assert_eq!(task.state, TaskState::Proposed);
    assert_eq!(task.revision, 0);
    let text = serde_json::to_string(&task).unwrap();
    let back: Task = serde_json::from_str(&text).unwrap();
    assert_eq!(back, task);
    // The spec serializes back to the contract shape.
    assert_eq!(serde_json::to_value(&task.spec).unwrap(), spec_json());
}

#[test]
fn spec_conforms_to_task_schema() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/task.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let v: Validator = jsonschema::validator_for(&schema).unwrap();
    assert!(v.is_valid(&serde_json::to_value(spec()).unwrap()));
}

#[test]
fn task_requires_acceptance_and_valid_limits() {
    let mut s = spec();
    s.acceptance.clear();
    assert_eq!(
        Task::new(run_id(), s).unwrap_err(),
        DomainError::NoAcceptance
    );

    let mut s = spec();
    s.acceptance = vec![Acceptance::Test { command: vec![] }];
    assert!(Task::new(run_id(), s).is_err());

    let mut s = spec();
    s.limits.timeout_seconds = 0;
    assert!(Task::new(run_id(), s).is_err());

    let mut s = spec();
    s.limits.cost_cap_usd = f64::NAN;
    assert!(Task::new(run_id(), s).is_err());
}

#[test]
fn ids_reject_empty_and_oversized() {
    assert!(TaskId::new("").is_err());
    assert!(TaskId::new("  ").is_err());
    assert!(TaskId::new("x".repeat(129)).is_err());
    assert!(serde_json::from_str::<TaskId>("\"\"").is_err());
}

#[test]
fn unknown_fields_and_states_rejected() {
    let mut v = spec_json();
    v["surprise"] = true.into();
    assert!(serde_json::from_value::<TaskSpec>(v).is_err());
    assert!(serde_json::from_str::<TaskState>("\"exploded\"").is_err());
}

#[test]
fn terminal_states() {
    for s in [
        TaskState::Completed,
        TaskState::Failed,
        TaskState::Cancelled,
    ] {
        assert!(s.is_terminal());
    }
    assert!(!TaskState::Blocked.is_terminal());
}

#[test]
fn project_and_run_construct() {
    let p = Project::new(ProjectId::new("proj").unwrap(), "Demo", "C:/repos/demo").unwrap();
    assert!(Project::new(p.id.clone(), "", "x").is_err());
    let r = Run::new(run_id(), p.id);
    assert_eq!(r.state, RunState::Planning);
}
