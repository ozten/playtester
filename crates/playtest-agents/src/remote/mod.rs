//! Remote-decision agents — deferring `choose` to an external client.
//!
//! See [`transport::RemoteAgentTransport`] for the port trait and
//! [`http_remote::HttpRemoteAgent`] for the HTTP-driven variant that
//! Phase 2.5 ships. Phase 3 adds [`stdio::StdioAgent`], an agent-owned
//! subprocess sibling that speaks the same "return an index into the
//! legal slice" contract but via newline-delimited JSON over a child
//! process's stdin/stdout.

pub mod http_remote;
pub mod stdio;
pub mod transport;

pub use http_remote::HttpRemoteAgent;
pub use stdio::{StdioAgent, StdioAgentConfig, StdioProtocolError};
pub use transport::{RemoteAgentTransport, RemoteTransportError};
