//! LLM-backed agent and its supporting machinery.
//!
//! See [`agent::LlmAgent`] for the `Agent` impl, [`scratch::ScratchBuffer`]
//! for the per-agent memory it owns, and [`sidecar::LlmSidecar`] for the
//! cost-observability log written alongside the main event log.

pub mod agent;
pub mod critique;
pub mod prompt;
pub mod scratch;
pub mod sidecar;

pub use agent::{LlmAgent, LlmAgentConfig};
pub use critique::{
    CODER_TAG_TAXONOMY, CodedTag, CodedTagRecord, CoderOutcome, CritiqueSidecar,
    CritiqueSidecarHeader, OpenEndedPrompt, PostGameCritic, QuestionItem,
    QuestionnaireResponseRecord, QuestionnaireSpec, SharedLlmAgent, SpecVersion,
    build_coder_request, build_coder_system_block, build_critique_instructions,
    build_critique_system_blocks, build_critique_user_message, build_shared_handles, code_once,
    default_questionnaire_v1, parse_coder_reply,
};
pub use scratch::{MAX_TURN_LOG, ScratchBuffer};
pub use sidecar::{LlmCallRecord, LlmSidecar, SidecarHeader, sha256_hex};
