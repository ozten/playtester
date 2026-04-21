//! `Playback<GameEventSink>`: a no-op sink that rejects every write.
//!
//! Replay is strictly a *read* path over an existing game event log. If
//! the engine somehow tries to emit during replay, that is a logic bug —
//! surface it immediately via [`GameEventSinkError::Closed`].

use playtest_ports::{GameEventSink, GameEventSinkError};

#[derive(Debug, Default, Clone, Copy)]
pub struct PlaybackGameEventSink;

impl PlaybackGameEventSink {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl GameEventSink for PlaybackGameEventSink {
    fn emit(&mut self, _line: &str) -> Result<(), GameEventSinkError> {
        Err(GameEventSinkError::Closed)
    }

    fn flush(&mut self) -> Result<(), GameEventSinkError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_sink_rejects_every_write() {
        let mut sink = PlaybackGameEventSink::new();
        let err = sink.emit("anything").unwrap_err();
        assert!(matches!(err, GameEventSinkError::Closed));
    }

    #[test]
    fn playback_sink_flush_is_noop() {
        let mut sink = PlaybackGameEventSink::new();
        sink.flush().unwrap();
    }
}
