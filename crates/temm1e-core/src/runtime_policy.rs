//! Snapshot of user-selected feature policy, shared by runtime construction paths.
use crate::types::config::{EngramConfig, Temm1eConfig};

#[derive(Clone, Debug)]
pub struct RuntimePolicy {
    pub v2_optimizations: bool,
    pub self_audit_enabled: bool,
    pub parallel_phases: bool,
    pub blueprint_notice: bool,
    pub engram: EngramConfig,
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
