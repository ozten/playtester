//! `GreatGyreGame` — the top-level [`playtest_core::Game`] implementation.
//!
//! Unit 1 landed `Phase::SurvivorDraft` (players pick a survivor in
//! seat order) and `Phase::AwaitingPostDraftShuffle` (the one
//! `resolve_chance` step Great Gyre needs — see `Event::PostDraftSetup`
//! and `setup.rs`'s doc comment for why the shuffle can't happen inside
//! `initial_state`).
//!
//! Unit 2 landed the five-phase turn machine: `Phase::Draw` (Phase 2),
//! `Phase::Actions` (Phase 3), and `Phase::ResolvingDecision`
//! (hand-limit discard-down and Phase 4 hungry/stand-up choices).
//! Phases 1 and 5 have no player decisions per the spec, so they never
//! appear as a `Phase` variant a player is prompted in — they're
//! folded directly into the event batch that ends the preceding
//! decision (see `begin_turn_chain` / `end_of_turn_chain` below),
//! exactly the "no no-op prompts" design decision from the plan.
//!
//! Unit 3 (this unit) wires up the active per-turn budget bonuses
//! (`add_bonus` / `draw_bonus` / `action_bonus` — `crate::turns`) and
//! the three special Phase-2 draw sources (Porter, Swimmer, Pirate).
//! Pirate's steal needs the `Rng` port, which `apply_action` doesn't
//! have, so it's a two-step dance: the action transitions to
//! `Phase::AwaitingPirateSteal` (`Actor::Chance`), and `resolve_chance`
//! picks the actual card. No event-card effects are wired up yet
//! (Unit 4) — `play_event` is not a legal action.

use playtest_core::{Actor, Game, GameError, GameResult, PlayerId};
use playtest_ports::Rng;

