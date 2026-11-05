//! In-process resource claims for the single daemon. Durable leases remain authoritative.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct ResourceLocks(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    next: u64,
    claims: BTreeMap<u64, Claim>,
}
struct Claim {
    project: String,
    paths: Vec<String>,
    exclusive: bool,
}

pub struct ResourceGuard {
    locks: ResourceLocks,
    id: u64,
}

impl Drop for ResourceGuard {
    fn drop(&mut self) {
        self.locks
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .claims
            .remove(&self.id);
    }
}

fn normalize(path: &str) -> Option<String> {
    if path.is_empty() || path.starts_with(['/', '\\']) || path.contains([':', '\0']) {
        return None;
    }
    let normalized = path.replace('\\', "/");
    let parts: Vec<_> = normalized.split('/').filter(|p| *p != ".").collect();
    if parts.iter().any(|p| p.is_empty() || *p == "..") || parts.is_empty() {
        return None;
    }
    Some(parts.join("/").to_lowercase())
}

fn overlaps(a: &str, b: &str) -> bool {
    a == b
        || a.strip_prefix(b).is_some_and(|v| v.starts_with('/'))
        || b.strip_prefix(a).is_some_and(|v| v.starts_with('/'))
}

impl ResourceLocks {
    /// Invalid or empty write scope fails closed. No filesystem access is performed;
    /// the broker must separately enforce canonical paths and assigned capabilities.
    pub fn try_acquire(
        &self,
        project: &str,
        paths: &[String],
        exclusive: bool,
    ) -> Option<ResourceGuard> {
        if project.is_empty() || (!exclusive && paths.is_empty()) {
            return None;
        }
        let paths = paths
            .iter()
            .map(|p| normalize(p))
            .collect::<Option<Vec<_>>>()?;
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.claims.values().any(|c| {
            c.project == project
                && (exclusive
                    || c.exclusive
                    || paths.iter().any(|p| c.paths.iter().any(|q| overlaps(p, q))))
        }) {
            return None;
        }
        state.next = state.next.checked_add(1)?;
        let id = state.next;
        state.claims.insert(
            id,
            Claim {
                project: project.into(),
                paths,
                exclusive,
            },
        );
        Some(ResourceGuard {
            locks: self.clone(),
            id,
        })
    }
}
