//! Per-game `TurnCoordinator`: production `RemoteAgentTransport` impl.
//!
//! Owns the pending-prompt state and the per-seat action channels for
//! a single game. Created by [`crate::runner`] before the engine spawns
//! and registered in [`crate::state::RunHandle::turn_coordinators`]
//! keyed by `game_id`. Dropped when the game finishes — dropping closes
//! the action channels, which unblocks any agent stuck on `await_action`
//! with [`RemoteTransportError::Cancelled`].
//!
//! ## Runtime topology
//!
//! The engine runs on a current-thread tokio runtime built inside a
//! `spawn_blocking` task. The HTTP handlers run on the main tokio
//! runtime. `tokio::sync` primitives are runtime-agnostic, so the
//! coordinator's `mpsc` channels bridge the two cleanly.
//!
//! ## Prompt lifecycle
//!
//! 1. Engine calls `HttpRemoteAgent::choose` at seat `s`.
//! 2. Agent calls `TurnCoordinator::issue_prompt(s, legal_json)`.
//! 3. Coordinator assigns `prompt_id`, stores `pending = Some(...)`,
//!    broadcasts the `turn_prompt` SSE frame.
//! 4. Agent calls `TurnCoordinator::await_action(s, prompt_id)` and
//!    blocks on the seat's mpsc receiver.
//! 5. HTTP handler receives `POST .../actions` and calls
//!    `TurnCoordinator::submit(s, prompt_id, action_index)`.
//! 6. `submit` validates and sends `(prompt_id, action_index)` on the
//!    seat's sender; `pending` is cleared.
//! 7. `await_action` pops the matching message and returns the index.

use std::collections::HashMap;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use playtest_agents::{RemoteAgentTransport, RemoteTransportError};
use playtest_api::{SseFrame, TurnPromptPayload};
use serde_json::Value as JsonValue;
use tokio::sync::{Mutex as TokioMutex, broadcast, mpsc};

/// Bounded per-seat action-submission channel capacity. Submissions are
/// consumed immediately by `await_action`, so a small buffer is plenty;
/// it only matters if a client posts two actions in a tight race.
const ACTION_CHANNEL_CAPACITY: usize = 8;

/// Snapshot of the currently-pending prompt for a game. Cloned by the
/// SSE route on reconnect to emit one last `turn_prompt` frame before
/// subscribing live.
#[derive(Clone, Debug)]
pub struct PendingPrompt {
    pub seat: u8,
    pub prompt_id: u64,
    pub legal_actions: Vec<JsonValue>,
}

impl PendingPrompt {
    /// Build the `turn_prompt` SSE frame JSON for this pending prompt.
    ///
    /// # Errors
    /// Returns the serialization error if `SseFrame` fails to serialize
    /// (programmer bug — should not happen in practice).
    pub fn to_sse_line(&self) -> Result<String, serde_json::Error> {
        let frame = SseFrame::TurnPrompt(TurnPromptPayload {
            seat: self.seat,
            prompt_id: self.prompt_id,
            legal_actions: self.legal_actions.clone(),
        });
        serde_json::to_string(&frame)
    }
}

/// Per-seat channel: a sender held by the coordinator (delivered from
/// HTTP handlers) and a receiver held by the agent via `await_action`.
struct Seat {
    tx: mpsc::Sender<(u64, usize)>,
    rx: TokioMutex<mpsc::Receiver<(u64, usize)>>,
}

/// One coordinator per live game that has at least one remote seat.
pub struct TurnCoordinator {
    next_prompt_id: AtomicU64,
    pending: StdMutex<Option<PendingPrompt>>,
    seats: HashMap<u8, Seat>,
    broadcaster: broadcast::Sender<String>,
}

