//! Immutable execution resources. Cloning retains owning handles, not runtime tasks.
use crate::{
    budget::{BudgetTracker, ModelPricing},
    runtime_policy::RuntimePolicy,
    Memory, Provider, Tool,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct RuntimeResources {
    pub provider: Arc<dyn Provider>,
    pub memory: Arc<dyn Memory>,
    pub budget: Arc<BudgetTracker>,
    pub model: String,
    pub pricing: ModelPricing,
    pub max_context_tokens: usize,
    pub policy: RuntimePolicy,
}
impl RuntimeResources {
    /// Return local tool instances without mutating templates or another turn.
    pub fn bind_tools(&self, tools: &[Arc<dyn Tool>]) -> Vec<Arc<dyn Tool>> {
        tools
            .iter()
            .map(|tool| tool.bind_runtime(self).unwrap_or_else(|| tool.clone()))
            .collect()
    }
}
