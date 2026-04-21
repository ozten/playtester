//! Turn-phase enum for a single ShipWreck game.
//!
//! Unit 22 implements the `Setup -> Play -> [ResolvingEvent] -> Finished`
//! transitions. This unit defines the enum so state records serialized
//! by downstream units never need a schema migration when Unit 22 lands.

use serde::{Deserialize, Serialize};

/// High-level state of the ShipWreck turn machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Initial deal is still in progress. After Unit 22's
    /// `initial_state` runs the full setup-event sequence, state
    /// transitions to [`Phase::Play`]; most code paths never observe a
    /// `Setup` state directly but the variant exists so mid-deal
    /// snapshots remain expressible.
    Setup,
    /// Normal turn-taking. The current player may extend their raft,
    /// place a player card, pick wreckage, play an event card, build
    /// equipment, or end their turn.
    Play,
    /// One or more players must resolve a queued event. The canonical
    /// example is a typhoon, where every player in turn chooses which
    /// upgrade/extension to sacrifice (or passes when they own
    /// nothing losable). `event_resolution_stack` on the
    /// `GameState` carries the remaining resolution work.
    ResolvingEvent,
    /// The wreckage deck and every face-up pool are empty, and no event
    /// is pending. The winner has been determined.
    Finished,
}
