//! `ShipWreckGame` — the top-level [`playtest_core::Game`]
//! implementation for ShipWreck.
//!
//! **Unit 22 scope**: the state machine moves between `Phase::Play`
//! and `Phase::Finished` only. `Phase::Setup` is ephemeral (consumed
//! during `initial_state`) and `Phase::ResolvingEvent` is Unit 23
//! territory.
//!
//! **Event-card resolution is deliberately excluded**:
//! `Action::PlayEventCard` and `Action::ResolveEvent` are never
//! enumerated by `legal_actions` and are rejected by `apply_action`
//! with `GameError::IllegalAction`. Unit 23 adds them.
//!
//! ## Design decisions surfaced here
//!
//! - `apply_action` *always* produces `EndTurn` events via the
//!   `EndTurn` action; end-of-turn is never auto-triggered. This
//!   keeps the engine's turn boundary explicit and lets a random
//!   agent's self-play reliably pass even when `legal_actions` is
//!   short (EndTurn is always legal).
//! - Food consumption at `EndTurn` consumes `food_cost` from the
//!   food counter per placed player card, in player-card order. If
//!   the counter cannot cover a card's cost, that card is dropped
//!   (fell off the raft) and `FoodConsumed { starved: true }` is
//!   emitted. The counter clamps at 0.
//! - `game_over` fires when the wreckage deck is empty AND every
//!   face-up pool is empty AND no pending event chain remains AND we
//!   are in `Phase::Play`. The face-down deck is empty after setup,
//!   so the condition reduces to "every face-up pool is empty." The
//!   engine applies the `EndGame` event on the *next* loop iteration
//!   after `game_over` reports `Some`.

use playtest_core::{Actor, EndReason, Game, GameError, GameResult, PlayerId};
use playtest_ports::Rng;

use crate::action::Action;
use crate::card::{Card, PlayerCardId};
use crate::config::ShipWreckConfig;
use crate::event::{Event, PlayerScore};
use crate::phase::Phase;
use crate::public_view::{ShipWreckPublicView, public_view as build_public_view};
use crate::raft::SlotId;
use crate::resource::{Resource, ResourceCost};
use crate::setup::build_initial_state;
use crate::state::{GameState, PlacedPlayerCard};
use crate::turns::{
    effective_cost, first_extension_in_hand, has_professor, has_telescope, legal_builds,
    legal_extends, legal_player_placements, legal_wreckage_picks, player_card_in_hand,
};

/// Zero-sized game marker. Instances are cheap and stateless.
#[derive(Debug, Default, Clone, Copy)]
pub struct ShipWreckGame;

impl ShipWreckGame {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Short identifier used in log headers.
    pub const NAME: &'static str = "shipwreck";
}

impl Game for ShipWreckGame {
    type State = GameState;
    type Action = Action;
    type Event = Event;
    type PublicView = ShipWreckPublicView;
    type Config = ShipWreckConfig;

    fn initial_state(&self, seed: u64, cfg: &ShipWreckConfig) -> GameState {
        // Setup is deterministic given (seed, cfg). We drive it through
        // a `ProductionRng` seeded off `seed` so `initial_state` is
        // pure — `resolve_chance` later in the game loop won't need to
        // produce any more chance events (all randomness is baked into
        // the initial state per the plan's Unit 22 rules).
        let mut rng = playtest_adapters_rng_for_seed(seed);
        build_initial_state(seed, cfg, &mut rng).state
    }

    fn next_actor(&self, state: &GameState) -> Actor {
        // `game_over` short-circuits the loop before this is asked; if
        // the caller still asks, we point at the current player so no
        // panic can propagate.
        Actor::Player(state.current_player)
    }

