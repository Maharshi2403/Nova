use harness_core::domain::{RunState, TaskId, TaskState};
use harness_runtime::readiness::{RunGate, TaskView, ready_tasks};

fn id(s: &str) -> TaskId {
    TaskId::new(s).unwrap()
}

fn view(t: &str, state: TaskState, deps: &[&str]) -> TaskView {
    TaskView {
        id: id(t),
        state,
        dependencies: deps.iter().map(|d| id(d)).collect(),
        retry_not_before: None,
    }
}

const RUNNING: RunGate = RunGate {
    state: RunState::Running,
    plan_approved: true,
};

fn names(v: &[TaskId]) -> Vec<&str> {
    v.iter().map(TaskId::as_str).collect()
}

#[test]
fn only_unblocked_tasks_ready() {
    let tasks = [
        view("a", TaskState::Completed, &[]),
        view("b", TaskState::Pending, &["a"]),
        view("c", TaskState::Pending, &["b"]),
        view("d", TaskState::Pending, &["a", "x-running"]),
        view("x-running", TaskState::Running, &[]),
        view("e", TaskState::Ready, &[]),
        view("f", TaskState::Pending, &["ghost"]),
    ];
    assert_eq!(names(&ready_tasks(RUNNING, &tasks, 0)), ["b", "e"]);
}

#[test]
fn paused_run_dispatches_none() {
    let tasks = [
        view("a", TaskState::Ready, &[]),
        view("b", TaskState::Pending, &[]),
    ];
    for state in [
        RunState::Paused,
        RunState::Planning,
        RunState::AwaitingApproval,
        RunState::Completed,
        RunState::Failed,
        RunState::Cancelled,
    ] {
        let gate = RunGate {
            state,
            plan_approved: true,
        };
        assert!(ready_tasks(gate, &tasks, 0).is_empty(), "{state:?}");
    }
    assert_eq!(ready_tasks(RUNNING, &tasks, 0).len(), 2);
}

#[test]
fn unapproved_plan_dispatches_none() {
    let gate = RunGate {
        state: RunState::Running,
        plan_approved: false,
    };
    assert!(ready_tasks(gate, &[view("a", TaskState::Ready, &[])], 0).is_empty());
}

#[test]
fn retry_wait_respects_retry_time() {
    let mut t = view("a", TaskState::RetryWait, &[]);
    t.retry_not_before = Some(100);
    assert!(ready_tasks(RUNNING, std::slice::from_ref(&t), 99).is_empty());
    assert_eq!(ready_tasks(RUNNING, std::slice::from_ref(&t), 100).len(), 1);
    t.retry_not_before = None;
    assert_eq!(ready_tasks(RUNNING, &[t], 0).len(), 1);
}

#[test]
fn in_flight_blocked_and_terminal_tasks_never_ready() {
    for s in [
        TaskState::Proposed,
        TaskState::Leased,
        TaskState::Running,
        TaskState::Verifying,
        TaskState::Reviewing,
        TaskState::Integrating,
        TaskState::Blocked,
        TaskState::Completed,
        TaskState::Failed,
        TaskState::Cancelled,
    ] {
        assert!(
            ready_tasks(RUNNING, &[view("a", s, &[])], 0).is_empty(),
            "{s:?}"
        );
    }
}

#[test]
fn failed_or_cancelled_dependency_blocks_dependents() {
    for dep_state in [TaskState::Failed, TaskState::Cancelled, TaskState::Blocked] {
        let tasks = [
            view("a", dep_state, &[]),
            view("b", TaskState::Pending, &["a"]),
        ];
        assert!(ready_tasks(RUNNING, &tasks, 0).is_empty(), "{dep_state:?}");
    }
}
