//! Snapshot of user-selected feature policy, shared by runtime construction paths.
use temm1e_core::types::config::{EngramConfig, Temm1eConfig};

#[derive(Clone, Debug)]
pub struct RuntimePolicy {
    pub(crate) v2_optimizations: bool,
    pub(crate) self_audit_enabled: bool,
    pub(crate) parallel_phases: bool,
    pub(crate) blueprint_notice: bool,
    pub(crate) engram: EngramConfig,
}

impl RuntimePolicy {
    pub fn from_config(config: &Temm1eConfig) -> Self {
        Self {
            v2_optimizations: config.agent.v2_optimizations,
            self_audit_enabled: config.agent.self_audit_enabled,
            parallel_phases: config.agent.parallel_phases,
            blueprint_notice: config.agent.blueprint_notice,
            engram: config.memory.engram.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{budget::BudgetTracker, AgentRuntime};
    use std::sync::Arc;
    use temm1e_test_utils::{MockMemory, MockProvider};

    #[test]
    fn descendants_inherit_effective_policy_without_replacing_budget() {
        let mut config = Temm1eConfig::default();
        config.agent.v2_optimizations = false;
        config.agent.self_audit_enabled = false;
        config.agent.parallel_phases = false;
        config.agent.blueprint_notice = true;
        config.memory.engram.enabled = false;
        config.memory.engram.curator = "off".into();
        config.memory.engram.max_facts = 7;
        let policy = RuntimePolicy::from_config(&config);
        // A later config mutation must not change an in-flight parent's policy.
        config.memory.engram.enabled = true;
        let make = || {
            AgentRuntime::new(
                Arc::new(MockProvider::with_text("ok")),
                Arc::new(MockMemory::new()),
                vec![],
                "test-model".into(),
                None,
            )
        };
        let parent = make().with_policy(&policy).with_parallel_phases(true);
        let budget = Arc::new(BudgetTracker::new(2.0));
        let child = make()
            .with_budget(budget.clone())
            .with_policy(&parent.runtime_policy());
        let inherited = child.runtime_policy();
        assert!(!inherited.v2_optimizations);
        assert!(!inherited.self_audit_enabled);
        assert!(inherited.parallel_phases);
        assert!(inherited.blueprint_notice);
        assert!(!inherited.engram.enabled);
        assert_eq!(inherited.engram.curator, "off");
        assert_eq!(inherited.engram.max_facts, 7);
        assert!(Arc::ptr_eq(&child.budget(), &budget));
    }
}