    fn legal_actions(&self, state: &GameState, player: PlayerId) -> Vec<Action> {
        if state.phase != Phase::Play {
            return Vec::new();
        }
        if state.current_player != player {
            return Vec::new();
        }

        let me = &state.players[player as usize];
        let mut out = Vec::new();

        // 1. ExtendRaft — only if we have an extension card in hand.
        if first_extension_in_hand(&me.hand).is_some() {
            out.extend(legal_extends(&me.raft));
        }

        // 2. PlacePlayerCard.
        out.extend(legal_player_placements(
            &me.hand,
            &me.raft,
            &me.played_players,
        ));

        // 3. PickWreckage (own pool + neighbors if Telescope).
        out.extend(legal_wreckage_picks(
            player,
            &state.face_up_pools,
            has_telescope(me),
        ));

        // 4. BuildEquipment.
        let current_kind = state.current_equipment().map(|eq| eq.kind);
        out.extend(legal_builds(
            &me.inventory,
            &me.raft,
            current_kind,
            has_professor(me),
        ));

        // 5. EndTurn — always legal (lets the game progress even when
        // the current player is stuck).
        out.push(Action::EndTurn);

        out
    }

    fn apply_action(
        &self,
        state: &GameState,
        player: PlayerId,
        action: &Action,
    ) -> Result<Vec<Event>, GameError> {
        if state.phase != Phase::Play {
            return Err(GameError::IllegalAction {
                player,
                message: format!(
                    "action rejected: phase is {:?}, not Play",
                    state.phase
                ),
            });
        }
        if state.current_player != player {
            return Err(GameError::IllegalAction {
                player,
                message: format!(
                    "action rejected: current_player is {}, not {player}",
                    state.current_player
                ),
            });
        }

        match action {
            Action::ExtendRaft { insert_after } => {
                apply_extend_raft(state, player, *insert_after)
            }
            Action::PlacePlayerCard { card, slot } => {
                apply_place_player_card(state, player, *card, *slot)
            }
            Action::PickWreckage {
                from_pool,
                card_index,
            } => apply_pick_wreckage(state, player, *from_pool, *card_index),
            Action::BuildEquipment {
                equipment_kind,
                slot,
            } => apply_build_equipment(state, player, *equipment_kind, *slot),
            Action::EndTurn => Ok(apply_end_turn(state, player)),
            Action::PlayEventCard { .. } | Action::ResolveEvent(_) => {
                // Unit 23 territory — explicitly reject rather than
                // silently drop, so Unit 22 tests have a clear signal.
                Err(GameError::IllegalAction {
                    player,
                    message: "event-card actions are not available in Unit 22".into(),
                })
            }
        }
    }

    fn resolve_chance(
        &self,
        _state: &GameState,
        _rng: &mut dyn Rng,
    ) -> Result<Event, GameError> {
        // All ShipWreck randomness is baked into `initial_state`. The
        // engine should never route to `resolve_chance` during Phase::Play
        // because `next_actor` always returns `Actor::Player(...)`.
        Err(GameError::ChanceFailed {
            message: "ShipWreck has no mid-game chance events".into(),
        })
    }

    fn apply_event(&self, state: &mut GameState, event: &Event) {
        apply_event_impl(state, event);
    }

    fn public_view(&self, state: &GameState, player: PlayerId) -> ShipWreckPublicView {
        build_public_view(state, player)
    }

    fn determinize(
        &self,
        state: &GameState,
        observer: PlayerId,
        rng: &mut dyn Rng,
    ) -> GameState {
        crate::determinize::determinize(state, observer, rng)
    }

    fn game_over(&self, state: &GameState) -> Option<GameResult> {
        if state.phase == Phase::Finished {
            return Some(build_game_result(state));
        }
        if state.phase != Phase::Play {
            return None;
        }
        if !state.event_resolution_stack.is_empty() {
            return None;
        }
        if !state.wreckage_deck.is_empty() {
            return None;
        }
        if state.face_up_pools.iter().any(|p| !p.is_empty()) {
            return None;
        }
        // All face-up pools empty, deck empty, no pending events —
        // the game is over. The loop will stop here; the final
        // EndGame event + `Phase::Finished` transition is produced
        // on the next turn boundary via the `EndTurn` path. We
        // return the result directly so the GameLoop halts.
        Some(build_game_result(state))
    }
}

