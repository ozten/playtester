//! Post-game LLM critique plumbing (Phase 5).
//!
//! `LlmAgent` answers a standardized questionnaire after the game ends.
//! This module owns the questionnaire schema, the prompt builder, and
//! (in follow-up units) the per-agent critique method, the sidecar
//! writer, and the coder-pass primitives.
//!
//! Three categories of files end up on disk per game:
//!
//! - `<gid>.jsonl` — main event log (authoritative game history; untouched).
//! - `<gid>.llm.jsonl` — cost-observability sidecar (Phase 3).
//! - `<gid>.critique.jsonl` — subjective-critique sidecar (Phase 5, this module).

pub mod agent;
pub mod prompt;
pub mod shared;
pub mod sidecar;
pub mod spec;

pub use agent::critique_once;
pub use prompt::{build_critique_instructions, build_critique_system_blocks, build_critique_user_message};
pub use shared::{PostGameCritic, SharedLlmAgent, build_shared_handles};
pub use sidecar::{
    CodedTag, CodedTagRecord, CritiqueSidecar, CritiqueSidecarHeader,
    QuestionnaireResponseRecord,
};
pub use spec::{
    OpenEndedPrompt, QuestionItem, QuestionnaireSpec, SpecVersion, default_questionnaire_v1,
};
