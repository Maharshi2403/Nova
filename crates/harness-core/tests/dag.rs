use harness_core::dag::{DagError, topological_order};
use harness_core::domain::TaskId;

fn id(s: &str) -> TaskId {
    TaskId::new(s).unwrap()
}

fn node(t: &str, deps: &[&str]) -> (TaskId, Vec<TaskId>) {
    (id(t), deps.iter().map(|d| id(d)).collect())
}

fn names(v: &[TaskId]) -> Vec<&str> {
    v.iter().map(TaskId::as_str).collect()
}

#[test]
fn dag_cycle_rejected() {
    let nodes = [
        node("a", &["c"]),
        node("b", &["a"]),
        node("c", &["b"]),
        node("d", &[]),
    ];
    match topological_order(&nodes) {
        Err(DagError::Cycle(ids)) => assert_eq!(names(&ids), ["a", "b", "c"]),
        other => panic!("expected cycle, got {other:?}"),
    }
}

#[test]
fn self_dependency_rejected() {
    let nodes = [node("a", &["a"])];
    assert_eq!(
        topological_order(&nodes),
        Err(DagError::SelfDependency(id("a")))
    );
}

#[test]
fn missing_dependency_rejected() {
    let nodes = [node("a", &["ghost"])];
    assert_eq!(
        topological_order(&nodes),
        Err(DagError::MissingDependency {
            task: id("a"),
            missing: id("ghost")
        })
    );
}

#[test]
fn duplicate_id_rejected() {
    let nodes = [node("a", &[]), node("a", &[])];
    assert_eq!(
        topological_order(&nodes),
        Err(DagError::DuplicateId(id("a")))
    );
}

#[test]
fn topological_order_stable() {
    // Input order is deliberately not dependency order.
    let nodes = [
        node("build", &["core", "api"]),
        node("api", &["core"]),
        node("docs", &[]),
        node("core", &[]),
    ];
    let first = topological_order(&nodes).unwrap();
    assert_eq!(names(&first), ["docs", "core", "api", "build"]);
    for _ in 0..20 {
        assert_eq!(topological_order(&nodes).unwrap(), first);
    }
}

#[test]
fn dependencies_always_precede_dependents() {
    let nodes = [
        node("e", &["d", "c"]),
        node("d", &["b"]),
        node("c", &["a", "b"]),
        node("b", &["a"]),
        node("a", &[]),
    ];
    let order = topological_order(&nodes).unwrap();
    let pos = |t: &str| order.iter().position(|x| x.as_str() == t).unwrap();
    for (t, deps) in &nodes {
        for d in deps {
            assert!(pos(d.as_str()) < pos(t.as_str()));
        }
    }
}

#[test]
fn empty_and_duplicate_dependency_entries_ok() {
    assert!(topological_order(&[]).unwrap().is_empty());
    let nodes = [node("a", &[]), node("b", &["a", "a"])];
    assert_eq!(names(&topological_order(&nodes).unwrap()), ["a", "b"]);
}
