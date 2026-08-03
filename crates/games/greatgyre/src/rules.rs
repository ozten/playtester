//! `GreatGyreGame` — the top-level [`playtest_core::Game`] implementation.
//!
//! Unit 1 scope: `Phase::SurvivorDraft` (players pick a survivor in
//! seat order) and `Phase::AwaitingPostDraftShuffle` (the one
//! `resolve_chance` step Great Gyre needs — see `Event::PostDraftSetup`
//! and `setup.rs`'s doc comment for why the shuffle can't happen inside
//! `initial_state`). `Phase::Draw` / `Actions` / `ResolvingDecision`
//! and the rest of the turn machine land in Unit 2, which will extend
//! `legal_actions` / `apply_action` / `apply_event` below.

use playtest_core::{Actor, Game, GameError, GameResult, PlayerId};
use playtest_ports::Rng;

use crate::action::Action;
use crate::card::Card;
use crate::config::GreatGyreConfig;
use crate::event::Event;
use crate::public_view::{GreatGyrePublicView, public_view as build_public_view};
use crate::state::{GameState, Phase};

/// Zero-sized game marker. Instances are cheap and stateless.
#[derive(Debug, Default, Clone, Copy)]
pub struct GreatGyreGame;

impl GreatGyreGame {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Short identifier used in log headers.
    pub const NAME: &'static str = "greatgyre";
}

impl Game for GreatGyreGame {
    type State = GameState;
    type Action = Action;
    type Event = Event;
    type PublicView = GreatGyrePublicView;
    type Config = GreatGyreConfig;

    fn initial_state(&self, _seed: u64, cfg: &GreatGyreConfig) -> GameState {
        // No randomness is used building the pre-draft state (see
        // `setup.rs`); the seed only matters once `resolve_chance`
        // runs the post-draft shuffle, which the harness seeds
        // independently via the `Rng` port.
        crate::setup::initial_state(cfg)
    }

    fn next_actor(&self, state: &GameState) -> Actor {
        match state.phase {
            Phase::AwaitingPostDraftShuffle => Actor::Chance,
            Phase::ResolvingDecision => {
                // The pending-decision stack is always for a single
                // player in Units 1-2 (no multi-player interruptions
                // yet); its top entry names who's up.
                state
                    .current_pending()
                    .map_or(Actor::Player(state.current_player), |p| {
                        Actor::Player(p.player)
                    })
            }
            Phase::SurvivorDraft | Phase::Draw | Phase::Actions | Phase::Finished => {
                Actor::Player(state.current_player)
            }
        }
    }

    fn legal_actions(&self, state: &GameState, player: PlayerId) -> Vec<Action> {
        match state.phase {
            Phase::SurvivorDraft => legal_draft_actions(state, player),
            Phase::Draw
            | Phase::Actions
            | Phase::ResolvingDecision
            | Phase::AwaitingPostDraftShuffle
            | Phase::Finished => Vec::new(),
        }
    }

    fn apply_action(
        &self,
        state: &GameState,
        player: PlayerId,
        action: &Action,
    ) -> Result<Vec<Event>, GameError> {
        match state.phase {
            Phase::SurvivorDraft => apply_draft_action(state, player, *action),
            Phase::AwaitingPostDraftShuffle
            | Phase::Draw
            | Phase::Actions
            | Phase::ResolvingDecision
            | Phase::Finished => Err(GameError::IllegalAction {
                player,
                message: format!(
                    "action rejected: phase is {:?}, no actions accepted yet",
                    state.phase
                ),
            }),
        }
    }

    fn resolve_chance(&self, state: &GameState, rng: &mut dyn Rng) -> Result<Event, GameError> {
        match state.phase {
            Phase::AwaitingPostDraftShuffle => Ok(build_post_draft_setup_event(state, rng)),
            _ => Err(GameError::ChanceFailed {
                message: format!("no chance step pending in phase {:?}", state.phase),
            }),
        }
    }

    fn apply_event(&self, state: &mut GameState, event: &Event) {
        apply_event_impl(state, event);
    }

    fn public_view(&self, state: &GameState, player: PlayerId) -> GreatGyrePublicView {
        build_public_view(state, player)
    }

    fn determinize(&self, state: &GameState, observer: PlayerId, rng: &mut dyn Rng) -> GameState {
        crate::determinize::determinize(state, observer, rng)
    }

    fn game_over(&self, state: &GameState) -> Option<GameResult> {
        if state.phase == Phase::Finished {
            let (result, _, _) = crate::scoring::build_game_result(state);
            return Some(result);
        }
        None
    }
}

// ---------- SurvivorDraft --------------------------------------------------

fn legal_draft_actions(state: &GameState, player: PlayerId) -> Vec<Action> {
    if state.current_player != player {
        return Vec::new();
    }
    state
        .undrafted_survivors
        .iter()
        .filter_map(|c| match c.kind {
            crate::card::CardKind::Survivor(s) => Some(Action::DraftSurvivor { survivor: s }),
            _ => None,
        })
        .collect()
}