// ---------- apply_action helpers ----------------------------------------

fn apply_extend_raft(
    state: &GameState,
    player: PlayerId,
    insert_after: SlotId,
) -> Result<Vec<Event>, GameError> {
    let me = &state.players[player as usize];
    let (_idx, ext) = first_extension_in_hand(&me.hand).ok_or_else(|| {
        GameError::IllegalAction {
            player,
            message: "ExtendRaft requires a raft-extension card in hand".into(),
        }
    })?;
    // Validate slot existence on the raft (BaseRight rejected upfront).
    match insert_after {
        SlotId::BaseRight => {
            return Err(GameError::IllegalAction {
                player,
                message: "cannot insert an extension past BaseRight".into(),
            });
        }
        SlotId::Extension(i) if usize::from(i) >= me.raft.extensions.len() => {
            return Err(GameError::IllegalAction {
                player,
                message: format!("unknown extension slot {i}"),
            });
        }
        _ => {}
    }
    Ok(vec![Event::ExtendedRaft {
        player,
        extension_serial: ext.serial,
        insert_after,
    }])
}

fn apply_place_player_card(
    state: &GameState,
    player: PlayerId,
    card: PlayerCardId,
    slot: SlotId,
) -> Result<Vec<Event>, GameError> {
    let me = &state.players[player as usize];
    if player_card_in_hand(&me.hand, card).is_none() {
        return Err(GameError::IllegalAction {
            player,
            message: format!("player card {card:?} not in hand"),
        });
    }
    if !me.raft.has_slot(slot) {
        return Err(GameError::IllegalAction {
            player,
            message: format!("slot {slot:?} does not exist on raft"),
        });
    }
    if me.has_player_card_on_slot(slot) {
        return Err(GameError::IllegalAction {
            player,
            message: format!("slot {slot:?} already holds a player card"),
        });
    }
    Ok(vec![Event::PlacedPlayerCard {
        player,
        card,
        slot,
    }])
}

fn apply_pick_wreckage(
    state: &GameState,
    player: PlayerId,
    from_pool: PlayerId,
    card_index: u16,
) -> Result<Vec<Event>, GameError> {
    let me = &state.players[player as usize];
    let pool_idx = usize::from(from_pool);
    if pool_idx >= state.face_up_pools.len() {
        return Err(GameError::IllegalAction {
            player,
            message: format!("unknown pool index {from_pool}"),
        });
    }
    let pool = &state.face_up_pools[pool_idx];
    let ci = usize::from(card_index);
    if ci >= pool.len() {
        return Err(GameError::IllegalAction {
            player,
            message: format!(
                "card_index {ci} out of range for pool {from_pool} (len={})",
                pool.len()
            ),
        });
    }
    // Reach check — own pool always OK, neighbors OK only with
    // Telescope.
    if from_pool != player {
        let n = state.face_up_pools.len();
        let left = (usize::from(player) + n - 1) % n;
        let right = (usize::from(player) + 1) % n;
        let reachable = pool_idx == left || pool_idx == right;
        if !(reachable && has_telescope(me)) {
            return Err(GameError::IllegalAction {
                player,
                message: format!("pool {from_pool} not in reach"),
            });
        }
    }
    Ok(vec![Event::PickedWreckage {
        player,
        from_pool,
        card: pool[ci],
    }])
}

