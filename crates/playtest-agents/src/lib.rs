//! Built-in agents: [`RandomAgent`], [`ScriptedAgent`], [`GreedyAgent`],
//! [`HeuristicAgent`], [`ISMCTSAgent`], [`LlmAgent`], [`HttpRemoteAgent`],
//! [`StdioAgent`].
//!
//! The `Agent` *trait* lives in `playtest-core` (see that crate's
//! lib docs and the architectural invariants memo for why). This crate
//! only provides concrete implementations.
//!
//! Agents choose one action from the engine's enumerated legal actions.
//! They never mutate game state or adjudicate rules — the engine is
//! authoritative.

pub mod eval;
pub mod greedy;
pub mod heuristic;
pub mod ismcts;
pub mod llm;
pub mod random;
pub mod remote;
pub mod scripted;

pub use eval::EvalFn;
pub use greedy::GreedyAgent;
pub use heuristic::{DEFAULT_TEMPERATURE, HeuristicAgent};
pub use ismcts::{ISMCTSAgent, ISMCTSConfig, parse_config_overrides};
pub use llm::{
    CodedTag, CodedTagRecord, CritiqueSidecar, CritiqueSidecarHeader, LlmAgent, LlmAgentConfig,
    LlmCallRecord, LlmSidecar, OpenEndedPrompt, PostGameCritic, QuestionItem,
    QuestionnaireResponseRecord, QuestionnaireSpec, ScratchBuffer, SharedLlmAgent, SidecarHeader,
    SpecVersion, build_critique_instructions, build_critique_system_blocks,
    build_critique_user_message, build_shared_handles, default_questionnaire_v1, sha256_hex,
};
pub use random::RandomAgent;
pub use remote::{
    HttpRemoteAgent, RemoteAgentTransport, RemoteTransportError, StdioAgent, StdioAgentConfig,
    StdioProtocolError,
};
pub use scripted::ScriptedAgent;