impl core::fmt::Debug for TurnCoordinator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TurnCoordinator")
            .field(
                "next_prompt_id",
                &self.next_prompt_id.load(Ordering::Relaxed),
            )
            .field(
                "pending_seat",
                &self
                    .pending
                    .lock()
                    .ok()
                    .and_then(|p| p.as_ref().map(|pp| pp.seat)),
            )
            .field("seats", &self.seats.keys().copied().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl TurnCoordinator {
    /// Create a coordinator for a game with `remote_seats` backed by an
    /// HTTP remote, broadcasting `turn_prompt` frames on `broadcaster`.
    #[must_use]
    pub fn new(remote_seats: &[u8], broadcaster: broadcast::Sender<String>) -> Self {
        let mut seats = HashMap::new();
        for &s in remote_seats {
            let (tx, rx) = mpsc::channel(ACTION_CHANNEL_CAPACITY);
            seats.insert(
                s,
                Seat {
                    tx,
                    rx: TokioMutex::new(rx),
                },
            );
        }
        Self {
            next_prompt_id: AtomicU64::new(0),
            pending: StdMutex::new(None),
            seats,
            broadcaster,
        }
    }

    /// Snapshot of the current pending prompt, if any. Cloned so the
    /// caller doesn't hold the internal lock.
    #[must_use]
    pub fn pending_snapshot(&self) -> Option<PendingPrompt> {
        self.pending.lock().expect("pending mutex poisoned").clone()
    }

    /// True if `seat` is backed by an HTTP remote. Used by the POST
    /// handler to reject submissions for AI-only seats up front.
    #[must_use]
    pub fn has_seat(&self, seat: u8) -> bool {
        self.seats.contains_key(&seat)
    }

    /// Seats this coordinator manages, for introspection / testing.
    #[must_use]
    pub fn seat_count(&self) -> usize {
        self.seats.len()
    }

    /// Deliver an action submission. Validates seat, pending state,
    /// prompt_id match, and action_index range. On success, clears
    /// pending and wakes the agent.
    ///
    /// # Errors
    /// See [`SubmitError`] for the rejection taxonomy.
    pub fn submit(
        &self,
        seat: u8,
        prompt_id: u64,
        action_index: usize,
    ) -> Result<(), SubmitError> {
        let Some(seat_chan) = self.seats.get(&seat) else {
            return Err(SubmitError::NoRemoteAgentAtSeat);
        };

        let mut pending = self.pending.lock().expect("pending mutex poisoned");
        let Some(p) = pending.as_ref() else {
            return Err(SubmitError::NotYourTurn);
        };
        if p.seat != seat {
            return Err(SubmitError::NotYourTurn);
        }
        if p.prompt_id != prompt_id {
            return Err(SubmitError::StaleTick {
                submitted: prompt_id,
                expected: p.prompt_id,
            });
        }
        if action_index >= p.legal_actions.len() {
            return Err(SubmitError::IllegalActionIndex {
                submitted: action_index,
                legal_count: p.legal_actions.len(),
            });
        }

        seat_chan
            .tx
            .try_send((prompt_id, action_index))
            .map_err(|e| SubmitError::Transport(e.to_string()))?;
        *pending = None;
        Ok(())
    }
}

/// Errors returned by [`TurnCoordinator::submit`]. Map these to HTTP
/// responses in the POST handler.
#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    /// No HTTP remote is backing this seat — it's AI-only.
    #[error("no remote agent at this seat")]
    NoRemoteAgentAtSeat,

    /// The pending prompt (if any) is not addressed to this seat, or
    /// no prompt is pending at all.
    #[error("no prompt pending for this seat")]
    NotYourTurn,

    /// The submitted `prompt_id` does not match the pending one.
    #[error("stale prompt_id (submitted={submitted}, expected={expected})")]
    StaleTick { submitted: u64, expected: u64 },

    /// `action_index` out of range of the legal-actions list.
    #[error("action_index {submitted} out of range (legal_count={legal_count})")]
    IllegalActionIndex {
        submitted: usize,
        legal_count: usize,
    },

    /// Could not deliver the action to the agent — channel full,
    /// closed, or similar transport-level failure.
    #[error("transport error delivering action: {0}")]
    Transport(String),
}

#[async_trait]
impl RemoteAgentTransport for TurnCoordinator {
    async fn issue_prompt(
        &self,
        seat: u8,
        legal_json: Vec<JsonValue>,
    ) -> Result<u64, RemoteTransportError> {
        if !self.seats.contains_key(&seat) {
            return Err(RemoteTransportError::Other(format!(
                "issue_prompt called for seat {seat} with no remote agent channel"
            )));
        }
        let prompt_id = self.next_prompt_id.fetch_add(1, Ordering::SeqCst);
        let pending_prompt = PendingPrompt {
            seat,
            prompt_id,
            legal_actions: legal_json,
        };

        // Build the SSE line before taking the pending lock so the lock
        // scope stays tight. Broadcast happens after pending is stored
        // so a client reconnecting between the two steps finds pending
        // populated and re-emits the frame itself.
        let sse_line = pending_prompt
            .to_sse_line()
            .map_err(|e| RemoteTransportError::Other(format!("serialize turn_prompt: {e}")))?;

        {
            let mut slot = self
                .pending
                .lock()
                .expect("pending mutex poisoned");
            *slot = Some(pending_prompt);
        }

        // Best-effort: Err(SendError) means no SSE subscribers, which
        // is fine — reconnect replay covers that.
        let _ = self.broadcaster.send(sse_line);

        Ok(prompt_id)
    }