fn apply_build_equipment(
    state: &GameState,
    player: PlayerId,
    equipment_kind: crate::card::EquipmentKind,
    slot: SlotId,
) -> Result<Vec<Event>, GameError> {
    let me = &state.players[player as usize];
    let Some(top) = state.current_equipment() else {
        return Err(GameError::IllegalAction {
            player,
            message: "no equipment card available".into(),
        });
    };
    if top.kind != equipment_kind {
        return Err(GameError::IllegalAction {
            player,
            message: format!(
                "top equipment is {:?}, not {:?}",
                top.kind, equipment_kind
            ),
        });
    }
    if !me.raft.has_slot(slot) {
        return Err(GameError::IllegalAction {
            player,
            message: format!("slot {slot:?} does not exist"),
        });
    }
    if me.raft.upgrade_at(slot).is_some() {
        return Err(GameError::IllegalAction {
            player,
            message: format!("slot {slot:?} already has an upgrade"),
        });
    }
    let cost: ResourceCost = effective_cost(equipment_kind, has_professor(me));
    if !cost.can_pay(&me.inventory) {
        return Err(GameError::IllegalAction {
            player,
            message: format!(
                "insufficient resources for {equipment_kind:?} (cost {:?}, have {:?})",
                cost.amounts(),
                me.inventory
            ),
        });
    }

    let mut events: Vec<Event> = Vec::new();
    for r in Resource::ALL {
        let amt = cost.amount_of(r);
        if amt > 0 {
            events.push(Event::ResourceSpent {
                player,
                resource: r,
                amount: amt,
            });
        }
    }
    events.push(Event::BuiltEquipment {
        player,
        equipment_kind,
        slot,
    });
    Ok(events)
}

/// Apply `EndTurn`: emit per-placed-player-card food consumption,
/// then the end-of-turn event. Never fails — food_counter saturates
/// at zero.
fn apply_end_turn(state: &GameState, player: PlayerId) -> Vec<Event> {
    let me = &state.players[player as usize];

    let mut events: Vec<Event> = Vec::new();

    // Simulate the food counter drain for each placed player card.
    let mut remaining: i16 = me.food_counter;
    for pp in &me.played_players {
        let cost = i16::from(pp.card.food_cost);
        if cost == 0 {
            continue; // e.g., Wilson — no FoodConsumed emitted.
        }
        if remaining >= cost {
            remaining -= cost;
            events.push(Event::FoodConsumed {
                player,
                slot: pp.slot,
                amount: pp.card.food_cost,
                starved: false,
            });
        } else {
            // Starved — card falls off.
            events.push(Event::FoodConsumed {
                player,
                slot: pp.slot,
                amount: pp.card.food_cost,
                starved: true,
            });
        }
    }

    events.push(Event::EndTurn { player });

    // If after this EndTurn every pool is empty and the deck is empty
    // and there are no pending events, tack on an EndGame event — this
    // keeps the loop converging.
    let deck_empty = state.wreckage_deck.is_empty();
    let face_up_empty = state.face_up_pools.iter().all(Vec::is_empty);
    let no_pending = state.event_resolution_stack.is_empty();
    if deck_empty && face_up_empty && no_pending {
        let scores = compute_scores(state);
        let (winner, reason) = winner_from_scores(&scores);
        events.push(Event::EndGame {
            winner,
            reason,
            final_scores: scores,
        });
    }

    events
}

// ---------- apply_event --------------------------------------------------