use crate::action::{Action, DecisionChoice};
use crate::card::{Card, CardInstanceId, CardKind};
use crate::config::GreatGyreConfig;
use crate::event::Event;
use crate::public_view::{GreatGyrePublicView, public_view as build_public_view};
use crate::state::{
    CurrentCard, Face, GameState, PendingChance, PendingDecision, PendingDecisionKind, Phase,
    PlacedCard,
};
use crate::turns::{
    action_bonus, add_bonus, adjacent_players, can_afford, compute_food, compute_hand_limit,
    draw_bonus, extension_cost, find_in_hand, free_spaces, has_fisher, has_survivor,
    select_payment, storm_order, survivor_count,
};

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
            Phase::AwaitingPostDraftShuffle | Phase::AwaitingPirateSteal => Actor::Chance,
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
            Phase::Draw => legal_draw_actions(state, player),
            Phase::Actions => legal_action_phase_actions(state, player),
            Phase::ResolvingDecision => legal_decision_actions(state, player),
            Phase::AwaitingPostDraftShuffle | Phase::AwaitingPirateSteal | Phase::Finished => {
                Vec::new()
            }
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
            Phase::Draw | Phase::Actions | Phase::ResolvingDecision => {
                apply_turn_action(state, player, *action)
            }
            Phase::AwaitingPostDraftShuffle
            | Phase::AwaitingPirateSteal
            | Phase::Finished => Err(GameError::IllegalAction {
                player,
                message: format!(
                    "action rejected: phase is {:?}, no actions accepted",
                    state.phase
                ),
            }),
        }
    }

    fn resolve_chance(&self, state: &GameState, rng: &mut dyn Rng) -> Result<Event, GameError> {
        match state.phase {
            Phase::AwaitingPostDraftShuffle => Ok(build_post_draft_setup_event(state, rng)),
            Phase::AwaitingPirateSteal => resolve_pirate_steal(state, rng),
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

// ---------- chance: Pirate steal --------------------------------------------

fn resolve_pirate_steal(state: &GameState, rng: &mut dyn Rng) -> Result<Event, GameError> {
    let Some(PendingChance::PirateSteal { player, target }) = state.pending_chance else {
        return Err(GameError::ChanceFailed {
            message: "Phase::AwaitingPirateSteal with no PendingChance::PirateSteal set".into(),
        });
    };
    let hand = &state.players[target as usize].hand;
    let n = u64::try_from(hand.len()).map_err(|_| GameError::ChanceFailed {
        message: "target hand too large for a u64 range".into(),
    })?;
    if n == 0 {
        return Err(GameError::ChanceFailed {
            message: format!("Pirate steal target {target} has an empty hand"),
        });
    }
    let idx = rng.gen_range(0..n).map_err(|source| GameError::RngFailed { source })?;
    let card = hand[usize::try_from(idx).expect("idx < n fits in usize")];
    Ok(Event::PirateStole {
        player,
        target,
        card,
    })
}

// ---------- apply_event ----------------------------------------------------

#[allow(clippy::too_many_lines, reason = "one exhaustive Event match; splitting it would scatter the fold logic across files for no readability gain, matching ShipWreck's rules.rs precedent")]
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
            // Any survivors nobody drafted were folded into the shuffle
            // pool by `build_post_draft_setup_event` (see that
            // function) and are now dealt out into hands/Currents/
            // decks above — clear the source list so it doesn't keep a
            // stale second copy of those same physical cards (Unit 5's
            // `public_view`/`determinize` work surfaced this: a
            // lingering leftover-survivor entry here would have been
            // simultaneously "public" via `undrafted_survivors` and
            // hidden via whichever hand/Current/deck it actually
            // landed in).
            state.undrafted_survivors.clear();
            state.pending_shuffle_pool.clear();
            state.pending_event_pool.clear();
            state.deep_sea_deck.clone_from(deep_sea_deck);
            state.final_round_deck.clone_from(final_round_deck);
            state.event_deck.clone_from(event_deck);

            // Begin the very first turn: seat 0, Phase 2 (draw) next.
            state.current_player = 0;
            state.first_player = 0;
            state.players[0].draws_remaining = 1 + draw_bonus(&state.players[0]);
            state.players[0].actions_remaining = 1 + action_bonus(&state.players[0]);
            if let Some(card) = first_add {
                state.players[0].current.push(crate::state::CurrentCard {
                    card: *card,
                    face: crate::state::Face::Down,
                });
            }
            state.phase = Phase::Draw;
        }

        // ---------- turn machine (Units 2-3) --------------------------
        Event::TurnStarted { player } => {
            let idx = *player as usize;
            state.current_player = *player;
            state.players[idx].draws_remaining = 1 + draw_bonus(&state.players[idx]);
            state.players[idx].actions_remaining = 1 + action_bonus(&state.players[idx]);
            state.players[idx].porter_used = false;
            state.players[idx].swimmer_used = false;
            state.players[idx].pirate_used = false;
            state.players[idx].work_day_active = false;
            state.phase = Phase::Draw;
        }
        Event::FinalRoundTriggered => {
            state.final_round = true;
        }
        Event::CurrentCardAdded { player, card } => {
            if state.deep_sea_deck.last() == Some(card) {
                state.deep_sea_deck.pop();
            } else if let Some(pos) = state.final_round_deck.iter().position(|c| c == card) {
                state.final_round_deck.remove(pos);
            }
            state.players[*player as usize].current.push(CurrentCard {
                card: *card,
                face: Face::Down,
            });
        }
        Event::CardDrawnFromCurrent { player, card } => {
            let idx = *player as usize;
            if let Some(pos) = state.players[idx]
                .current
                .iter()
                .position(|c| c.card.id == card.id)
            {
                state.players[idx].current.remove(pos);
            }
            state.players[idx].hand.push(*card);
            state.players[idx].draws_remaining =
                state.players[idx].draws_remaining.saturating_sub(1);
        }
        Event::DrewFromDiscardPile { player, card } => {
            let idx = *player as usize;
            if let Some(pos) = state.discard_pile.iter().position(|c| c.id == card.id) {
                state.discard_pile.remove(pos);
            }
            state.players[idx].hand.push(*card);
            state.players[idx].draws_remaining =
                state.players[idx].draws_remaining.saturating_sub(1);
            state.players[idx].porter_used = true;
        }
        Event::DrewFromAdjacentCurrent {
            player,
            neighbor,
            card,
        } => {
            let n_idx = *neighbor as usize;
            if let Some(pos) = state.players[n_idx]
                .current
                .iter()
                .position(|c| c.card.id == card.id)
            {
                state.players[n_idx].current.remove(pos);
            }
            let idx = *player as usize;
            state.players[idx].hand.push(*card);
            state.players[idx].draws_remaining =
                state.players[idx].draws_remaining.saturating_sub(1);
            state.players[idx].swimmer_used = true;
        }
        Event::PirateStealInitiated { player, target } => {
            state.pending_chance = Some(PendingChance::PirateSteal {
                player: *player,
                target: *target,
            });
            state.phase = Phase::AwaitingPirateSteal;
        }
        Event::PirateStole {
            player,
            target,
            card,
        } => {
            let t_idx = *target as usize;
            if let Some(pos) = state.players[t_idx]
                .hand
                .iter()
                .position(|c| c.id == card.id)
            {
                state.players[t_idx].hand.remove(pos);
            }
            let idx = *player as usize;
            state.players[idx].hand.push(*card);
            state.players[idx].draws_remaining =
                state.players[idx].draws_remaining.saturating_sub(1);
            state.players[idx].pirate_used = true;
            state.pending_chance = None;
            state.phase = Phase::Draw;
        }
        Event::DrawingFinished { .. } => {
            // `actions_remaining` was already set correctly (to
            // `1 + action_bonus`) by this turn's `TurnStarted`; only
            // the phase changes here.
            state.phase = Phase::Actions;
        }
        Event::SurvivorPlayed { player, card } | Event::ModificationBuilt { player, card } => {
            let idx = *player as usize;
            remove_from_hand(&mut state.players[idx].hand, card.id);
            state.players[idx].placed.push(PlacedCard {
                card: *card,
                hungry: false,
            });
            state.players[idx].actions_remaining =
                state.players[idx].actions_remaining.saturating_sub(1);
        }
        Event::ExtensionBuilt { player, extension } => {
            let idx = *player as usize;
            if let Some(pos) = state
                .extension_pile
                .iter()
                .position(|c| c.id == extension.id)
            {
                state.extension_pile.remove(pos);
            }
            state.players[idx].built_extensions.push(*extension);
            state.players[idx].actions_remaining =
                state.players[idx].actions_remaining.saturating_sub(1);
        }
        Event::PendingDecisionOpened { player, decision } => {
            // Nothing in Units 1-4 nests decisions — every decision
            // kind's resolution pops itself (via `decrement_pending` /
            // `pop_pending`, both driven by the resolving *event*, not
            // a bare mutation on a scratch clone) before the chain
            // opens the next one. If this ever fires, a resolution
            // path is popping on a throwaway `work` clone instead of
            // through an event, so the pop never reached real state —
            // exactly the bug class `pop_pending`'s doc comment
            // describes (caught here once, for Walrus/Octopus reaction
            // resolution, before Unit 4 shipped).
            assert!(
                state.pending_decisions.is_empty(),
                "pushing {decision:?} for player {player} while the pending-decision stack \
                 already has {:?} — a resolution path popped a scratch clone instead of \
                 emitting an event",
                state.pending_decisions
            );
            state.pending_decisions.push(PendingDecision {
                player: *player,
                kind: decision.clone(),
            });
            state.phase = Phase::ResolvingDecision;
        }
        Event::CardDiscarded { player, card } => {
            let idx = *player as usize;
            remove_from_hand(&mut state.players[idx].hand, card.id);
            state.players[idx].current.push(CurrentCard {
                card: *card,
                face: Face::Down,
            });
            decrement_pending(state, *player, |k| {
                matches!(
                    k,
                    PendingDecisionKind::DiscardDown { .. } | PendingDecisionKind::StormDiscard { .. }
                )
            });
        }
        Event::SurvivorMadeHungry { player, survivor } => {
            let idx = *player as usize;
            if let Some(p) = state.players[idx]
                .placed
                .iter_mut()
                .find(|p| p.card.id == survivor.id)
            {
                p.hungry = true;
            }
            decrement_pending(state, *player, |k| matches!(k, PendingDecisionKind::MakeHungry { .. }));
        }
        Event::SurvivorAbandoned { player, survivor } => {
            let idx = *player as usize;
            if let Some(pos) = state.players[idx]
                .placed
                .iter()
                .position(|p| p.card.id == survivor.id)
            {
                let pc = state.players[idx].placed.remove(pos);
                state.players[idx].current.push(CurrentCard {
                    card: pc.card,
                    face: Face::Up,
                });
            }
            decrement_pending(state, *player, |k| {
                matches!(k, PendingDecisionKind::AbandonHungry { .. })
            });
        }
        Event::SurvivorStoodUp { player, survivor } => {
            let idx = *player as usize;
            if let Some(p) = state.players[idx]
                .placed
                .iter_mut()
                .find(|p| p.card.id == survivor.id)
            {
                p.hungry = false;
            }
            decrement_pending(state, *player, |k| matches!(k, PendingDecisionKind::StandUp { .. }));
        }
        Event::ActionsFinished { .. } | Event::TurnEnded { .. } => {
            // Pure log markers; the next event in the same batch
            // always carries the real state transition (a
            // pending-decision open, Phase 4 auto-resolution, or the
            // following `TurnStarted`/`EndGame`).
        }
        Event::ReactionDeclined { player } => {
            pop_pending(state, *player, |k| matches!(k, PendingDecisionKind::EventReaction { .. }));
        }
        Event::StormDiscardRouteTaken { player } => {
            pop_pending(state, *player, |k| matches!(k, PendingDecisionKind::StormChoice { .. }));
        }
        Event::CurrentsPassed { new_first_player } => {
            let n = state.players.len();
            let mut incoming: Vec<Vec<CurrentCard>> = vec![Vec::new(); n];
            for (i, p) in state.players.iter_mut().enumerate() {
                let dest = (i + 1) % n;
                incoming[dest] = std::mem::take(&mut p.current);
            }
            for (i, p) in state.players.iter_mut().enumerate() {
                p.current = std::mem::take(&mut incoming[i]);
            }
            state.first_player = *new_first_player;
        }
        Event::EndGame { .. } => {
            state.phase = Phase::Finished;
        }

        // ---------- Unit 4: events & reactions --------------------------
        Event::EventCardPlayed { player, card, .. } => {
            let idx = *player as usize;
            remove_from_hand(&mut state.players[idx].hand, card.id);
            // Walrus stays in play (physically ends up on a raft, not
            // the discard pile) unless the reaction negates it — see
            // `Event::WalrusPlaced` / `Event::WalrusDiscarded`.
            if !matches!(card.kind, CardKind::Event(crate::card::EventKind::Walrus)) {
                state.discard_pile.push(*card);
            }
            state.players[idx].actions_remaining =
                state.players[idx].actions_remaining.saturating_sub(1);
        }
        Event::ResourceDiscarded { player, card } => {
            let idx = *player as usize;
            remove_from_hand(&mut state.players[idx].hand, card.id);
            state.discard_pile.push(*card);
        }
        Event::ReactedWithDeadFish { player, card } | Event::ReactedWithFisher { player, card } => {
            let idx = *player as usize;
            remove_from_hand(&mut state.players[idx].hand, card.id);
            state.discard_pile.push(*card);
            pop_pending(state, *player, |k| matches!(k, PendingDecisionKind::EventReaction { .. }));
        }
        Event::WalrusPlaced { player, card, .. } => {
            state.players[*player as usize].blocked_by_walrus.push(*card);
        }
        Event::WalrusDiscarded { card, .. } => {
            state.discard_pile.push(*card);
        }
        Event::WalrusRemoved {
            player,
            dead_fish,
            walrus,
        } => {
            let idx = *player as usize;
            remove_from_hand(&mut state.players[idx].hand, dead_fish.id);
            state.discard_pile.push(*dead_fish);
            if let Some(pos) = state.players[idx]
                .blocked_by_walrus
                .iter()
                .position(|c| c.id == walrus.id)
            {
                state.players[idx].blocked_by_walrus.remove(pos);
            }
            state.discard_pile.push(*walrus);
            state.players[idx].actions_remaining =
                state.players[idx].actions_remaining.saturating_sub(1);
        }
        Event::SurvivorLostToShark { player, survivor } => {
            let idx = *player as usize;
            if let Some(pos) = state.players[idx]
                .placed
                .iter()
                .position(|pc| pc.card.id == survivor.id)
            {
                state.players[idx].placed.remove(pos);
            }
            state.discard_pile.push(*survivor);
            pop_pending(state, *player, |k| {
                matches!(k, PendingDecisionKind::SharkChooseSurvivor { .. })
            });
        }
        Event::ExtensionLostToOctopus { player, extension } => {
            let idx = *player as usize;
            if let Some(pos) = state.players[idx]
                .built_extensions
                .iter()
                .position(|c| c.id == extension.id)
            {
                state.players[idx].built_extensions.remove(pos);
            }
            state.extension_pile.push(*extension);
        }
        Event::RelocatedFromOctopus { player, card } => {
            let idx = *player as usize;
            if let Some(pos) = state.players[idx]
                .placed
                .iter()
                .position(|pc| pc.card.id == card.id)
            {
                state.players[idx].placed.remove(pos);
            }
            state.players[idx].current.push(CurrentCard {
                card: *card,
                face: Face::Up,
            });
            decrement_pending(state, *player, |k| {
                matches!(k, PendingDecisionKind::OctopusRelocate { .. })
            });
        }
        Event::SurvivorGivenToLoveBoat {
            player,
            recipient,
            survivor,
        } => {
            let idx = *player as usize;
            if let Some(pos) = state.players[idx]
                .placed
                .iter()
                .position(|pc| pc.card.id == survivor.id)
            {
                state.players[idx].placed.remove(pos);
            }
            state.players[*recipient as usize].placed.push(PlacedCard {
                card: *survivor,
                hungry: false,
            });
            pop_pending(state, *player, |k| {
                matches!(k, PendingDecisionKind::LoveBoatChooseSurvivor { .. })
            });
        }
        Event::StormCardRemoved { player, card } => {
            let idx = *player as usize;
            if let Some(pos) = state.players[idx]
                .placed
                .iter()
                .position(|pc| pc.card.id == card.id)
            {
                state.players[idx].placed.remove(pos);
            }
            state.players[idx].current.push(CurrentCard {
                card: *card,
                face: Face::Up,
            });
            pop_pending(state, *player, |k| matches!(k, PendingDecisionKind::StormChoice { .. }));
        }
        Event::WorkDayActivated { player } => {
            state.players[*player as usize].work_day_active = true;
        }
        Event::WorkDayDrew { player, card } => {
            let idx = *player as usize;
            if let Some(pos) = state.players[idx]
                .current
                .iter()
                .position(|c| c.card.id == card.id)
            {
                state.players[idx].current.remove(pos);
            }
            state.players[idx].hand.push(*card);
        }
        Event::TelescopeActivated {
            player,
            telescope,
            drawn,
        } => {
            let idx = *player as usize;
            if let Some(pos) = state.players[idx]
                .placed
                .iter()
                .position(|pc| pc.card.id == telescope.id)
            {
                state.players[idx].placed.remove(pos);
            }
            state.discard_pile.push(*telescope);
            if let Some(pos) = state.event_deck.iter().position(|c| c.id == drawn.id) {
                state.event_deck.remove(pos);
            }
            state.players[idx].hand.push(*drawn);
            state.players[idx].actions_remaining =
                state.players[idx].actions_remaining.saturating_sub(1);
        }
        Event::EventResolved { .. } => {
            state.phase = Phase::Actions;
        }
    }
}

/// Remove the first hand card matching `id`, if present.
fn remove_from_hand(hand: &mut Vec<Card>, id: CardInstanceId) {
    if let Some(pos) = hand.iter().position(|c| c.id == id) {
        hand.remove(pos);
    }
}