fn apply_draft_action(
    state: &GameState,
    player: PlayerId,
    action: Action,
) -> Result<Vec<Event>, GameError> {
    if state.current_player != player {
        return Err(GameError::IllegalAction {
            player,
            message: format!(
                "action rejected: current_player is {}, not {player}",
                state.current_player
            ),
        });
    }
    let Action::DraftSurvivor { survivor } = action else {
        return Err(GameError::IllegalAction {
            player,
            message: "only DraftSurvivor is legal during Phase::SurvivorDraft".into(),
        });
    };
    let Some(card) = state
        .undrafted_survivors
        .iter()
        .find(|c| matches!(c.kind, crate::card::CardKind::Survivor(s) if s == survivor))
        .copied()
    else {
        return Err(GameError::IllegalAction {
            player,
            message: format!("survivor {survivor:?} is not available to draft"),
        });
    };
    Ok(vec![Event::SurvivorDrafted {
        player,
        survivor: card,
    }])
}

// ---------- chance: post-draft shuffle -------------------------------------

fn build_post_draft_setup_event(state: &GameState, rng: &mut dyn Rng) -> Event {
    let n = state.players.len();

    // Shuffle: draft leftovers + modifications + Dead Fish + resources.
    let mut pool: Vec<Card> = state.pending_shuffle_pool.clone();
    pool.extend(state.undrafted_survivors.iter().copied());
    fisher_yates(&mut pool, rng);

    let mut hands: Vec<Vec<Card>> = Vec::with_capacity(n);
    for _ in 0..n {
        hands.push(drain_from_end(&mut pool, 3));
    }
    let mut currents: Vec<Vec<Card>> = Vec::with_capacity(n);
    for _ in 0..n {
        currents.push(drain_from_end(&mut pool, 3));
    }
    let final_round_deck = drain_from_end(&mut pool, 2 * n);
    let mut deep_sea_deck = pool; // remainder

    // Event deck: shuffled independently, 1 dealt per seat.
    let mut events: Vec<Card> = state.pending_event_pool.clone();
    fisher_yates(&mut events, rng);
    let mut event_hand_cards: Vec<Card> = Vec::with_capacity(n);
    for _ in 0..n {
        event_hand_cards.extend(drain_from_end(&mut events, 1));
    }
    let event_deck = events;

    // Seat 0's opening Phase-1 add, so the very next actor is a real
    // decision (`Phase::Draw`), not another no-op prompt.
    let first_add = deep_sea_deck.pop();

    Event::PostDraftSetup {
        hands,
        currents,
        event_hand_cards,
        deep_sea_deck,
        final_round_deck,
        event_deck,
        first_add,
    }
}

/// Remove and return the last `n` elements of `v` (order preserved),
/// like repeated `pop()`. Used for dealing off the top of a shuffled
/// deck.
fn drain_from_end<T>(v: &mut Vec<T>, n: usize) -> Vec<T> {
    let take = n.min(v.len());
    v.split_off(v.len() - take)
}

fn fisher_yates<T>(slice: &mut [T], rng: &mut dyn Rng) {
    let n = slice.len();
    if n < 2 {
        return;
    }
    let mut i = n - 1;
    while i > 0 {
        let upper = u64::try_from(i).expect("usize fits in u64") + 1;
        let j_u64 = rng
            .gen_range(0..upper)
            .expect("0..(i+1) is never empty for i > 0");
        let j = usize::try_from(j_u64).expect("j <= i fits in usize");
        slice.swap(i, j);
        i -= 1;
    }
}

// ---------- apply_event ----------------------------------------------------

fn apply_event_impl(state: &mut GameState, event: &Event) {
    match event {
        Event::SurvivorDrafted { player, survivor } => {
            let idx = *player as usize;
            if let Some(pos) = state
                .undrafted_survivors
                .iter()
                .position(|c| c.id == survivor.id)
            {
                state.undrafted_survivors.remove(pos);
            }
            state.players[idx].placed.push(crate::state::PlacedCard {
                card: *survivor,
                hungry: false,
            });
            let n = state.players.len();
            let next = usize::from(*player) + 1;
            if next >= n {
                state.phase = Phase::AwaitingPostDraftShuffle;
            } else {
                state.current_player = u8::try_from(next).expect("seat fits in u8");
            }
        }
        Event::PostDraftSetup {
            hands,
            currents,
            event_hand_cards,
            deep_sea_deck,
            final_round_deck,
            event_deck,
            first_add,
        } => {
            for (i, p) in state.players.iter_mut().enumerate() {
                p.hand.clone_from(&hands[i]);
                p.hand.push(event_hand_cards[i]);
                p.current = currents[i]
                    .iter()
                    .map(|&card| crate::state::CurrentCard {
                        card,
                        face: crate::state::Face::Up,
                    })
                    .collect();
            }
            state.pending_shuffle_pool.clear();
            state.pending_event_pool.clear();
            state.deep_sea_deck.clone_from(deep_sea_deck);
            state.final_round_deck.clone_from(final_round_deck);
            state.event_deck.clone_from(event_deck);

            // Begin the very first turn: seat 0, Phase 2 (draw) next.
            state.current_player = 0;
            state.first_player = 0;
            state.players[0].draws_remaining = 1;
            state.players[0].actions_remaining = 1;
            if let Some(card) = first_add {
                state.players[0].current.push(crate::state::CurrentCard {
                    card: *card,
                    face: crate::state::Face::Down,
                });
            }
            state.phase = Phase::Draw;
        }
        // Unit 2 fills in the turn-machine event handlers.
        _ => {}
    }
}
