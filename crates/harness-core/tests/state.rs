use harness_core::domain::TaskState::{self, *};
use harness_core::state::{ALLOWED, TransitionError, is_allowed, transition};

const ALL: [TaskState; 13] = [
    Proposed,
    Pending,
    Ready,
    Leased,
    Running,
    Verifying,
    Reviewing,
    Integrating,
    RetryWait,
    Blocked,
    Completed,
    Failed,
    Cancelled,
];

#[test]
fn rejects_running_to_completed() {
    assert_eq!(
        transition(Running, Completed),
        Err(TransitionError::Illegal {
            from: Running,
            to: Completed
        })
    );
    // Completion is only reachable through integration.
    for from in ALL {
        assert_eq!(is_allowed(from, Completed), from == Integrating, "{from:?}");
    }
}

#[test]
fn rejects_terminal_reentry() {
    for from in [Completed, Failed, Cancelled] {
        for to in ALL {
            assert_eq!(
                transition(from, to),
                Err(TransitionError::TerminalState(from))
            );
        }
    }
}

#[test]
fn normal_path_is_allowed() {
    let path = [
        Proposed,
        Pending,
        Ready,
        Leased,
        Running,
        Verifying,
        Reviewing,
        Integrating,
        Completed,
    ];
    for w in path.windows(2) {
        assert_eq!(transition(w[0], w[1]), Ok(w[1]));
    }
}

#[test]
fn allowed_table_matches_contract() {
    let expect: &[(TaskState, &[TaskState])] = &[
        (Proposed, &[Pending, Cancelled]),
        (Pending, &[Ready, Blocked, Cancelled]),
        (Ready, &[Leased, Blocked, Cancelled]),
        (Leased, &[Running, RetryWait, Blocked, Cancelled]),
        (Running, &[Verifying, RetryWait, Blocked, Failed, Cancelled]),
        (Verifying, &[Reviewing, RetryWait, Blocked, Failed]),
        (Reviewing, &[Integrating, Blocked, Failed]),
        (Integrating, &[Completed, Blocked, RetryWait, Failed]),
        (RetryWait, &[Ready, Blocked, Failed, Cancelled]),
        (Blocked, &[Ready, Failed, Cancelled]),
        (Completed, &[]),
        (Failed, &[]),
        (Cancelled, &[]),
    ];
    for (from, tos) in expect {
        for to in ALL {
            assert_eq!(
                is_allowed(*from, to),
                tos.contains(&to),
                "{from:?} -> {to:?}"
            );
        }
    }
    let total: usize = expect.iter().map(|(_, t)| t.len()).sum();
    assert_eq!(total, ALLOWED.len());
}

#[test]
fn self_transitions_and_skips_rejected() {
    for s in ALL {
        assert!(transition(s, s).is_err(), "{s:?}");
    }
    assert!(transition(Ready, Running).is_err());
    assert!(transition(Pending, Leased).is_err());
    assert!(transition(Verifying, Integrating).is_err());
}