/// If the top of the pending-decision stack belongs to `player` and
/// its kind matches `pred`, decrement its `needed` counter (popping
/// the stack when it reaches 0). No-op otherwise — safe to call after
/// *every* hungry/stand/discard event, whether it was decision-driven
/// or an auto-resolved forced step.
fn decrement_pending(
    state: &mut GameState,
    player: PlayerId,
    pred: impl Fn(&PendingDecisionKind) -> bool,
) {
    let Some(top) = state.pending_decisions.last_mut() else {
        return;
    };
    if top.player != player || !pred(&top.kind) {
        return;
    }
    let remaining = top.kind.needed().saturating_sub(1);
    top.kind = match &top.kind {
        PendingDecisionKind::DiscardDown { .. } => PendingDecisionKind::DiscardDown { needed: remaining },
        PendingDecisionKind::MakeHungry { .. } => PendingDecisionKind::MakeHungry { needed: remaining },
        PendingDecisionKind::AbandonHungry { .. } => {
            PendingDecisionKind::AbandonHungry { needed: remaining }
        }
        PendingDecisionKind::StandUp { .. } => PendingDecisionKind::StandUp { needed: remaining },
        PendingDecisionKind::OctopusRelocate { attacker, .. } => PendingDecisionKind::OctopusRelocate {
            needed: remaining,
            attacker: *attacker,
        },
        PendingDecisionKind::StormDiscard {
            attacker,
            queue_tail,
            ..
        } => PendingDecisionKind::StormDiscard {
            needed: remaining,
            attacker: *attacker,
            queue_tail: queue_tail.clone(),
        },
        PendingDecisionKind::EventReaction { .. }
        | PendingDecisionKind::SharkChooseSurvivor { .. }
        | PendingDecisionKind::LoveBoatChooseSurvivor { .. }
        | PendingDecisionKind::StormChoice { .. } => {
            unreachable!("decrement_pending is never called with a `pred` matching a single-shot decision kind")
        }
    };
    if remaining == 0 {
        state.pending_decisions.pop();
        if state.pending_decisions.is_empty() {
            // Left dangling as `ResolvingDecision` until the caller's
            // chain appends the next real transition in the same
            // event batch (see `apply_resolve_decision`).
        }
    }
}

/// If the top of the pending-decision stack belongs to `player` and
/// its kind matches `pred`, pop it unconditionally (single-shot Unit 4
/// decision kinds — `EventReaction`, `SharkChooseSurvivor`,
/// `LoveBoatChooseSurvivor`, `StormChoice` — have no `needed` counter
/// to decrement; one answer always fully resolves them). No-op
/// otherwise.
///
/// This — not a bare `work.pending_decisions.pop()` on the throwaway
/// clone used to compute a chain's events — is what actually removes
/// the entry, because it runs from inside `apply_event_impl`. A pop
/// that only touched the scratch clone would never be replayed onto
/// the real state (the same fold-discipline bug U2's `round_queue`
/// caught): the resolving *event* is the only thing that reaches
/// `GameState` when the log is replayed, so any state change —
/// including "this decision is done" — has to live in an event's
/// `apply_event`, not in code that runs alongside it on a scratch copy.
fn pop_pending(state: &mut GameState, player: PlayerId, pred: impl Fn(&PendingDecisionKind) -> bool) {
    let Some(top) = state.pending_decisions.last() else {
        return;
    };
    if top.player != player || !pred(&top.kind) {
        return;
    }
    state.pending_decisions.pop();
}

// ============================================================================
// Unit 2: the five-phase turn machine
// ============================================================================

// ---------- Phase 2 (Draw) -------------------------------------------------

fn legal_draw_actions(state: &GameState, player: PlayerId) -> Vec<Action> {
    if state.current_player != player {
        return Vec::new();
    }
    let p = &state.players[player as usize];
    let mut out = Vec::new();
    if p.draws_remaining > 0 {
        out.extend(
            p.current
                .iter()
                .map(|c| Action::DrawFromCurrent { card: c.card.id }),
        );

        // Special substitute sources — each usable at most once per
        // Phase 2 (`docs/greatgyre.md`'s `[A]` ruling).
        if !p.porter_used
            && has_survivor(p, crate::card::SurvivorId::Porter)
            && !state.discard_pile.is_empty()
        {
            out.push(Action::DrawFromDiscardPile);
        }
        if !p.swimmer_used && has_survivor(p, crate::card::SurvivorId::Swimmer) {
            for neighbor in adjacent_players(state.players.len(), player) {
                out.extend(state.players[neighbor as usize].current.iter().map(|c| {
                    Action::DrawFromAdjacentCurrent {
                        neighbor,
                        card: c.card.id,
                    }
                }));
            }
        }
        if !p.pirate_used && has_survivor(p, crate::card::SurvivorId::Pirate) {
            out.extend(
                (0..state.players.len())
                    .map(|i| u8::try_from(i).expect("seat fits in u8"))
                    .filter(|&target| target != player && !state.players[target as usize].hand.is_empty())
                    .map(|target| Action::DrawRandomFromHand { target }),
            );
        }
    }
    out.push(Action::FinishDrawing);
    out
}

// ---------- Phase 3 (Actions) -----------------------------------------------

fn legal_action_phase_actions(state: &GameState, player: PlayerId) -> Vec<Action> {
    if state.current_player != player {
        return Vec::new();
    }
    let p = &state.players[player as usize];
    let mut out = Vec::new();
    if actions_available(p) {
        for c in &p.hand {
            match c.kind {
                CardKind::Survivor(s) if !s.occupies_space() || free_spaces(p) >= 1 => {
                    out.push(Action::PlaySurvivor { card: c.id });
                }
                CardKind::Modification(m)
                    if can_afford(&p.hand, m.cost())
                        && (!m.occupies_space() || free_spaces(p) >= 1) =>
                {
                    out.push(Action::BuildModification { card: c.id });
                }
                _ => {}
            }
        }
        if !state.extension_pile.is_empty() && can_afford(&p.hand, extension_cost()) {
            out.push(Action::BuildExtension);
        }
        out.extend(legal_play_event_actions(state, player));
        out.extend(legal_activate_telescope_actions(state, p));
        out.extend(legal_remove_walrus_actions(p));
    }
    if p.work_day_active {
        // Work Day: drawing from your own Current becomes a free
        // repeatable Phase-3 action, independent of `actions_remaining`.
        out.extend(
            p.current
                .iter()
                .map(|c| Action::WorkDayDraw { card: c.card.id }),
        );
    }
    out.push(Action::FinishActions);
    out
}

/// Legal `PlayEvent` actions: one per (event card in hand × legal
/// target), per `docs/greatgyre.md`'s Events table. Land Sighting is
/// never offered — "never played". Storm and Work Day take no target.
fn legal_play_event_actions(state: &GameState, player: PlayerId) -> Vec<Action> {
    let p = &state.players[player as usize];
    let n = state.players.len();
    let mut out = Vec::new();
    for c in &p.hand {
        let CardKind::Event(kind) = c.kind else {
            continue;
        };
        match kind {
            crate::card::EventKind::SharkAttack => {
                out.extend((0..n).filter_map(|i| {
                    let target = u8::try_from(i).expect("seat fits in u8");
                    (target != player && survivor_count(&state.players[i]) >= 2).then_some(
                        Action::PlayEvent {
                            card: c.id,
                            target: crate::action::EventTarget::Player { target },
                        },
                    )
                }));
            }
            crate::card::EventKind::OctopusAttack => {
                out.extend((0..n).filter_map(|i| {
                    let target = u8::try_from(i).expect("seat fits in u8");
                    (target != player && !state.players[i].built_extensions.is_empty()).then_some(
                        Action::PlayEvent {
                            card: c.id,
                            target: crate::action::EventTarget::Player { target },
                        },
                    )
                }));
            }
            crate::card::EventKind::Walrus => {
                out.extend((0..n).filter_map(|i| {
                    let target = u8::try_from(i).expect("seat fits in u8");
                    (target != player && free_spaces(&state.players[i]) >= 1).then_some(
                        Action::PlayEvent {
                            card: c.id,
                            target: crate::action::EventTarget::Player { target },
                        },
                    )
                }));
            }
            crate::card::EventKind::LoveBoat => {
                if free_spaces(p) >= 1 {
                    out.extend((0..n).filter_map(|i| {
                        let target = u8::try_from(i).expect("seat fits in u8");
                        (target != player && survivor_count(&state.players[i]) >= 2).then_some(
                            Action::PlayEvent {
                                card: c.id,
                                target: crate::action::EventTarget::Player { target },
                            },
                        )
                    }));
                }
            }
            crate::card::EventKind::Storm | crate::card::EventKind::WorkDay => {
                out.push(Action::PlayEvent {
                    card: c.id,
                    target: crate::action::EventTarget::None,
                });
            }
            crate::card::EventKind::LandSighting => {}
        }
    }
    out
}

fn legal_activate_telescope_actions(state: &GameState, p: &crate::state::PlayerState) -> Vec<Action> {
    if state.event_deck.is_empty() {
        return Vec::new();
    }
    p.placed
        .iter()
        .filter(|pc| matches!(pc.card.kind, CardKind::Modification(crate::card::ModificationKind::Telescope)))
        .map(|pc| Action::ActivateTelescope { card: pc.card.id })
        .collect()
}

fn legal_remove_walrus_actions(p: &crate::state::PlayerState) -> Vec<Action> {
    if p.blocked_by_walrus.is_empty() {
        return Vec::new();
    }
    let dead_fish: Vec<CardInstanceId> = p
        .hand
        .iter()
        .filter(|c| matches!(c.kind, CardKind::DeadFish))
        .map(|c| c.id)
        .collect();
    let mut out = Vec::new();
    for walrus in &p.blocked_by_walrus {
        for &dead_fish in &dead_fish {
            out.push(Action::RemoveWalrus {
                dead_fish,
                walrus: walrus.id,
            });
        }
    }
    out
}

// ---------- ResolvingDecision (discard-down / Phase 4) ---------------------

