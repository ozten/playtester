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
//! Unit 1 (this unit) ships the crate skeleton, card catalog, state
//! shape, and the setup/survivor-draft flow: `Phase::SurvivorDraft`
//! plus the one post-draft chance step. The five-phase turn machine
//! (`Phase::Draw` / `Actions` / `ResolvingDecision`) lands in Unit 2.

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

#[doc(hidden)]
pub mod pool;
#[doc(hidden)]
pub mod setup;

pub use action::{Action, DecisionChoice};
pub use card::{
    Card, CardInstanceId, CardKind, EventKind, ModificationKind, SurvivorId,
};
pub use config::{ConfigError, GreatGyreConfig, MAX_PLAYERS, MIN_PLAYERS};
pub use event::{Event, ScoreRow};
pub use public_view::{GreatGyrePublicView, public_view};
pub use resource::{Resource, ResourceCost};
pub use rules::GreatGyreGame;
pub use state::{
    CurrentCard, Face, GameState, PendingDecision, PendingDecisionKind, Phase, PlacedCard,
    PlayerState,
};
