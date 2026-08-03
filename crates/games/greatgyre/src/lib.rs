//! Great Gyre — a raft-survival game for 2-4 players.
//!
//! Each player runs a raft (`Raft Left` + `Raft Right`, never
//! destroyed) with survivors and modifications placed on fungible
//! spaces between them. Every round, the active player draws cards
//! into their own Current, draws from it into hand, takes actions
//! (play a survivor, build a modification or raft extension), discards
//! down to their hand limit, and confirms their food supply. After all
//! seats go, Currents pass to the left and a new round begins. Win:
//! most Hope (face-up hope icons) after the Final Round; ties all win.
//!
//! See `docs/greatgyre.md` for the full rules spec (including its
//! `[A]` assumption rulings) and
//! `docs/plans/2026-08-03-001-feat-greatgyre-engine-plan.md` for the
//! unit breakdown.
//!
//! Unit 1 shipped the crate skeleton, card catalog, state shape, and
//! the setup/survivor-draft flow. Unit 2 landed the five-phase turn
//! machine (`Game` trait impl), sans survivor abilities beyond their
//! printed stat tabs and sans event-card effects. Unit 3 wired up the
//! active per-turn budget bonuses (add/draw/action) and the three
//! special Phase-2 draw sources (Porter, Swimmer, Pirate). Unit 4
//! (this unit) lands the event deck: `play_event` targeting legality,
//! all seven event effects (Shark/Octopus/Walrus/Love Boat/Storm/Work
//! Day/Land Sighting), the Dead Fish/Fisher reaction windows,
//! Telescope activation, Walrus removal, and the Land Sighting/
//! Influencer terms of final scoring.

pub mod action;
pub mod card;
pub mod config;
pub mod determinize;
pub mod event;
pub mod public_view;
pub mod resource;
pub mod rules;
pub mod scoring;
pub mod state;
pub mod turns;

#[doc(hidden)]
pub mod pool;
#[doc(hidden)]
pub mod setup;

pub use action::{Action, DecisionChoice, EventTarget};
pub use card::{
    Card, CardInstanceId, CardKind, EventKind, ModificationKind, SurvivorId,
};
pub use config::{ConfigError, GreatGyreConfig, MAX_PLAYERS, MIN_PLAYERS};
pub use event::{Event, ScoreRow};
pub use public_view::{GreatGyrePublicView, public_view};
pub use resource::{Resource, ResourceCost};
pub use rules::GreatGyreGame;
pub use state::{
    CurrentCard, Face, GameState, PendingChance, PendingDecision, PendingDecisionKind, Phase,
    PlacedCard, PlayerState,
};
