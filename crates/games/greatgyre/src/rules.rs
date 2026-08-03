//! `GreatGyreGame` — the top-level [`playtest_core::Game`] implementation.
//!
//! Unit 1 landed `Phase::SurvivorDraft` (players pick a survivor in
//! seat order) and `Phase::AwaitingPostDraftShuffle` (the one
//! `resolve_chance` step Great Gyre needs — see `Event::PostDraftSetup`
//! and `setup.rs`'s doc comment for why the shuffle can't happen inside
//! `initial_state`).
//!
//! Unit 2 (this unit) lands the five-phase turn machine:
//! `Phase::Draw` (Phase 2), `Phase::Actions` (Phase 3), and
//! `Phase::ResolvingDecision` (hand-limit discard-down and Phase 4
//! hungry/stand-up choices). Phases 1 and 5 have no player decisions
//! per the spec, so they never appear as a `Phase` variant a player is
//! prompted in — they're folded directly into the event batch that
//! ends the preceding decision (see `begin_turn_chain` /
//! `end_of_turn_chain` below), exactly the "no no-op prompts" design
//! decision from the plan. No survivor ability beyond its printed stat
//! tabs is modeled yet (Unit 3), and no event-card effects are wired
//! up (Unit 4) — `play_event` is not a legal action.

use playtest_core::{Actor, Game, GameError, GameResult, PlayerId};
use playtest_ports::Rng;

use crate::action::{Action, DecisionChoice};
use crate::card::{Card, CardInstanceId, CardKind};
use crate::config::GreatGyreConfig;
use crate::event::Event;
use crate::public_view::{GreatGyrePublicView, public_view as build_public_view};
use crate::state::{CurrentCard, Face, GameState, PendingDecision, PendingDecisionKind, Phase, PlacedCard};
use crate::turns::{can_afford, compute_food, compute_hand_limit, extension_cost, find_in_hand, free_spaces, select_payment};

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
            Phase::Draw => legal_draw_actions(state, player),
            Phase::Actions => legal_action_phase_actions(state, player),
            Phase::ResolvingDecision => legal_decision_actions(state, player),
            Phase::AwaitingPostDraftShuffle | Phase::Finished => Vec::new(),
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
            Phase::AwaitingPostDraftShuffle | Phase::Finished => Err(GameError::IllegalAction {
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

        // ---------- turn machine (Unit 2) ----------------------------
        Event::TurnStarted { player } => {
            let idx = *player as usize;
            state.current_player = *player;
            state.players[idx].draws_remaining = 1;
            state.players[idx].actions_remaining = 1;
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
        Event::DrawingFinished { player } => {
            state.phase = Phase::Actions;
            let idx = *player as usize;
            if state.players[idx].actions_remaining == 0 {
                state.players[idx].actions_remaining = 1;
            }
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
        Event::ResourceDiscarded { player, card } => {
            let idx = *player as usize;
            remove_from_hand(&mut state.players[idx].hand, card.id);
            state.discard_pile.push(*card);
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
            state.pending_decisions.push(PendingDecision {
                player: *player,
                kind: *decision,
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
            decrement_pending(state, *player, |k| matches!(k, PendingDecisionKind::DiscardDown { .. }));
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
            // pending-decision open, Phase 4 auto-resolution, the
            // following `TurnStarted`, or `EndGame`).
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
    top.kind = match top.kind {
        PendingDecisionKind::DiscardDown { .. } => PendingDecisionKind::DiscardDown { needed: remaining },
        PendingDecisionKind::MakeHungry { .. } => PendingDecisionKind::MakeHungry { needed: remaining },
        PendingDecisionKind::AbandonHungry { .. } => {
            PendingDecisionKind::AbandonHungry { needed: remaining }
        }
        PendingDecisionKind::StandUp { .. } => PendingDecisionKind::StandUp { needed: remaining },
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
    if p.actions_remaining > 0 {
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
    }
    out.push(Action::FinishActions);
    out
}

// ---------- ResolvingDecision (discard-down / Phase 4) ---------------------

fn legal_decision_actions(state: &GameState, player: PlayerId) -> Vec<Action> {
    let Some(top) = state.current_pending() else {
        return Vec::new();
    };
    if top.player != player {
        return Vec::new();
    }
    let p = &state.players[player as usize];
    match top.kind {
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

fn apply_play_survivor(
    state: &GameState,
    player: PlayerId,
    card_id: CardInstanceId,
) -> Result<Vec<Event>, GameError> {
    if state.current_player != player {
        return Err(not_your_turn(player, state.current_player));
    }
    let p = &state.players[player as usize];
    if p.actions_remaining == 0 {
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
    if work.players[player as usize].actions_remaining == 0 {
        events.extend(finish_actions_chain(&mut work, player));
    }
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
    if p.actions_remaining == 0 {
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
    if work.players[player as usize].actions_remaining == 0 {
        events.extend(finish_actions_chain(&mut work, player));
    }
    Ok(events)
}

fn apply_build_extension(state: &GameState, player: PlayerId) -> Result<Vec<Event>, GameError> {
    if state.current_player != player {
        return Err(not_your_turn(player, state.current_player));
    }
    let p = &state.players[player as usize];
    if p.actions_remaining == 0 {
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
    if work.players[player as usize].actions_remaining == 0 {
        events.extend(finish_actions_chain(&mut work, player));
    }
    Ok(events)
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
    let kind = top.kind;
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
        }
    }
    Ok(events)
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

    if let Some((card, triggers_final_round)) = peek_next_add_card(work) {
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