#[allow(clippy::too_many_lines, reason = "one exhaustive PendingDecisionKind match enumerating each kind's legal choices; splitting it would scatter closely-related enumeration logic across files for no readability gain")]
fn legal_decision_actions(state: &GameState, player: PlayerId) -> Vec<Action> {
    let Some(top) = state.current_pending() else {
        return Vec::new();
    };
    if top.player != player {
        return Vec::new();
    }
    let p = &state.players[player as usize];
    match &top.kind {
        PendingDecisionKind::DiscardDown { .. } => p
            .hand
            .iter()
            .map(|c| {
                Action::ResolveDecision {
                    choice: DecisionChoice::Discard { card: c.id },
                }
            })
            .collect(),
        PendingDecisionKind::MakeHungry { .. } => p
            .placed
            .iter()
            .filter(|pc| !pc.hungry && matches!(pc.card.kind, CardKind::Survivor(_)))
            .map(|pc| Action::ResolveDecision {
                choice: DecisionChoice::MakeHungry { survivor: pc.card.id },
            })
            .collect(),
        PendingDecisionKind::AbandonHungry { .. } => p
            .placed
            .iter()
            .filter(|pc| pc.hungry)
            .map(|pc| Action::ResolveDecision {
                choice: DecisionChoice::AbandonHungry { survivor: pc.card.id },
            })
            .collect(),
        PendingDecisionKind::StandUp { .. } => p
            .placed
            .iter()
            .filter(|pc| pc.hungry)
            .map(|pc| Action::ResolveDecision {
                choice: DecisionChoice::StandUp { survivor: pc.card.id },
            })
            .collect(),

        // ---------- Unit 4: events & reactions ----------------------
        PendingDecisionKind::EventReaction { event, .. } => {
            let mut out: Vec<Action> = p
                .hand
                .iter()
                .filter(|c| matches!(c.kind, CardKind::DeadFish))
                .map(|c| Action::ResolveDecision {
                    choice: DecisionChoice::ReactWithDeadFish { card: c.id },
                })
                .collect();
            // Fisher only negates Shark Attack / Octopus Attack, not
            // Walrus (per `docs/greatgyre.md`'s Reactions section).
            if *event != crate::card::EventKind::Walrus && has_fisher(p) {
                out.extend(p.hand.iter().map(|c| Action::ResolveDecision {
                    choice: DecisionChoice::ReactWithFisher { card: c.id },
                }));
            }
            out.push(Action::ResolveDecision {
                choice: DecisionChoice::DeclineReaction,
            });
            out
        }
        PendingDecisionKind::SharkChooseSurvivor { .. } => p
            .placed
            .iter()
            .filter(|pc| matches!(pc.card.kind, CardKind::Survivor(_)))
            .map(|pc| Action::ResolveDecision {
                choice: DecisionChoice::LoseSurvivorToShark { survivor: pc.card.id },
            })
            .collect(),
        PendingDecisionKind::OctopusRelocate { .. } => p
            .placed
            .iter()
            .map(|pc| Action::ResolveDecision {
                choice: DecisionChoice::RelocateFromOctopus { card: pc.card.id },
            })
            .collect(),
        PendingDecisionKind::LoveBoatChooseSurvivor { .. } => p
            .placed
            .iter()
            .filter(|pc| matches!(pc.card.kind, CardKind::Survivor(_)))
            .map(|pc| Action::ResolveDecision {
                choice: DecisionChoice::GiveSurvivorToLoveBoat { survivor: pc.card.id },
            })
            .collect(),
        PendingDecisionKind::StormChoice { .. } => {
            let mut out: Vec<Action> = p
                .placed
                .iter()
                .map(|pc| Action::ResolveDecision {
                    choice: DecisionChoice::StormRemoveCard { card: pc.card.id },
                })
                .collect();
            out.push(Action::ResolveDecision {
                choice: DecisionChoice::StormTakeDiscardRoute,
            });
            out
        }
        PendingDecisionKind::StormDiscard { .. } => p
            .hand
            .iter()
            .filter(|c| !matches!(c.kind, CardKind::Event(_)))
            .map(|c| Action::ResolveDecision {
                choice: DecisionChoice::StormDiscard { card: c.id },
            })
            .collect(),
    }
}

// ---------- apply_action dispatch for Draw/Actions/ResolvingDecision -------

fn apply_turn_action(
    state: &GameState,
    player: PlayerId,
    action: Action,
) -> Result<Vec<Event>, GameError> {
    match (state.phase, action) {
        (Phase::Draw, Action::DrawFromCurrent { card }) => apply_draw_from_current(state, player, card),
        (Phase::Draw, Action::DrawFromDiscardPile) => apply_draw_from_discard_pile(state, player),
        (Phase::Draw, Action::DrawFromAdjacentCurrent { neighbor, card }) => {
            apply_draw_from_adjacent_current(state, player, neighbor, card)
        }
        (Phase::Draw, Action::DrawRandomFromHand { target }) => {
            apply_draw_random_from_hand(state, player, target)
        }
        (Phase::Draw, Action::FinishDrawing) => {
            if state.current_player != player {
                return Err(not_your_turn(player, state.current_player));
            }
            let mut work = state.clone();
            Ok(finish_drawing_chain(&mut work, player))
        }
        (Phase::Actions, Action::PlaySurvivor { card }) => apply_play_survivor(state, player, card),
        (Phase::Actions, Action::BuildModification { card }) => {
            apply_build_modification(state, player, card)
        }
        (Phase::Actions, Action::BuildExtension) => apply_build_extension(state, player),
        (Phase::Actions, Action::PlayEvent { card, target }) => {
            apply_play_event(state, player, card, target)
        }
        (Phase::Actions, Action::ActivateTelescope { card }) => {
            apply_activate_telescope(state, player, card)
        }
        (Phase::Actions, Action::RemoveWalrus { dead_fish, walrus }) => {
            apply_remove_walrus(state, player, dead_fish, walrus)
        }
        (Phase::Actions, Action::WorkDayDraw { card }) => apply_work_day_draw(state, player, card),
        (Phase::Actions, Action::FinishActions) => {
            if state.current_player != player {
                return Err(not_your_turn(player, state.current_player));
            }
            let mut work = state.clone();
            Ok(finish_actions_chain(&mut work, player))
        }
        (Phase::ResolvingDecision, Action::ResolveDecision { choice }) => {
            apply_resolve_decision(state, player, choice)
        }
        _ => Err(GameError::IllegalAction {
            player,
            message: format!("action {action:?} is not legal in phase {:?}", state.phase),
        }),
    }
}

fn not_your_turn(player: PlayerId, current: PlayerId) -> GameError {
    GameError::IllegalAction {
        player,
        message: format!("action rejected: current_player is {current}, not {player}"),
    }
}

fn apply_draw_from_current(
    state: &GameState,
    player: PlayerId,
    card_id: CardInstanceId,
) -> Result<Vec<Event>, GameError> {
    if state.current_player != player {
        return Err(not_your_turn(player, state.current_player));
    }
    let p = &state.players[player as usize];
    if p.draws_remaining == 0 {
        return Err(GameError::IllegalAction {
            player,
            message: "no draws remaining this turn".into(),
        });
    }
    let Some(entry) = p.current.iter().find(|c| c.card.id == card_id) else {
        return Err(GameError::IllegalAction {
            player,
            message: format!("card {card_id:?} is not in {player}'s own Current"),
        });
    };
    let card = entry.card;
    let mut work = state.clone();
    let mut events = vec![Event::CardDrawnFromCurrent { player, card }];
    apply_event_impl(&mut work, &events[0]);
    if work.players[player as usize].draws_remaining == 0 {
        events.extend(finish_drawing_chain(&mut work, player));
    }
    Ok(events)
}

fn finish_drawing_chain(work: &mut GameState, player: PlayerId) -> Vec<Event> {
    let ev = Event::DrawingFinished { player };
    apply_event_impl(work, &ev);
    vec![ev]
}

fn apply_draw_from_discard_pile(state: &GameState, player: PlayerId) -> Result<Vec<Event>, GameError> {
    if state.current_player != player {
        return Err(not_your_turn(player, state.current_player));
    }
    let p = &state.players[player as usize];
    if p.draws_remaining == 0 {
        return Err(GameError::IllegalAction {
            player,
            message: "no draws remaining this turn".into(),
        });
    }
    if p.porter_used {
        return Err(GameError::IllegalAction {
            player,
            message: "Porter's draw-from-Discard-Pile has already been used this Phase 2".into(),
        });
    }
    if !has_survivor(p, crate::card::SurvivorId::Porter) {
        return Err(GameError::IllegalAction {
            player,
            message: "no Porter on this player's raft".into(),
        });
    }
    let Some(card) = state.discard_pile.last().copied() else {
        return Err(GameError::IllegalAction {
            player,
            message: "the Discard Pile is empty".into(),
        });
    };
    let mut work = state.clone();
    let mut events = vec![Event::DrewFromDiscardPile { player, card }];
    apply_event_impl(&mut work, &events[0]);
    if work.players[player as usize].draws_remaining == 0 {
        events.extend(finish_drawing_chain(&mut work, player));
    }
    Ok(events)
}

fn apply_draw_from_adjacent_current(
    state: &GameState,
    player: PlayerId,
    neighbor: PlayerId,
    card_id: CardInstanceId,
) -> Result<Vec<Event>, GameError> {
    if state.current_player != player {
        return Err(not_your_turn(player, state.current_player));
    }
    let p = &state.players[player as usize];
    if p.draws_remaining == 0 {
        return Err(GameError::IllegalAction {
            player,
            message: "no draws remaining this turn".into(),
        });
    }
    if p.swimmer_used {
        return Err(GameError::IllegalAction {
            player,
            message: "Swimmer's draw-from-adjacent-Current has already been used this Phase 2".into(),
        });
    }
    if !has_survivor(p, crate::card::SurvivorId::Swimmer) {
        return Err(GameError::IllegalAction {
            player,
            message: "no Swimmer on this player's raft".into(),
        });
    }
    if !adjacent_players(state.players.len(), player).contains(&neighbor) {
        return Err(GameError::IllegalAction {
            player,
            message: format!("seat {neighbor} is not adjacent to {player}"),
        });
    }
    let Some(entry) = state.players[neighbor as usize]
        .current
        .iter()
        .find(|c| c.card.id == card_id)
    else {
        return Err(GameError::IllegalAction {
            player,
            message: format!("card {card_id:?} is not in seat {neighbor}'s Current"),
        });
    };
    let card = entry.card;
    let mut work = state.clone();
    let mut events = vec![Event::DrewFromAdjacentCurrent {
        player,
        neighbor,
        card,
    }];
    apply_event_impl(&mut work, &events[0]);
    if work.players[player as usize].draws_remaining == 0 {
        events.extend(finish_drawing_chain(&mut work, player));
    }
    Ok(events)
}

