//! Initial (pre-draft) state construction for a fresh Great Gyre game.
//!
//! ## Reconciling the setup order with the in-game draft
//!
//! `docs/greatgyre.md`'s literal setup order is: assign rafts → each
//! player *chooses* a survivor → shuffle the leftover survivors into
//! the Main Deck + resources → deal hands/Currents/Final Round Deck →
//! deal 1 Event card each. The plan requires the draft itself to be an
//! **in-game phase** (`Phase::SurvivorDraft`) so a human seat can pick
//! via the normal legal-actions prompt — which means the "shuffle
//! leftovers into the deck" step can only happen *after* the draft
//! resolves, not before.
//!
//! This module (`initial_state`, called from `Game::initial_state`)
//! therefore only does the *deterministic, pre-draft* part: assign
//! each seat's fixed Left/Right raft, and stage the three card pools
//! (`undrafted_survivors`, `pending_shuffle_pool`, `pending_event_pool`)
//! without shuffling or dealing anything from them. `Phase` starts at
//! `SurvivorDraft`. Once the last seat drafts, `rules.rs` transitions
//! to `Phase::AwaitingPostDraftShuffle`, and `resolve_chance` performs
//! the shuffle + deal (it needs the `Rng` port, which only
//! `resolve_chance` has access to) — see `Event::PostDraftSetup`.
//!
//! This is the cleanest deterministic reading available: every card's
//! `CardInstanceId` is still assigned once, up front, by
//! [`crate::pool::build_catalog`]; only the *shuffle* and *deal* are
//! deferred to match the draft's in-game timing.

use crate::config::GreatGyreConfig;
use crate::pool::build_catalog;
use crate::state::{GameState, Phase, PlayerState};

/// Build the pre-draft initial state. No randomness is used here —
/// the draft itself is a sequence of visible player choices, and the
/// only genuinely random step (shuffling draft leftovers into the Deep
/// Sea Deck) is deferred to `resolve_chance`.
#[must_use]
pub fn initial_state(cfg: &GreatGyreConfig) -> GameState {
    let catalog = build_catalog(cfg.num_players);

    let players: Vec<PlayerState> = catalog
        .raft_pairs
        .into_iter()
        .map(|(left, right)| PlayerState::fresh(left, right))
        .collect();

    GameState {
        config: *cfg,
        players,
        undrafted_survivors: catalog.survivors,
        pending_shuffle_pool: catalog.shuffle_pool,
        pending_event_pool: catalog.events,
        deep_sea_deck: Vec::new(),
        final_round_deck: Vec::new(),
        event_deck: Vec::new(),
        discard_pile: Vec::new(),
        extension_pile: catalog.extensions,
        phase: Phase::SurvivorDraft,
        current_player: 0,
        first_player: 0,
        final_round: false,
        pending_decisions: Vec::new(),
    }
}