#[allow(clippy::too_many_lines)]
fn apply_event_impl(state: &mut GameState, event: &Event) {
    match event {
        Event::DealPlayerCard { player, card } => {
            use crate::pool::all_player_cards;
            // Identify the full PlayerCard by id from the pool.
            let Some(pc) = all_player_cards().into_iter().find(|p| p.id == *card) else {
                // Shouldn't happen; the setup phase only deals real cards.
                return;
            };
            state.players[*player as usize]
                .hand
                .push(Card::Player(pc));
        }
        Event::DealWreckageHand { player, cards } => {
            for &c in cards {
                state.players[*player as usize].hand.push(c);
            }
        }
        Event::DealWreckageFaceUp { player, card } => {
            state.face_up_pools[*player as usize].push(*card);
        }
        Event::PickedWreckage {
            player,
            from_pool,
            card,
        } => {
            // Remove the first matching card from the named pool.
            let pool = &mut state.face_up_pools[*from_pool as usize];
            if let Some(pos) = pool.iter().position(|c| c == card) {
                pool.remove(pos);
            }
            // Place into hand / inventory.
            apply_picked_into_hand_or_inventory(
                &mut state.players[*player as usize],
                *card,
            );
        }
        Event::PlacedPlayerCard {
            player,
            card,
            slot,
        } => {
            let me = &mut state.players[*player as usize];
            if let Some(pos) = me.hand.iter().position(
                |c| matches!(c, Card::Player(pc) if pc.id == *card),
            ) {
                let removed = me.hand.remove(pos);
                if let Card::Player(pc) = removed {
                    me.played_players.push(PlacedPlayerCard {
                        card: pc,
                        slot: *slot,
                    });
                }
            }
        }
        Event::ExtendedRaft {
            player,
            extension_serial,
            insert_after,
        } => {
            let me = &mut state.players[*player as usize];
            // Find extension card in hand with the matching serial.
            if let Some(pos) = me.hand.iter().position(|c| {
                matches!(c, Card::RaftExtension(ext) if ext.serial == *extension_serial)
            }) {
                let removed = me.hand.remove(pos);
                if let Card::RaftExtension(ext) = removed {
                    // Raft::extend validates and re-indexes upgrades.
                    let _ = me.raft.extend(ext, *insert_after);
                }
            }
        }
        Event::BuiltEquipment {
            player,
            equipment_kind,
            slot,
        } => {
            let me = &mut state.players[*player as usize];
            // Pop the top of the equipment deck. The action validated
            // that top.kind matches equipment_kind.
            if let Some(top) = state.equipment_deck.pop() {
                if top.kind == *equipment_kind {
                    let _ = me.raft.build_upgrade(*slot, top);
                } else {
                    // Mismatch — push back (shouldn't happen if action
                    // validation is sound).
                    state.equipment_deck.push(top);
                }
            }
        }
        Event::ResourceSpent {
            player,
            resource,
            amount,
        } => {
            let inv = &mut state.players[*player as usize].inventory;
            let idx = resource.index();
            inv[idx] = inv[idx].saturating_sub(*amount);
        }
        Event::EventCardPlayed { .. } | Event::EventResolved { .. } => {
            unimplemented!("Unit 23: event-card resolution");
        }
        Event::FoodConsumed {
            player,
            slot,
            amount,
            starved,
        } => {
            let me = &mut state.players[*player as usize];
            if *starved {
                // Drop the placed player card at `slot` (if present).
                // Food counter stays put (would have gone negative).
                if let Some(pos) =
                    me.played_players.iter().position(|pp| pp.slot == *slot)
                {
                    me.played_players.remove(pos);
                }
            } else {
                me.food_counter = me.food_counter.saturating_sub(i16::from(*amount));
            }
        }
        Event::EndTurn { player: _ } => {
            // Reset the per-turn food-charge tracker (implicitly: each
            // end_turn starts fresh; since we compute next_food_cost_to_charge
            // by iterating played_players each time and the event-stream
            // re-applies the same events, the accumulation is already
            // captured in played_players mutations).
            //
            // Advance to next seat.
            let n = state.players.len();
            state.current_player =
                (state.current_player + 1) % u8::try_from(n).expect("n fits in u8");
        }
        Event::EndGame {
            winner: _,
            reason: _,
            final_scores: _,
        } => {
            state.phase = Phase::Finished;
        }
    }
}

/// Move a picked wreckage card into the player's hand/inventory in
/// the shape the rules want: items land in inventory counters;
/// players/extensions/equipment/events land in hand.
fn apply_picked_into_hand_or_inventory(
    player_state: &mut crate::state::PlayerState,
    card: Card,
) {
    match card {
        Card::Item(item) => {
            let r = item.resource();
            let slot = &mut player_state.inventory[r.index()];
            *slot = slot.saturating_add(1);
        }
        // Every other card type goes into the hand. Equipment picked
        // this way (an unexpected edge — equipment cards don't
        // normally live in face-up pools) also goes to hand.
        _ => player_state.hand.push(card),
    }
}