/// Initiate a Pirate steal. Unlike the other special draw sources this
/// doesn't move a card itself — it transitions to
/// `Phase::AwaitingPirateSteal` so `resolve_chance` can pick the
/// actual card via the `Rng` port, which `apply_action` doesn't have
/// access to. `draws_remaining` / `pirate_used` update when the steal
/// actually resolves (`Event::PirateStole`), not here.
fn apply_draw_random_from_hand(
    state: &GameState,
    player: PlayerId,
    target: PlayerId,
) -> Result<Vec<Event>, GameError> {
    if state.current_player != player {
        return Err(not_your_turn(player, state.current_player));
    }
    let p = &state.players[player as usize];
    if p.draws_remaining == 0 {
        return Err(GameError::IllegalAction {
            player,
            message: "no draws remaining this turn".into(),
        });
    }
    if p.pirate_used {
        return Err(GameError::IllegalAction {
            player,
            message: "Pirate's random-steal has already been used this Phase 2".into(),
        });
    }
    if !has_survivor(p, crate::card::SurvivorId::Pirate) {
        return Err(GameError::IllegalAction {
            player,
            message: "no Pirate on this player's raft".into(),
        });
    }
    if target == player {
        return Err(GameError::IllegalAction {
            player,
            message: "Pirate cannot steal from yourself".into(),
        });
    }
    let Some(target_state) = state.players.get(target as usize) else {
        return Err(GameError::IllegalAction {
            player,
            message: format!("unknown target seat {target}"),
        });
    };
    if target_state.hand.is_empty() {
        return Err(GameError::IllegalAction {
            player,
            message: format!("seat {target} has an empty hand"),
        });
    }
    let mut work = state.clone();
    let ev = Event::PirateStealInitiated { player, target };
    apply_event_impl(&mut work, &ev);
    Ok(vec![ev])
}

fn apply_play_survivor(
    state: &GameState,
    player: PlayerId,
    card_id: CardInstanceId,
) -> Result<Vec<Event>, GameError> {
    if state.current_player != player {
        return Err(not_your_turn(player, state.current_player));
    }
    let p = &state.players[player as usize];
    if !actions_available(p) {
        return Err(GameError::IllegalAction {
            player,
            message: "no actions remaining this turn".into(),
        });
    }
    let Some(card) = find_in_hand(&p.hand, card_id) else {
        return Err(GameError::IllegalAction {
            player,
            message: format!("card {card_id:?} is not in {player}'s hand"),
        });
    };
    let CardKind::Survivor(survivor) = card.kind else {
        return Err(GameError::IllegalAction {
            player,
            message: format!("card {card_id:?} is not a survivor"),
        });
    };
    if survivor.occupies_space() && free_spaces(p) == 0 {
        return Err(GameError::IllegalAction {
            player,
            message: "no free raft space to play this survivor".into(),
        });
    }
    let mut work = state.clone();
    let mut events = vec![Event::SurvivorPlayed { player, card }];
    apply_event_impl(&mut work, &events[0]);
    events.extend(maybe_finish_actions(&mut work, player));
    Ok(events)
}

fn apply_build_modification(
    state: &GameState,
    player: PlayerId,
    card_id: CardInstanceId,
) -> Result<Vec<Event>, GameError> {
    if state.current_player != player {
        return Err(not_your_turn(player, state.current_player));
    }
    let p = &state.players[player as usize];
    if !actions_available(p) {
        return Err(GameError::IllegalAction {
            player,
            message: "no actions remaining this turn".into(),
        });
    }
    let Some(card) = find_in_hand(&p.hand, card_id) else {
        return Err(GameError::IllegalAction {
            player,
            message: format!("card {card_id:?} is not in {player}'s hand"),
        });
    };
    let CardKind::Modification(modification) = card.kind else {
        return Err(GameError::IllegalAction {
            player,
            message: format!("card {card_id:?} is not a modification"),
        });
    };
    let cost = modification.cost();
    if !can_afford(&p.hand, cost) {
        return Err(GameError::IllegalAction {
            player,
            message: format!("insufficient resources for {modification:?}"),
        });
    }
    if modification.occupies_space() && free_spaces(p) == 0 {
        return Err(GameError::IllegalAction {
            player,
            message: "no free raft space to build this modification".into(),
        });
    }
    let payment = select_payment(&p.hand, cost);
    let mut work = state.clone();
    let mut events: Vec<Event> = payment
        .into_iter()
        .map(|c| Event::ResourceDiscarded { player, card: c })
        .collect();
    for e in &events {
        apply_event_impl(&mut work, e);
    }
    let built = Event::ModificationBuilt { player, card };
    apply_event_impl(&mut work, &built);
    events.push(built);
    events.extend(maybe_finish_actions(&mut work, player));
    Ok(events)
}

fn apply_build_extension(state: &GameState, player: PlayerId) -> Result<Vec<Event>, GameError> {
    if state.current_player != player {
        return Err(not_your_turn(player, state.current_player));
    }
    let p = &state.players[player as usize];
    if !actions_available(p) {
        return Err(GameError::IllegalAction {
            player,
            message: "no actions remaining this turn".into(),
        });
    }
    let Some(extension) = state.extension_pile.last().copied() else {
        return Err(GameError::IllegalAction {
            player,
            message: "the raft-extension pile is empty".into(),
        });
    };
    let cost = extension_cost();
    if !can_afford(&p.hand, cost) {
        return Err(GameError::IllegalAction {
            player,
            message: "insufficient resources for a raft extension".into(),
        });
    }
    let payment = select_payment(&p.hand, cost);
    let mut work = state.clone();
    let mut events: Vec<Event> = payment
        .into_iter()
        .map(|c| Event::ResourceDiscarded { player, card: c })
        .collect();
    for e in &events {
        apply_event_impl(&mut work, e);
    }
    let built = Event::ExtensionBuilt { player, extension };
    apply_event_impl(&mut work, &built);
    events.push(built);
    events.extend(maybe_finish_actions(&mut work, player));
    Ok(events)
}

// ---------- Unit 4: play_event / activate_telescope / remove_walrus --------

/// Play an event card. Re-derives target legality from `state` rather
/// than trusting the caller — `legal_play_event_actions` is the
/// enumerator, this is the independent check.
#[allow(clippy::too_many_lines, reason = "one target-legality match plus one per-kind dispatch match, both over the same small EventKind enum; splitting would separate closely related validation from its dispatch")]
fn apply_play_event(
    state: &GameState,
    player: PlayerId,
    card_id: CardInstanceId,
    target: crate::action::EventTarget,
) -> Result<Vec<Event>, GameError> {
    use crate::action::EventTarget;
    use crate::card::EventKind;

    if state.current_player != player {
        return Err(not_your_turn(player, state.current_player));
    }
    let p = &state.players[player as usize];
    if !actions_available(p) {
        return Err(GameError::IllegalAction {
            player,
            message: "no actions remaining this turn".into(),
        });
    }
    let Some(card) = find_in_hand(&p.hand, card_id) else {
        return Err(GameError::IllegalAction {
            player,
            message: format!("card {card_id:?} is not in {player}'s hand"),
        });
    };
    let CardKind::Event(kind) = card.kind else {
        return Err(GameError::IllegalAction {
            player,
            message: format!("card {card_id:?} is not an event card"),
        });
    };

    match (kind, target) {
        (EventKind::SharkAttack, EventTarget::Player { target: t }) => {
            if t == player || survivor_count(&state.players[t as usize]) < 2 {
                return Err(GameError::IllegalAction {
                    player,
                    message: format!("{t} is not a legal Shark Attack target"),
                });
            }
        }
        (EventKind::OctopusAttack, EventTarget::Player { target: t }) => {
            if t == player || state.players[t as usize].built_extensions.is_empty() {
                return Err(GameError::IllegalAction {
                    player,
                    message: format!("{t} is not a legal Octopus Attack target"),
                });
            }
        }
        (EventKind::Walrus, EventTarget::Player { target: t }) => {
            if t == player || free_spaces(&state.players[t as usize]) == 0 {
                return Err(GameError::IllegalAction {
                    player,
                    message: format!("{t} is not a legal Walrus target"),
                });
            }
        }
        (EventKind::LoveBoat, EventTarget::Player { target: t }) => {
            if free_spaces(p) == 0 || t == player || survivor_count(&state.players[t as usize]) < 2 {
                return Err(GameError::IllegalAction {
                    player,
                    message: "Love Boat requires your own free space and a target with >=2 survivors".into(),
                });
            }
        }
        (EventKind::Storm | EventKind::WorkDay, EventTarget::None) => {}
        (EventKind::LandSighting, _) => {
            return Err(GameError::IllegalAction {
                player,
                message: "Land Sighting is never played".into(),
            });
        }
        _ => {
            return Err(GameError::IllegalAction {
                player,
                message: format!("target shape does not match event kind {kind:?}"),
            });
        }
    }

    let mut work = state.clone();
    let card_played_ev = Event::EventCardPlayed { player, card, target };
    apply_event_impl(&mut work, &card_played_ev);
    let mut events = vec![card_played_ev];

    match kind {
        EventKind::SharkAttack | EventKind::OctopusAttack | EventKind::Walrus => {
            let EventTarget::Player { target: t } = target else {
                unreachable!("validated above")
            };
            let held_card = (kind == EventKind::Walrus).then_some(card);
            let ev = Event::PendingDecisionOpened {
                player: t,
                decision: PendingDecisionKind::EventReaction {
                    attacker: player,
                    event: kind,
                    held_card,
                },
            };
            apply_event_impl(&mut work, &ev);
            events.push(ev);
        }
        EventKind::LoveBoat => {
            let EventTarget::Player { target: t } = target else {
                unreachable!("validated above")
            };
            let ev = Event::PendingDecisionOpened {
                player: t,
                decision: PendingDecisionKind::LoveBoatChooseSurvivor { attacker: player },
            };
            apply_event_impl(&mut work, &ev);
            events.push(ev);
        }
        EventKind::Storm => {
            let order = storm_order(state.players.len(), player);
            if let Some((&first, rest)) = order.split_first() {
                let ev = Event::PendingDecisionOpened {
                    player: first,
                    decision: PendingDecisionKind::StormChoice {
                        attacker: player,
                        queue_tail: rest.to_vec(),
                    },
                };
                apply_event_impl(&mut work, &ev);
                events.push(ev);
            } else {
                // Unreachable under num_players >= 2, but harmless if
                // it ever weren't: nobody else to resolve against.
                events.extend(maybe_finish_actions(&mut work, player));
            }
        }
        EventKind::WorkDay => {
            let ev = Event::WorkDayActivated { player };
            apply_event_impl(&mut work, &ev);
            events.push(ev);
            events.extend(maybe_finish_actions(&mut work, player));
        }
        EventKind::LandSighting => unreachable!("rejected above"),
    }
    Ok(events)
}

