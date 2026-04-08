//! # Cambium: Gap-Driven Code Evolution with Verification Harness
//!
//! The cambium pipeline enables Tem to modify its own codebase through a
//! controlled, trust-gated process:
//!
//! 1. **Zone checking** -- protected zone enforcement ensures immutable kernel
//!    files are never modified and trust levels gate what can change autonomously.
//! 2. **Trust engine** -- a state machine that tracks earned trust from a
//!    successful track record, graduating zones from approval-required to
//!    autonomous operation.
//! 3. **Budget enforcement** -- per-session limits on lines changed and files
//!    touched prevent runaway modifications.
//! 4. **History** -- JSON-file-based persistence of growth sessions and trust
//!    state for auditability.
//!
//! This is a library crate -- nothing calls it yet. The pipeline logic lives
//! here, ready to be wired into the agent runtime.

pub mod budget;
pub mod deploy;
pub mod history;
pub mod llm_generator;
pub mod pipeline;
pub mod sandbox;
pub mod trust;
pub mod zone_checker;
