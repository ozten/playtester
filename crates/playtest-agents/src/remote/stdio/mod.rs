//! Stdio-subprocess remote agent.
//!
//! Sibling to [`HttpRemoteAgent`](super::http_remote::HttpRemoteAgent):
//! a [`StdioAgent<G>`] owns its own `tokio::process::Child` 1:1 with
//! the game and defers `choose` to it over newline-delimited JSON.
//! See [`protocol`] for the wire format and [`agent`] for the
//! lifecycle and error taxonomy.

pub mod agent;
pub mod protocol;

pub use agent::{StdioAgent, StdioAgentConfig, StdioProtocolError};
pub use protocol::{ReplyFrame, ReplyScratch, STDIO_API_VERSION, TurnFrame};