    async fn await_action(
        &self,
        seat: u8,
        prompt_id: u64,
    ) -> Result<usize, RemoteTransportError> {
        let Some(seat_chan) = self.seats.get(&seat) else {
            return Err(RemoteTransportError::Other(format!(
                "await_action called for seat {seat} with no remote agent channel"
            )));
        };
        let mut rx = seat_chan.rx.lock().await;
        loop {
            // Stale submissions (e.g., one that arrived after a submit
            // race cleared pending for a different id) are dropped
            // silently; keep waiting for the matching prompt_id.
            match rx.recv().await {
                None => return Err(RemoteTransportError::Cancelled),
                Some((id, idx)) if id == prompt_id => return Ok(idx),
                Some(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_coord(seats: &[u8]) -> (Arc<TurnCoordinator>, broadcast::Receiver<String>) {
        let (tx, rx) = broadcast::channel::<String>(16);
        (Arc::new(TurnCoordinator::new(seats, tx)), rx)
    }

    #[tokio::test]
    async fn happy_path_issue_then_submit_then_await_returns_index() {
        let (coord, mut sse_rx) = make_coord(&[0]);

        let coord_clone = coord.clone();
        let agent = tokio::spawn(async move {
            let pid = coord_clone
                .issue_prompt(0, vec![JsonValue::Null, JsonValue::Null])
                .await
                .unwrap();
            coord_clone.await_action(0, pid).await.unwrap()
        });

        // Wait for the prompt to be broadcast
        let frame_line = sse_rx.recv().await.unwrap();
        assert!(frame_line.contains("turn_prompt"));
        assert!(frame_line.contains("\"seat\":0"));

        // Submit and join the agent task
        let pending = coord.pending_snapshot().unwrap();
        coord.submit(0, pending.prompt_id, 1).unwrap();
        let got = agent.await.unwrap();
        assert_eq!(got, 1);
        assert!(coord.pending_snapshot().is_none());
    }

    #[tokio::test]
    async fn two_seats_resolve_independently() {
        let (coord, _sse_rx) = make_coord(&[0, 1]);

        let c0 = coord.clone();
        let a0 = tokio::spawn(async move {
            let pid = c0.issue_prompt(0, vec![JsonValue::Null]).await.unwrap();
            c0.await_action(0, pid).await.unwrap()
        });
        // Give seat 0 a moment to install its prompt.
        tokio::task::yield_now().await;
        {
            let pending = coord.pending_snapshot().unwrap();
            coord.submit(0, pending.prompt_id, 0).unwrap();
        }
        assert_eq!(a0.await.unwrap(), 0);

        let c1 = coord.clone();
        let a1 = tokio::spawn(async move {
            let pid = c1
                .issue_prompt(1, vec![JsonValue::Null, JsonValue::Null])
                .await
                .unwrap();
            c1.await_action(1, pid).await.unwrap()
        });
        tokio::task::yield_now().await;
        {
            let pending = coord.pending_snapshot().unwrap();
            coord.submit(1, pending.prompt_id, 1).unwrap();
        }
        assert_eq!(a1.await.unwrap(), 1);
    }

    #[tokio::test]
    async fn submit_without_pending_is_not_your_turn() {
        let (coord, _) = make_coord(&[0]);
        let err = coord.submit(0, 0, 0).unwrap_err();
        assert!(matches!(err, SubmitError::NotYourTurn));
    }

    #[tokio::test]
    async fn submit_with_wrong_prompt_id_is_stale_tick() {
        let (coord, _) = make_coord(&[0]);
        let pid = coord.issue_prompt(0, vec![JsonValue::Null]).await.unwrap();
        let err = coord.submit(0, pid + 99, 0).unwrap_err();
        match err {
            SubmitError::StaleTick {
                submitted,
                expected,
            } => {
                assert_eq!(submitted, pid + 99);
                assert_eq!(expected, pid);
            }
            other => panic!("expected StaleTick, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn submit_to_unregistered_seat_is_no_remote_agent() {
        let (coord, _) = make_coord(&[0]);
        coord
            .issue_prompt(0, vec![JsonValue::Null])
            .await
            .unwrap();
        let err = coord.submit(1, 0, 0).unwrap_err();
        assert!(matches!(err, SubmitError::NoRemoteAgentAtSeat));
    }

    #[tokio::test]
    async fn submit_with_out_of_range_index_is_illegal() {
        let (coord, _) = make_coord(&[0]);
        let pid = coord
            .issue_prompt(0, vec![JsonValue::Null, JsonValue::Null])
            .await
            .unwrap();
        let err = coord.submit(0, pid, 5).unwrap_err();
        match err {
            SubmitError::IllegalActionIndex {
                submitted,
                legal_count,
            } => {
                assert_eq!(submitted, 5);
                assert_eq!(legal_count, 2);
            }
            other => panic!("expected IllegalActionIndex, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn issue_prompt_for_unregistered_seat_errors() {
        let (coord, _) = make_coord(&[1]);
        let err = coord.issue_prompt(0, vec![JsonValue::Null]).await.unwrap_err();
        let RemoteTransportError::Other(msg) = err else {
            panic!("expected Other, got {err:?}");
        };
        assert!(msg.contains("seat 0"), "message was: {msg}");
    }

    #[tokio::test]
    async fn emits_turn_prompt_sse_frame_once_per_issue() {
        let (coord, mut rx) = make_coord(&[0]);
        coord
            .issue_prompt(
                0,
                vec![
                    serde_json::json!({"label": 0}),
                    serde_json::json!({"label": 1}),
                    serde_json::json!({"label": 2}),
                ],
            )
            .await
            .unwrap();
        let line = rx.recv().await.unwrap();
        let parsed: SseFrame = serde_json::from_str(&line).unwrap();
        let SseFrame::TurnPrompt(payload) = parsed else {
            panic!("expected TurnPrompt, got {parsed:?}");
        };
        assert_eq!(payload.seat, 0);
        assert_eq!(payload.prompt_id, 0);
        assert_eq!(payload.legal_actions.len(), 3);
    }
}