fn apply_activate_telescope(
    state: &GameState,
    player: PlayerId,
    card_id: CardInstanceId,
) -> Result<Vec<Event>, GameError> {
    if state.current_player != player {
        return Err(not_your_turn(player, state.current_player));
    }
    let p = &state.players[player as usize];
    if !actions_available(p) {
        return Err(GameError::IllegalAction {
            player,
            message: "no actions remaining this turn".into(),
        });
    }
    let Some(telescope) = p.placed.iter().find(|pc| {
        pc.card.id == card_id
            && matches!(
                pc.card.kind,
                CardKind::Modification(crate::card::ModificationKind::Telescope)
            )
    }) else {
        return Err(GameError::IllegalAction {
            player,
            message: format!("{card_id:?} is not a built Telescope on {player}'s raft"),
        });
    };
    let telescope = telescope.card;
    let Some(drawn) = state.event_deck.last().copied() else {
        return Err(GameError::IllegalAction {
            player,
            message: "the Event Deck is empty".into(),
        });
    };
    let mut work = state.clone();
    let ev = Event::TelescopeActivated {
        player,
        telescope,
        drawn,
    };
    apply_event_impl(&mut work, &ev);
    let mut events = vec![ev];
    events.extend(maybe_finish_actions(&mut work, player));
    Ok(events)
}

fn apply_remove_walrus(
    state: &GameState,
    player: PlayerId,
    dead_fish_id: CardInstanceId,
    walrus_id: CardInstanceId,
) -> Result<Vec<Event>, GameError> {
    if state.current_player != player {
        return Err(not_your_turn(player, state.current_player));
    }
    let p = &state.players[player as usize];
    if !actions_available(p) {
        return Err(GameError::IllegalAction {
            player,
            message: "no actions remaining this turn".into(),
        });
    }
    let Some(dead_fish) = find_in_hand(&p.hand, dead_fish_id).filter(|c| matches!(c.kind, CardKind::DeadFish)) else {
        return Err(GameError::IllegalAction {
            player,
            message: format!("{dead_fish_id:?} is not a Dead Fish in {player}'s hand"),
        });
    };
    let Some(walrus) = p.blocked_by_walrus.iter().find(|c| c.id == walrus_id).copied() else {
        return Err(GameError::IllegalAction {
            player,
            message: format!("{walrus_id:?} is not blocking a space on {player}'s raft"),
        });
    };
    let mut work = state.clone();
    let ev = Event::WalrusRemoved {
        player,
        dead_fish,
        walrus,
    };
    apply_event_impl(&mut work, &ev);
    let mut events = vec![ev];
    events.extend(maybe_finish_actions(&mut work, player));
    Ok(events)
}

fn apply_work_day_draw(
    state: &GameState,
    player: PlayerId,
    card_id: CardInstanceId,
) -> Result<Vec<Event>, GameError> {
    if state.current_player != player {
        return Err(not_your_turn(player, state.current_player));
    }
    let p = &state.players[player as usize];
    if !p.work_day_active {
        return Err(GameError::IllegalAction {
            player,
            message: "Work Day is not active this turn".into(),
        });
    }
    let Some(entry) = p.current.iter().find(|c| c.card.id == card_id) else {
        return Err(GameError::IllegalAction {
            player,
            message: format!("card {card_id:?} is not in {player}'s own Current"),
        });
    };
    let card = entry.card;
    let mut work = state.clone();
    let ev = Event::WorkDayDrew { player, card };
    apply_event_impl(&mut work, &ev);
    Ok(vec![ev])
}

/// End Phase 3: emit the marker event, then either open a hand-limit
/// discard decision or continue straight to Phase 4.
fn finish_actions_chain(work: &mut GameState, player: PlayerId) -> Vec<Event> {
    let mut events = vec![Event::ActionsFinished { player }];
    apply_event_impl(work, &events[0]);

    let p = &work.players[player as usize];
    let hand_limit = compute_hand_limit(p);
    let hand_len = u32::try_from(p.hand.len()).unwrap_or(u32::MAX);
    if hand_len > hand_limit {
        let needed = u8::try_from(hand_len - hand_limit).unwrap_or(u8::MAX);
        let ev = Event::PendingDecisionOpened {
            player,
            decision: PendingDecisionKind::DiscardDown { needed },
        };
        apply_event_impl(work, &ev);
        events.push(ev);
        return events;
    }
    events.extend(after_hand_ok_chain(work, player));
    events
}

/// Phase 4: compute food, then either auto-resolve or open the
/// appropriate hungry/stand-up decision.
fn after_hand_ok_chain(work: &mut GameState, player: PlayerId) -> Vec<Event> {
    let mut events = Vec::new();
    let p = &work.players[player as usize];
    let food = compute_food(p);
    let standing: Vec<Card> = p
        .placed
        .iter()
        .filter(|pc| !pc.hungry && matches!(pc.card.kind, CardKind::Survivor(_)))
        .map(|pc| pc.card)
        .collect();
    let hungry_list: Vec<Card> = p
        .placed
        .iter()
        .filter(|pc| pc.hungry)
        .map(|pc| pc.card)
        .collect();

    if food < 0 {
        let deficit = u32::try_from(-food).unwrap_or(u32::MAX);
        let standing_len = u32::try_from(standing.len()).unwrap_or(u32::MAX);
        if standing_len >= deficit {
            if standing_len > deficit {
                // Real choice: pick `deficit` of the standing survivors.
                let needed = u8::try_from(deficit).unwrap_or(u8::MAX);
                let ev = Event::PendingDecisionOpened {
                    player,
                    decision: PendingDecisionKind::MakeHungry { needed },
                };
                apply_event_impl(work, &ev);
                events.push(ev);
                return events;
            }
            // Forced: exactly `deficit` standing survivors, flip them all.
            for c in &standing {
                let ev = Event::SurvivorMadeHungry {
                    player,
                    survivor: *c,
                };
                apply_event_impl(work, &ev);
                events.push(ev);
            }
        } else {
            // Not enough standing survivors: forced-flip all of them,
            // then cover the remaining shortfall by abandoning
            // already-Hungry survivors.
            for c in &standing {
                let ev = Event::SurvivorMadeHungry {
                    player,
                    survivor: *c,
                };
                apply_event_impl(work, &ev);
                events.push(ev);
            }
            let shortfall = deficit - standing_len;
            let mut hungry_pool = hungry_list.clone();
            hungry_pool.extend(standing.iter().copied());
            let pool_len = u32::try_from(hungry_pool.len()).unwrap_or(u32::MAX);
            if shortfall > 0 {
                if pool_len > shortfall {
                    let needed = u8::try_from(shortfall).unwrap_or(u8::MAX);
                    let ev = Event::PendingDecisionOpened {
                        player,
                        decision: PendingDecisionKind::AbandonHungry { needed },
                    };
                    apply_event_impl(work, &ev);
                    events.push(ev);
                    return events;
                }
                for c in &hungry_pool {
                    let ev = Event::SurvivorAbandoned {
                        player,
                        survivor: *c,
                    };
                    apply_event_impl(work, &ev);
                    events.push(ev);
                }
            }
        }
    } else {
        let hungry_count = u32::try_from(hungry_list.len()).unwrap_or(u32::MAX);
        if food > 0 && hungry_count > 0 {
            let food_u = u32::try_from(food).unwrap_or(u32::MAX);
            if food_u >= hungry_count {
                for c in &hungry_list {
                    let ev = Event::SurvivorStoodUp {
                        player,
                        survivor: *c,
                    };
                    apply_event_impl(work, &ev);
                    events.push(ev);
                }
            } else {
                let needed = u8::try_from(food_u).unwrap_or(u8::MAX);
                let ev = Event::PendingDecisionOpened {
                    player,
                    decision: PendingDecisionKind::StandUp { needed },
                };
                apply_event_impl(work, &ev);
                events.push(ev);
                return events;
            }
        }
    }

    events.extend(end_of_turn_chain(work, player));
    events
}

fn apply_resolve_decision(
    state: &GameState,
    player: PlayerId,
    choice: DecisionChoice,
) -> Result<Vec<Event>, GameError> {
    let Some(top) = state.current_pending() else {
        return Err(GameError::IllegalAction {
            player,
            message: "no pending decision is open".into(),
        });
    };
    if top.player != player {
        return Err(GameError::IllegalAction {
            player,
            message: format!("pending decision belongs to {}, not {player}", top.player),
        });
    }
    let kind = top.kind.clone();
    match kind {
        PendingDecisionKind::DiscardDown { .. }
        | PendingDecisionKind::MakeHungry { .. }
        | PendingDecisionKind::AbandonHungry { .. }
        | PendingDecisionKind::StandUp { .. } => apply_resolve_u2_decision(state, player, &kind, choice),
        PendingDecisionKind::EventReaction {
            attacker,
            event,
            held_card,
        } => apply_event_reaction_choice(state, player, attacker, event, held_card, choice),
        PendingDecisionKind::SharkChooseSurvivor { attacker } => {
            apply_shark_choose_survivor(state, player, attacker, choice)
        }
        PendingDecisionKind::OctopusRelocate { attacker, .. } => {
            apply_octopus_relocate_choice(state, player, attacker, choice)
        }
        PendingDecisionKind::LoveBoatChooseSurvivor { attacker } => {
            apply_love_boat_choose_survivor(state, player, attacker, choice)
        }
        PendingDecisionKind::StormChoice {
            attacker,
            queue_tail,
        } => apply_storm_choice(state, player, attacker, queue_tail, choice),
        PendingDecisionKind::StormDiscard {
            attacker,
            queue_tail,
            ..
        } => apply_storm_discard_choice(state, player, attacker, queue_tail, choice),
    }
}

