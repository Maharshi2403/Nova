//! Pure task dependency graph validation and stable topological ordering.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::domain::TaskId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DagError {
    DuplicateId(TaskId),
    SelfDependency(TaskId),
    MissingDependency { task: TaskId, missing: TaskId },
    Cycle(Vec<TaskId>),
}

impl std::fmt::Display for DagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateId(t) => write!(f, "duplicate task id {t}"),
            Self::SelfDependency(t) => write!(f, "task {t} depends on itself"),
            Self::MissingDependency { task, missing } => {
                write!(f, "task {task} depends on unknown task {missing}")
            }
            Self::Cycle(ids) => {
                let names: Vec<&str> = ids.iter().map(TaskId::as_str).collect();
                write!(f, "dependency cycle among: {}", names.join(", "))
            }
        }
    }
}

impl std::error::Error for DagError {}

/// Validate `(task, dependencies)` nodes and return a stable topological order.
///
/// Stability: among tasks that are simultaneously ready, the one that appears
/// earliest in the input comes first, so equal inputs always give equal output.
pub fn topological_order(nodes: &[(TaskId, Vec<TaskId>)]) -> Result<Vec<TaskId>, DagError> {
    let mut position: HashMap<&TaskId, usize> = HashMap::new();
    for (i, (id, _)) in nodes.iter().enumerate() {
        if position.insert(id, i).is_some() {
            return Err(DagError::DuplicateId(id.clone()));
        }
    }

    let mut remaining: Vec<usize> = vec![0; nodes.len()];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (i, (id, deps)) in nodes.iter().enumerate() {
        let unique: BTreeSet<&TaskId> = deps.iter().collect();
        for dep in unique {
            if dep == id {
                return Err(DagError::SelfDependency(id.clone()));
            }
            let Some(&j) = position.get(dep) else {
                return Err(DagError::MissingDependency {
                    task: id.clone(),
                    missing: dep.clone(),
                });
            };
            remaining[i] += 1;
            dependents[j].push(i);
        }
    }

    // BTreeMap keyed by input position gives a deterministic ready queue.
    let mut ready: BTreeMap<usize, ()> = remaining
        .iter()
        .enumerate()
        .filter(|(_, n)| **n == 0)
        .map(|(i, _)| (i, ()))
        .collect();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some((i, _)) = ready.pop_first() {
        order.push(nodes[i].0.clone());
        for &d in &dependents[i] {
            remaining[d] -= 1;
            if remaining[d] == 0 {
                ready.insert(d, ());
            }
        }
    }

    if order.len() != nodes.len() {
        let done: HashSet<&TaskId> = order.iter().collect();
        let stuck = nodes
            .iter()
            .map(|(id, _)| id)
            .filter(|id| !done.contains(id))
            .cloned()
            .collect();
        return Err(DagError::Cycle(stuck));
    }
    Ok(order)
}
