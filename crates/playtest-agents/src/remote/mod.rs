//! Remote-decision agents — deferring `choose` to an external client.
//!
//! See [`transport::RemoteAgentTransport`] for the port trait and
//! [`http_remote::HttpRemoteAgent`] for the HTTP-driven variant that
//! Phase 2.5 ships.

pub mod http_remote;
pub mod transport;

pub use http_remote::HttpRemoteAgent;
pub use transport::{RemoteAgentTransport, RemoteTransportError};