/// The four Unit 2 decision kinds: hand-limit discard and Phase-4
/// hungry/stand-up. Unchanged from Unit 2 except for the `kind` clone
/// (`PendingDecisionKind` lost `Copy` once Storm's `Vec<PlayerId>`
/// payload was added in Unit 4).
fn apply_resolve_u2_decision(
    state: &GameState,
    player: PlayerId,
    kind: &PendingDecisionKind,
    choice: DecisionChoice,
) -> Result<Vec<Event>, GameError> {
    let p = &state.players[player as usize];
    let core_event = match (kind, choice) {
        (PendingDecisionKind::DiscardDown { .. }, DecisionChoice::Discard { card }) => {
            let card = find_in_hand(&p.hand, card).ok_or_else(|| GameError::IllegalAction {
                player,
                message: format!("card {card:?} is not in {player}'s hand"),
            })?;
            Event::CardDiscarded { player, card }
        }
        (PendingDecisionKind::MakeHungry { .. }, DecisionChoice::MakeHungry { survivor }) => {
            let card = find_standing_survivor(p, survivor).ok_or_else(|| GameError::IllegalAction {
                player,
                message: format!("survivor {survivor:?} is not a standing survivor on {player}'s raft"),
            })?;
            Event::SurvivorMadeHungry {
                player,
                survivor: card,
            }
        }
        (PendingDecisionKind::AbandonHungry { .. }, DecisionChoice::AbandonHungry { survivor }) => {
            let card = find_hungry_survivor(p, survivor).ok_or_else(|| GameError::IllegalAction {
                player,
                message: format!("survivor {survivor:?} is not a Hungry survivor on {player}'s raft"),
            })?;
            Event::SurvivorAbandoned {
                player,
                survivor: card,
            }
        }
        (PendingDecisionKind::StandUp { .. }, DecisionChoice::StandUp { survivor }) => {
            let card = find_hungry_survivor(p, survivor).ok_or_else(|| GameError::IllegalAction {
                player,
                message: format!("survivor {survivor:?} is not a Hungry survivor on {player}'s raft"),
            })?;
            Event::SurvivorStoodUp {
                player,
                survivor: card,
            }
        }
        _ => {
            return Err(GameError::IllegalAction {
                player,
                message: "decision choice does not match the open decision kind".into(),
            });
        }
    };

    let mut work = state.clone();
    let mut events = vec![core_event];
    apply_event_impl(&mut work, &events[0]);

    if work.pending_decisions.is_empty() {
        match kind {
            PendingDecisionKind::DiscardDown { .. } => {
                events.extend(after_hand_ok_chain(&mut work, player));
            }
            PendingDecisionKind::MakeHungry { .. }
            | PendingDecisionKind::AbandonHungry { .. }
            | PendingDecisionKind::StandUp { .. } => {
                events.extend(end_of_turn_chain(&mut work, player));
            }
            _ => unreachable!("apply_resolve_u2_decision is only called with a Unit-2 decision kind"),
        }
    }
    Ok(events)
}

// ---------- Unit 4: event reaction / effect decision resolution ------------

/// `player` (the target) answers the reaction window opened by
/// `attacker`'s Shark/Octopus/Walrus play.
fn apply_event_reaction_choice(
    state: &GameState,
    player: PlayerId,
    attacker: PlayerId,
    event_kind: crate::card::EventKind,
    held_card: Option<Card>,
    choice: DecisionChoice,
) -> Result<Vec<Event>, GameError> {
    let p = &state.players[player as usize];
    let mut work = state.clone();
    let mut events = Vec::new();

    let negated = match choice {
        DecisionChoice::DeclineReaction => {
            let ev = Event::ReactionDeclined { player };
            apply_event_impl(&mut work, &ev);
            events.push(ev);
            false
        }
        DecisionChoice::ReactWithDeadFish { card } => {
            let card = find_in_hand(&p.hand, card)
                .filter(|c| matches!(c.kind, CardKind::DeadFish))
                .ok_or_else(|| GameError::IllegalAction {
                    player,
                    message: "card is not a Dead Fish in hand".into(),
                })?;
            let ev = Event::ReactedWithDeadFish { player, card };
            apply_event_impl(&mut work, &ev);
            events.push(ev);
            true
        }
        DecisionChoice::ReactWithFisher { card } => {
            if event_kind == crate::card::EventKind::Walrus {
                return Err(GameError::IllegalAction {
                    player,
                    message: "Fisher does not negate Walrus".into(),
                });
            }
            if !has_fisher(p) {
                return Err(GameError::IllegalAction {
                    player,
                    message: "no Fisher on this player's raft".into(),
                });
            }
            let card = find_in_hand(&p.hand, card).ok_or_else(|| GameError::IllegalAction {
                player,
                message: format!("card {card:?} is not in {player}'s hand"),
            })?;
            let ev = Event::ReactedWithFisher { player, card };
            apply_event_impl(&mut work, &ev);
            events.push(ev);
            true
        }
        _ => {
            return Err(GameError::IllegalAction {
                player,
                message: "expected a reaction choice (Dead Fish, Fisher, or decline)".into(),
            });
        }
    };

    // `EventReaction` is a single-shot decision; the reaction event
    // applied just above (`ReactionDeclined` / `ReactedWithDeadFish` /
    // `ReactedWithFisher`) already popped it via `pop_pending` — that's
    // *in* the event's own `apply_event`, so replay reproduces it too
    // (a bare `work.pending_decisions.pop()` here would only mutate
    // this throwaway clone, never the real state — the bug this
    // helper's doc comment describes).

    if negated {
        if let Some(walrus_card) = held_card {
            let ev = Event::WalrusDiscarded {
                player: attacker,
                card: walrus_card,
            };
            apply_event_impl(&mut work, &ev);
            events.push(ev);
        }
        events.extend(resume_actions_chain(&mut work, attacker));
        return Ok(events);
    }

    match event_kind {
        crate::card::EventKind::SharkAttack => {
            let ev = Event::PendingDecisionOpened {
                player,
                decision: PendingDecisionKind::SharkChooseSurvivor { attacker },
            };
            apply_event_impl(&mut work, &ev);
            events.push(ev);
        }
        crate::card::EventKind::OctopusAttack => {
            events.extend(apply_octopus_effect(&mut work, player, attacker));
        }
        crate::card::EventKind::Walrus => {
            let walrus_card = held_card.expect("Walrus's EventReaction always carries held_card");
            let ev = Event::WalrusPlaced {
                player,
                attacker,
                card: walrus_card,
            };
            apply_event_impl(&mut work, &ev);
            events.push(ev);
            events.extend(resume_actions_chain(&mut work, attacker));
        }
        _ => unreachable!("only Shark/Octopus/Walrus open an EventReaction decision"),
    }
    Ok(events)
}

/// Octopus Attack's non-reaction-window effect: lose one (fungible)
/// extension, then — if that drops capacity below current occupancy —
/// open the relocation decision.
fn apply_octopus_effect(work: &mut GameState, player: PlayerId, attacker: PlayerId) -> Vec<Event> {
    let idx = player as usize;
    let extension = work.players[idx]
        .built_extensions
        .last()
        .copied()
        .expect("OctopusAttack was only legal because the target has >=1 extension");
    let ev = Event::ExtensionLostToOctopus { player, extension };
    apply_event_impl(work, &ev);
    let mut events = vec![ev];

    let (used, total) = crate::turns::raft_capacity(&work.players[idx]);
    let deficit = used.saturating_sub(total);
    if deficit > 0 {
        let placed_len = u32::try_from(work.players[idx].placed.len()).unwrap_or(u32::MAX);
        let needed = u8::try_from(deficit.min(placed_len)).unwrap_or(u8::MAX);
        if needed > 0 {
            let ev = Event::PendingDecisionOpened {
                player,
                decision: PendingDecisionKind::OctopusRelocate { needed, attacker },
            };
            apply_event_impl(work, &ev);
            events.push(ev);
            return events;
        }
    }
    events.extend(resume_actions_chain(work, attacker));
    events
}

fn apply_shark_choose_survivor(
    state: &GameState,
    player: PlayerId,
    attacker: PlayerId,
    choice: DecisionChoice,
) -> Result<Vec<Event>, GameError> {
    let DecisionChoice::LoseSurvivorToShark { survivor } = choice else {
        return Err(GameError::IllegalAction {
            player,
            message: "expected LoseSurvivorToShark".into(),
        });
    };
    let p = &state.players[player as usize];
    let card = p
        .placed
        .iter()
        .find(|pc| pc.card.id == survivor && matches!(pc.card.kind, CardKind::Survivor(_)))
        .map(|pc| pc.card)
        .ok_or_else(|| GameError::IllegalAction {
            player,
            message: format!("survivor {survivor:?} is not on {player}'s raft"),
        })?;
    let mut work = state.clone();
    let ev = Event::SurvivorLostToShark { player, survivor: card };
    apply_event_impl(&mut work, &ev); // also pops the SharkChooseSurvivor decision
    let mut events = vec![ev];
    events.extend(resume_actions_chain(&mut work, attacker));
    Ok(events)
}

fn apply_octopus_relocate_choice(
    state: &GameState,
    player: PlayerId,
    attacker: PlayerId,
    choice: DecisionChoice,
) -> Result<Vec<Event>, GameError> {
    let DecisionChoice::RelocateFromOctopus { card } = choice else {
        return Err(GameError::IllegalAction {
            player,
            message: "expected RelocateFromOctopus".into(),
        });
    };
    let p = &state.players[player as usize];
    let found = p
        .placed
        .iter()
        .find(|pc| pc.card.id == card)
        .map(|pc| pc.card)
        .ok_or_else(|| GameError::IllegalAction {
            player,
            message: format!("card {card:?} is not on {player}'s raft"),
        })?;
    let mut work = state.clone();
    let ev = Event::RelocatedFromOctopus { player, card: found };
    apply_event_impl(&mut work, &ev);
    let mut events = vec![ev];
    if work.pending_decisions.is_empty() {
        events.extend(resume_actions_chain(&mut work, attacker));
    }
    Ok(events)
}

