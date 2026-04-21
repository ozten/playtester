//! Broadcasting `GameEventSink`: wraps an inner sink and fans every
//! line out to a `tokio::sync::broadcast` channel in addition to the
//! durable storage the inner sink provides.
//!
//! The `playtest-server` crate uses this to tee live game events into
//! per-run broadcast channels that SSE subscribers listen on. The
//! durable write (to JSONL via the inner sink) is the source of truth;
//! the broadcast is strictly additive and best-effort.
//!
//! # Zero-receiver behavior
//!
//! `broadcast::Sender::send` returns `Err(SendError)` when there are
//! no active receivers. That is the common case: the engine can start
//! emitting events before any SSE subscriber has attached. The adapter
//! treats that error as expected and swallows it — only failures from
//! the inner sink propagate as a [`GameEventSinkError`].

use playtest_ports::{GameEventSink, GameEventSinkError};
use tokio::sync::broadcast;

/// `GameEventSink` wrapper that emits to an inner sink *and* publishes
/// each line to a `tokio::sync::broadcast` channel for live fan-out.
///
/// The type parameter `I` is the inner sink — typically a
/// [`ProductionGameEventSink`](crate::game_event_sink::ProductionGameEventSink)
/// writing to a JSONL file.
#[derive(Debug)]
pub struct BroadcastGameEventSink<I: GameEventSink> {
    inner: I,
    broadcaster: broadcast::Sender<String>,
}

impl<I: GameEventSink> BroadcastGameEventSink<I> {
    /// Wrap `inner` and publish every emitted line through `broadcaster`.
    pub fn new(inner: I, broadcaster: broadcast::Sender<String>) -> Self {
        Self { inner, broadcaster }
    }

    /// Borrow the broadcaster so subscribers can attach to it.
    #[must_use]
    pub fn broadcaster(&self) -> &broadcast::Sender<String> {
        &self.broadcaster
    }

    /// Consume the sink and return the inner sink (e.g. so a test can
    /// inspect an underlying stub filesystem).
    pub fn into_inner(self) -> I {
        self.inner
    }
}

impl<I: GameEventSink> GameEventSink for BroadcastGameEventSink<I> {
    fn emit(&mut self, line: &str) -> Result<(), GameEventSinkError> {
        // Durable write first: the JSONL file is the source of truth.
        self.inner.emit(line)?;
        // Broadcast is best-effort. `SendError` means "no receivers"
        // which is the common case; discard it.
        let _ = self.broadcaster.send(line.to_owned());
        Ok(())
    }

    fn flush(&mut self) -> Result<(), GameEventSinkError> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_event_sink::StubGameEventSink;

    #[test]
    fn emit_forwards_to_inner_and_broadcast() {
        let (tx, mut rx) = broadcast::channel::<String>(16);
        let inner = StubGameEventSink::new();
        let mut sink = BroadcastGameEventSink::new(inner, tx);

        sink.emit("hello").unwrap();
        sink.emit("world\n").unwrap();

        // Inner sink normalises trailing newlines.
        let inner = sink.into_inner();
        assert_eq!(inner.lines(), &["hello\n".to_owned(), "world\n".to_owned()]);

        // Broadcast carries the unmodified line strings.
        assert_eq!(rx.try_recv().unwrap(), "hello".to_owned());
        assert_eq!(rx.try_recv().unwrap(), "world\n".to_owned());
    }

    #[test]
    fn emit_succeeds_when_no_receivers_attached() {
        // The engine typically starts emitting before any SSE client
        // subscribes. Zero-receiver `send` must not surface as an error.
        let (tx, rx) = broadcast::channel::<String>(4);
        drop(rx);
        let inner = StubGameEventSink::new();
        let mut sink = BroadcastGameEventSink::new(inner, tx);

        // No subscribers at all — must still succeed.
        sink.emit("orphan").unwrap();
        let inner = sink.into_inner();
        assert_eq!(inner.lines(), &["orphan\n".to_owned()]);
    }

    #[test]
    fn inner_sink_error_propagates() {
        let (tx, _rx) = broadcast::channel::<String>(4);
        let mut inner = StubGameEventSink::new();
        inner.close();
        let mut sink = BroadcastGameEventSink::new(inner, tx);

        let err = sink.emit("x").unwrap_err();
        assert!(matches!(err, GameEventSinkError::Closed));
    }

    #[test]
    fn flush_delegates_to_inner() {
        let (tx, _rx) = broadcast::channel::<String>(4);
        let inner = StubGameEventSink::new();
        let mut sink = BroadcastGameEventSink::new(inner, tx);
        sink.flush().unwrap();
    }
}