// ---------- scoring ------------------------------------------------------

fn compute_scores(state: &GameState) -> Vec<PlayerScore> {
    let n = state.players.len();
    let mut scores = Vec::with_capacity(n);
    for (i, p) in state.players.iter().enumerate() {
        let rescue_points: u32 = p
            .played_players
            .iter()
            .map(|pp| u32::from(pp.card.rescue_points))
            .sum();
        let raft_length = u16::try_from(p.raft.length())
            .expect("raft length fits in u16");
        let invention_count = u16::try_from(p.raft.invention_count())
            .expect("invention count fits in u16");
        scores.push(PlayerScore {
            player: u8::try_from(i).expect("seat < 4 fits in u8"),
            rescue_points: u16::try_from(rescue_points).unwrap_or(u16::MAX),
            raft_length,
            invention_count,
        });
    }
    scores
}

/// Pick a winner from `scores` with spec tie-breakers: rescue points
/// desc, then raft_length desc, then invention_count desc, then draw.
fn winner_from_scores(scores: &[PlayerScore]) -> (Option<PlayerId>, EndReason) {
    if scores.is_empty() {
        return (None, EndReason::Draw);
    }
    // Sort by descending (rescue_points, raft_length, invention_count).
    let mut ranked: Vec<&PlayerScore> = scores.iter().collect();
    ranked.sort_by(|a, b| {
        b.rescue_points
            .cmp(&a.rescue_points)
            .then(b.raft_length.cmp(&a.raft_length))
            .then(b.invention_count.cmp(&a.invention_count))
    });
    let top = ranked[0];
    if ranked.len() >= 2 {
        let second = ranked[1];
        if top.rescue_points == second.rescue_points
            && top.raft_length == second.raft_length
            && top.invention_count == second.invention_count
        {
            return (None, EndReason::Draw);
        }
    }
    (Some(top.player), EndReason::Other("deck_exhausted".into()))
}

/// Build the GameResult from state scores — used by `game_over`.
fn build_game_result(state: &GameState) -> GameResult {
    let scores = compute_scores(state);
    let (winner, reason) = winner_from_scores(&scores);
    let score_vec: Vec<i32> = scores
        .iter()
        .map(|s| i32::from(s.rescue_points))
        .collect();
    GameResult {
        winner,
        reason,
        scores: score_vec,
    }
}

// ---------- misc helpers -------------------------------------------------

/// A lightweight ChaCha20Rng without pulling the adapter crate into
/// `playtest-shipwreck`'s runtime deps. We reuse the rand_chacha dep
/// that `playtest-adapters` exposes via its production RNG — pulling
/// it here directly is cleaner than re-exposing the adapter just to
/// seed initial_state.
fn playtest_adapters_rng_for_seed(seed: u64) -> SeededRng {
    SeededRng::from_seed(seed)
}

/// Minimal seeded `Rng` built on `rand_chacha::ChaCha20Rng`. Lives
/// here (rather than in `playtest-adapters`) to keep Unit 22's trait
/// impl self-contained; the production adapter in `playtest-adapters`
/// is identical in shape and both end up using the same algorithm.
use rand_chacha::rand_core::{RngCore, SeedableRng};

struct SeededRng {
    inner: rand_chacha::ChaCha20Rng,
}

impl SeededRng {
    fn from_seed(seed: u64) -> Self {
        Self {
            inner: rand_chacha::ChaCha20Rng::seed_from_u64(seed),
        }
    }
}

impl Rng for SeededRng {
    fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }

    fn gen_range(
        &mut self,
        range: core::ops::Range<u64>,
    ) -> Result<u64, playtest_ports::RngError> {
        if range.start >= range.end {
            return Err(playtest_ports::RngError::InvalidRange {
                start: range.start,
                end: range.end,
            });
        }
        let span = range.end - range.start;
        Ok(range.start + self.inner.next_u64() % span)
    }
}