fn apply_love_boat_choose_survivor(
    state: &GameState,
    player: PlayerId,
    attacker: PlayerId,
    choice: DecisionChoice,
) -> Result<Vec<Event>, GameError> {
    let DecisionChoice::GiveSurvivorToLoveBoat { survivor } = choice else {
        return Err(GameError::IllegalAction {
            player,
            message: "expected GiveSurvivorToLoveBoat".into(),
        });
    };
    let p = &state.players[player as usize];
    let card = p
        .placed
        .iter()
        .find(|pc| pc.card.id == survivor && matches!(pc.card.kind, CardKind::Survivor(_)))
        .map(|pc| pc.card)
        .ok_or_else(|| GameError::IllegalAction {
            player,
            message: format!("survivor {survivor:?} is not on {player}'s raft"),
        })?;
    let mut work = state.clone();
    let ev = Event::SurvivorGivenToLoveBoat {
        player,
        recipient: attacker,
        survivor: card,
    };
    apply_event_impl(&mut work, &ev); // also pops the LoveBoatChooseSurvivor decision
    let mut events = vec![ev];
    events.extend(resume_actions_chain(&mut work, attacker));
    Ok(events)
}

fn apply_storm_choice(
    state: &GameState,
    player: PlayerId,
    attacker: PlayerId,
    queue_tail: Vec<PlayerId>,
    choice: DecisionChoice,
) -> Result<Vec<Event>, GameError> {
    let mut work = state.clone();
    let mut events = Vec::new();
    match choice {
        DecisionChoice::StormRemoveCard { card } => {
            let p = &state.players[player as usize];
            let found = p
                .placed
                .iter()
                .find(|pc| pc.card.id == card)
                .map(|pc| pc.card)
                .ok_or_else(|| GameError::IllegalAction {
                    player,
                    message: format!("card {card:?} is not on {player}'s raft"),
                })?;
            let ev = Event::StormCardRemoved { player, card: found };
            apply_event_impl(&mut work, &ev); // also pops the StormChoice decision
            events.push(ev);
            events.extend(advance_storm_or_finish(&mut work, attacker, queue_tail));
        }
        DecisionChoice::StormTakeDiscardRoute => {
            let ev = Event::StormDiscardRouteTaken { player };
            apply_event_impl(&mut work, &ev); // also pops the StormChoice decision
            events.push(ev);
            let eligible: Vec<Card> = work.players[player as usize]
                .hand
                .iter()
                .filter(|c| !matches!(c.kind, CardKind::Event(_)))
                .copied()
                .collect();
            if eligible.len() > 2 {
                // A real choice: which 2 of >2 eligible cards.
                let ev2 = Event::PendingDecisionOpened {
                    player,
                    decision: PendingDecisionKind::StormDiscard {
                        needed: 2,
                        attacker,
                        queue_tail: queue_tail.clone(),
                    },
                };
                apply_event_impl(&mut work, &ev2);
                events.push(ev2);
            } else {
                // Forced: discard every eligible card (0, 1, or exactly
                // 2) — no real choice of *which*, so no decision needed,
                // matching the Phase-4 hungry/stand-up forced-case
                // pattern ("discard what they can").
                for c in eligible {
                    let ev3 = Event::CardDiscarded { player, card: c };
                    apply_event_impl(&mut work, &ev3);
                    events.push(ev3);
                }
                events.extend(advance_storm_or_finish(&mut work, attacker, queue_tail));
            }
        }
        _ => {
            return Err(GameError::IllegalAction {
                player,
                message: "expected a Storm choice (remove a card, or take the discard route)".into(),
            });
        }
    }
    Ok(events)
}

fn apply_storm_discard_choice(
    state: &GameState,
    player: PlayerId,
    attacker: PlayerId,
    queue_tail: Vec<PlayerId>,
    choice: DecisionChoice,
) -> Result<Vec<Event>, GameError> {
    let DecisionChoice::StormDiscard { card } = choice else {
        return Err(GameError::IllegalAction {
            player,
            message: "expected StormDiscard".into(),
        });
    };
    let p = &state.players[player as usize];
    let card = find_in_hand(&p.hand, card)
        .filter(|c| !matches!(c.kind, CardKind::Event(_)))
        .ok_or_else(|| GameError::IllegalAction {
            player,
            message: format!("card {card:?} is not an eligible (non-event) hand card"),
        })?;
    let mut work = state.clone();
    // Reuses `CardDiscarded` (hand -> own Current, face-down) — the
    // effect is identical to hand-limit discard-down.
    let ev = Event::CardDiscarded { player, card };
    apply_event_impl(&mut work, &ev);
    let mut events = vec![ev];
    if work.pending_decisions.is_empty() {
        events.extend(advance_storm_or_finish(&mut work, attacker, queue_tail));
    }
    Ok(events)
}

/// Move Storm's around-the-table resolution to the next player in
/// `queue_tail`, or (empty) return control to `attacker`.
fn advance_storm_or_finish(
    work: &mut GameState,
    attacker: PlayerId,
    mut queue_tail: Vec<PlayerId>,
) -> Vec<Event> {
    if queue_tail.is_empty() {
        return resume_actions_chain(work, attacker);
    }
    let next = queue_tail.remove(0);
    let ev = Event::PendingDecisionOpened {
        player: next,
        decision: PendingDecisionKind::StormChoice {
            attacker,
            queue_tail,
        },
    };
    apply_event_impl(work, &ev);
    vec![ev]
}

/// Whether `player` may take a Phase-3 action right now: either a
/// normal action remains, or Work Day made this turn's actions
/// unlimited.
fn actions_available(p: &crate::state::PlayerState) -> bool {
    p.work_day_active || p.actions_remaining > 0
}

/// After a Phase-3 action that didn't open an interrupt (a build, a
/// survivor play, Work Day's own activation, Telescope, Walrus
/// removal): if the action budget just hit 0 and Work Day isn't
/// active, auto-transition out of Phase 3 exactly like Unit 2's
/// direct actions did. No-op (and no `EventResolved`) otherwise —
/// unlike `resume_actions_chain`, this never left `Phase::Actions`.
fn maybe_finish_actions(work: &mut GameState, player: PlayerId) -> Vec<Event> {
    if actions_available(&work.players[player as usize]) {
        Vec::new()
    } else {
        finish_actions_chain(work, player)
    }
}

/// The event-interrupt chain (reaction + effect, or the multi-player
/// Storm chain) fully resolved: return `attacker` to `Phase::Actions`
/// and, if their budget is now exhausted, continue straight into the
/// same end-of-Phase-3 chain a direct action would have triggered.
fn resume_actions_chain(work: &mut GameState, attacker: PlayerId) -> Vec<Event> {
    let ev = Event::EventResolved { player: attacker };
    apply_event_impl(work, &ev);
    let mut events = vec![ev];
    events.extend(maybe_finish_actions(work, attacker));
    events
}

fn find_standing_survivor(p: &crate::state::PlayerState, id: CardInstanceId) -> Option<Card> {
    p.placed
        .iter()
        .find(|pc| pc.card.id == id && !pc.hungry && matches!(pc.card.kind, CardKind::Survivor(_)))
        .map(|pc| pc.card)
}

fn find_hungry_survivor(p: &crate::state::PlayerState, id: CardInstanceId) -> Option<Card> {
    p.placed
        .iter()
        .find(|pc| pc.card.id == id && pc.hungry)
        .map(|pc| pc.card)
}

/// End of this player's turn segment: advance to the next seat in the
/// round, or (round complete) either pass Currents into a new round or
/// end the game.
///
/// Turn order within a round is derived arithmetically —
/// `(player + 1) % n`, wrapping back to `first_player` marks the round
/// complete — rather than tracked in a separate mutable queue. A queue
/// would need its own event to stay in sync across replay; deriving it
/// from `current_player` / `first_player` (both already event-sourced
/// via `TurnStarted` / `CurrentsPassed`) sidesteps that whole class of
/// bug.
fn end_of_turn_chain(work: &mut GameState, player: PlayerId) -> Vec<Event> {
    let mut events = vec![Event::TurnEnded { player }];
    apply_event_impl(work, &events[0]);

    let n = work.players.len();
    let next = u8::try_from((usize::from(player) + 1) % n).expect("seat fits in u8");
    if next != work.first_player {
        events.extend(begin_turn_chain(work, next));
        return events;
    }

    if work.final_round {
        let (result, winners, scores) = crate::scoring::build_game_result(work);
        let ev = Event::EndGame {
            winners,
            reason: result.reason,
            final_scores: scores,
        };
        apply_event_impl(work, &ev);
        events.push(ev);
        return events;
    }

    // `next == work.first_player` here (that's what "round complete"
    // means), so the new First Player is one more seat along.
    let new_first = u8::try_from((usize::from(next) + 1) % n).expect("seat fits in u8");
    let pass = Event::CurrentsPassed {
        new_first_player: new_first,
    };
    apply_event_impl(work, &pass);
    events.push(pass);
    events.extend(begin_turn_chain(work, new_first));
    events
}

/// Begin a new turn segment for `player`: Phase 1's automatic add,
/// including the Final Round trigger when the Deep Sea Deck runs dry.
fn begin_turn_chain(work: &mut GameState, player: PlayerId) -> Vec<Event> {
    let mut events = vec![Event::TurnStarted { player }];
    apply_event_impl(work, &events[0]);

    // `1 + add_bonus` cards (Sail +1 each, Millionaire +1). Raft
    // composition can't change mid-add (Phase 1 only touches the
    // Current pile), so it's safe to compute the count once.
    let add_count = 1 + add_bonus(&work.players[player as usize]);
    for _ in 0..add_count {
        let Some((card, triggers_final_round)) = peek_next_add_card(work) else {
            break; // Both decks exhausted — nothing left to add.
        };
        if triggers_final_round {
            let ev = Event::FinalRoundTriggered;
            apply_event_impl(work, &ev);
            events.push(ev);
        }
        let ev = Event::CurrentCardAdded { player, card };
        apply_event_impl(work, &ev);
        events.push(ev);
    }
    events
}

/// What Phase 1's next add would draw, without mutating state: the top
/// of the Deep Sea Deck, or (if empty) the top of the Final Round Deck
/// — which also triggers the Final Round unless it's already active.
/// `None` only in the pathological case where both piles are empty
/// (never expected under the spec's deck sizing, but handled rather
/// than panicking so a soak test surfaces it as a shape anomaly, not a
/// crash).
fn peek_next_add_card(work: &GameState) -> Option<(Card, bool)> {
    if let Some(&card) = work.deep_sea_deck.last() {
        return Some((card, false));
    }
    let triggers_final_round = !work.final_round;
    work.final_round_deck
        .last()
        .copied()
        .map(|card| (card, triggers_final_round))
}
