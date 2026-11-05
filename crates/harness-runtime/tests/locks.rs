use harness_runtime::locks::ResourceLocks;

#[test]
fn overlapping_writes_serialized_and_released() {
    let locks = ResourceLocks::default();
    let a = locks.try_acquire("p", &["src".into()], false).unwrap();
    assert!(
        locks
            .try_acquire("p", &["src/a.rs".into()], false)
            .is_none()
    );
    assert!(
        locks
            .try_acquire("p", &["SRC\\a.rs".into()], false)
            .is_none()
    );
    assert!(locks.try_acquire("p", &[], true).is_none());
    drop(a);
    assert!(
        locks
            .try_acquire("p", &["src/a.rs".into()], false)
            .is_some()
    );
}

#[test]
fn independent_paths_allowed_and_exclusive_is_project_scoped() {
    let locks = ResourceLocks::default();
    let _a = locks.try_acquire("p", &["a".into()], false).unwrap();
    let _b = locks.try_acquire("p", &["ab".into()], false).unwrap();
    assert!(locks.try_acquire("q", &[], true).is_some());
    for bad in ["../x", "C:x", "/x", "a//b", ""] {
        assert!(locks.try_acquire("p", &[bad.into()], false).is_none());
    }
}

#[test]
fn cleanup_on_unwind() {
    let locks = ResourceLocks::default();
    let clone = locks.clone();
    let _ = std::panic::catch_unwind(move || {
        let _guard = clone.try_acquire("p", &[], true).unwrap();
        panic!("injected error");
    });
    assert!(locks.try_acquire("p", &[], true).is_some());
}
