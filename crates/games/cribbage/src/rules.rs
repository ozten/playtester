//! `CribbageGame` — the top-level [`playtest_core::Game`]
//! implementation. Everything the harness needs to run a full game
//! lives behind this trait impl; `GameState` and friends are the
//! implementation detail.

use playtest_core::{Actor, Game, GameError, GameResult, PlayerId};
use playtest_ports::Rng;
use serde::{Deserialize, Serialize};

use crate::action::Action;
use crate::board::Board;
use crate::card::Card;
use crate::event::Event;
use crate::hand::Hand;
use crate::phase::Phase;
use crate::state::GameState;

/// Empty config — the single-game, 2-player, 121-point ruleset is the
/// whole scope of Phase 0. Future variants (exact-121, multi-player,
/// short games) would add fields here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CribbageConfig;

/// What an agent sees on their turn. Hides the opponent's hand and
/// the crib contents; exposes everything else needed to play.
#[derive(Debug, Clone)]
pub struct PublicView {
    pub player: PlayerId,
    pub own_hand: Hand,
    pub crib_size: usize,
    pub starter: Option<Card>,
    pub pegging_stack: Vec<Card>,
    pub running_total: u8,
    pub board: Board,
    pub phase: Phase,
    pub to_act: PlayerId,
}

/// Zero-sized game marker. Instances are cheap and stateless —
/// construct one with `CribbageGame` and pass it to [`GameLoop`](playtest_core::GameLoop).
#[derive(Debug, Default, Clone, Copy)]
pub struct CribbageGame;

impl CribbageGame {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Short identifier used in log headers.
    pub const NAME: &'static str = "cribbage";
}

impl Game for CribbageGame {
    type State = GameState;
    type Action = Action;
    type Event = Event;
    type PublicView = PublicView;
    type Config = CribbageConfig;

    fn initial_state(&self, _seed: u64, _cfg: &CribbageConfig) -> GameState {
        // Dealer = 0 on game start. Subsequent hands rotate via
        // `HandComplete`. `seed` is delivered to the engine's Rng
        // port, not consumed here.
        GameState::new(0)
    }

    fn next_actor(&self, state: &GameState) -> Actor {
        state
            .next_actor()
            .expect("GameLoop called next_actor after the game ended")
    }

    fn legal_actions(&self, state: &GameState, player: PlayerId) -> Vec<Action> {
        state.legal_actions(player)
    }

    fn apply_action(
        &self,
        state: &GameState,
        player: PlayerId,
        action: &Action,
    ) -> Result<Vec<Event>, GameError> {
        state.apply_action(player, action)
    }

    fn resolve_chance(&self, state: &GameState, rng: &mut dyn Rng) -> Result<Event, GameError> {
        state.resolve_chance(rng)
    }

    fn apply_event(&self, state: &mut GameState, event: &Event) {
        state.apply_event(event);
    }

    fn public_view(&self, state: &GameState, player: PlayerId) -> PublicView {
        PublicView {
            player,
            own_hand: state.hands[player as usize].clone(),
            crib_size: state.crib.len(),
            starter: state.starter,
            pegging_stack: state.pegging_stack.clone(),
            running_total: state.running_total,
            board: state.board.clone(),
            phase: state.phase,
            to_act: state.to_act,
        }
    }

    fn game_over(&self, state: &GameState) -> Option<GameResult> {
        state.game_over()
    }
}
