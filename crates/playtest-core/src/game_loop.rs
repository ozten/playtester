//! The game-agnostic game loop.
//!
//! Owns the evolving [`State`](Game::State) and drives turns by asking
//! the `Game` trait who acts next, then routing to either the chance
//! port or the player's agent. Every event is serialized to the
//! [`GameEventSink`] as a single JSONL line and folded back into state.
//!
//! The loop does **not** own the event log file or the JSONL header —
//! that's the caller's concern. The loop only knows about emitting the
//! per-event payload. Later units (the `playtest-log` crate) will
//! provide higher-level builders that construct the full envelope
//! around an emission.

use playtest_ports::{GameEventSink, Rng};

use crate::actor::Actor;
use crate::agent::Agent;
use crate::error::{AgentError, GameError};
use crate::game::Game;
use crate::result::GameResult;

/// Orchestrates a single game from initial state to termination.
pub struct GameLoop<'a, G: Game> {
    game: &'a G,
    state: G::State,
    tick: u64,
}

impl<'a, G: Game> GameLoop<'a, G> {
    /// Start a loop with a game and its already-constructed initial state.
    pub fn new(game: &'a G, initial_state: G::State) -> Self {
        Self {
            game,
            state: initial_state,
            tick: 0,
        }
    }

    /// Consume the loop and return the current state.
    pub fn into_state(self) -> G::State {
        self.state
    }

    /// Borrow the current state.
    pub fn state(&self) -> &G::State {
        &self.state
    }

    /// Number of events emitted so far (and the `tick` of the **next**
    /// event). Starts at 0.
    #[must_use]
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Run the loop to completion.
    ///
    /// `agents` is indexed by [`PlayerId`](crate::actor::PlayerId):
    /// `agents[p as usize]` plays for player `p`. `rng` and `sink` are
    /// the two input/output ports the loop drives directly.
    ///
    /// # Errors
    /// See [`GameError`] for the full failure taxonomy. The loop stops
    /// at the first error; partial state is preserved on the loop
    /// instance for caller inspection.
    pub async fn run(
        &mut self,
        agents: &mut [Box<dyn Agent<G>>],
        rng: &mut dyn Rng,
        sink: &mut dyn GameEventSink,
    ) -> Result<GameResult, GameError> {
        loop {
            if let Some(result) = self.game.game_over(&self.state) {
                return Ok(result);
            }

            match self.game.next_actor(&self.state) {
                Actor::Chance => {
                    let event = self.game.resolve_chance(&self.state, rng)?;
                    self.emit_event(sink, &event)?;
                    self.game.apply_event(&mut self.state, &event);
                }
                Actor::Player(p) => {
                    let legal = self.game.legal_actions(&self.state, p);
                    if legal.is_empty() {
                        return Err(GameError::NoLegalActions { player: p });
                    }

                    let view = self.game.public_view(&self.state, p);
                    let agent =
                        agents
                            .get_mut(p as usize)
                            .ok_or_else(|| GameError::AgentFailed {
                                player: p,
                                source: AgentError::Other(format!(
                                    "no agent registered for player {p}"
                                )),
                            })?;

                    let choice = agent
                        .choose(&view, &legal, &self.state)
                        .await
                        .map_err(|source| GameError::AgentFailed { player: p, source })?;

                    if choice >= legal.len() {
                        return Err(GameError::AgentChoseOutOfBounds {
                            player: p,
                            chosen: choice,
                            legal_count: legal.len(),
                        });
                    }

                    let events = self.game.apply_action(&self.state, p, &legal[choice])?;
                    for e in &events {
                        self.emit_event(sink, e)?;
                        self.game.apply_event(&mut self.state, e);
                    }
                }
            }
        }
    }

    fn emit_event(
        &mut self,
        sink: &mut dyn GameEventSink,
        event: &G::Event,
    ) -> Result<(), GameError> {
        // Wire format matches `playtest_log::LogRecord::Event`, so the log
        // crate's reader can deserialize the loop's raw output directly.
        #[derive(serde::Serialize)]
        struct EventLine<'a, E> {
            kind: &'static str,
            tick: u64,
            payload: &'a E,
        }
        let line = serde_json::to_string(&EventLine {
            kind: "event",
            tick: self.tick,
            payload: event,
        })
        .map_err(|source| GameError::EventSerialization { source })?;
        sink.emit(&line)
            .map_err(|source| GameError::SinkFailed { source })?;
        self.tick += 1;
        Ok(())
    }
}
