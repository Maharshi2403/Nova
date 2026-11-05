//! Deterministic model selection from task complexity and risk.

use crate::domain::{Complexity, Risk};
use crate::registry::{ModelDefinition, ModelRegistry, RegistryError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingPolicy {
    pub low_model: String,
    pub medium_model: String,
    pub high_model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub model_id: String,
    pub reason: String,
}

pub fn route(
    registry: &ModelRegistry,
    policy: &RoutingPolicy,
    complexity: Complexity,
    risk: Risk,
) -> Result<Route, RegistryError> {
    let (selected, reason) = match (complexity, risk) {
        (_, Risk::High) => (&policy.high_model, "high_risk"),
        (Complexity::Low, Risk::Low) => (&policy.low_model, "low_complexity_low_risk"),
        (Complexity::Low | Complexity::Medium, _) => (&policy.medium_model, "medium_tier"),
        (Complexity::High, _) => (&policy.high_model, "high_complexity"),
    };
    let model = registry.require(selected, None, true)?;
    Ok(Route {
        model_id: model.id.clone(),
        reason: reason.into(),
    })
}

pub fn selected_model<'a>(
    registry: &'a ModelRegistry,
    route: &Route,
) -> Result<&'a ModelDefinition, RegistryError> {
    registry.model(&route.model_id)
}
