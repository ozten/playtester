//! Evaluation function for Great Gyre one-ply agents (`GreedyAgent` /
//! `HeuristicAgent`), per `playtest_agents::eval::EvalFn`:
//! `fn(view: &GreatGyrePublicView, player: PlayerId) -> f64`.
//!
//! Higher = better for `player`. Terms, in the order the plan
//! specifies:
//!
//! 1. **Raft hope (dominant)** — the real end-of-game score formula
//!    (`crate::scoring::score_player`) applied to the *current*
//!    public state: hope icons on `player`'s placed survivors/
//!    modifications, `+15` if `player` holds Land Sighting, `-5` per
//!    *other* visible seat's placed Influencer. Every input is
//!    public (own hand + every raft's `placed` list is visible to
//!    everyone), so this term tracks the real scoring function
//!    exactly — not an approximation.
//! 2. **Buildable-hope potential** — a modest fraction of the hope
//!    value of the *single best* modification `player`'s hand can
//!    currently afford (a proxy for "hope one action away"). This
//!    deliberately takes a `max`, not a `sum`, over affordable kinds:
//!    summing would double- (triple-, ...) count hope across
//!    modifications that share the same underlying resource cards, and
//!    since building any one of them consumes those shared resources,
//!    a sum-based potential collapses by *more* than the hope actually
//!    realized — perversely rewarding hoarding resources over ever
//!    spending them (a real bug caught by the `heuristic-greatgyre
//!    beats random` sanity benchmark: an earlier sum-based version
//!    scored `FinishActions` over `BuildModification`/`BuildExtension`
//!    essentially every turn). The fraction (`W_BUILDABLE_HOPE_FRACTION
//!    < 1.0`) guarantees actually building the best-affordable item is
//!    still strictly better than merely holding the resources for it.
//! 3. **Realized extension investment + engine bonuses** — a small
//!    flat reward per *already-built* raft extension, and per built
//!    Net/Toolkit (draw/action-budget engines), all evaluated on
//!    realized state, not affordability (same reason as (2): these
//!    give no hope of their own — Net is 0 hope, extensions are none —
//!    so a "could afford it" potential term would only ever discourage
//!    spending the resources, and a one-ply lookahead can't otherwise
//!    see that a bigger permanent draw/action budget pays for itself
//!    over the *rest* of the game). Both terms feed the sanity
//!    benchmark: without them, `GreedyAgent` correctly preferred
//!    *quality* (highest-hope builds) but never built the engine
//!    pieces, and `RandomAgent` — which almost always exhausts its
//!    full per-turn draw/action budget just by picking a random legal
//!    action each time, "finish early" being only one option among
//!    many — ended up placing roughly twice as many cards per game by
//!    volume alone, more than offsetting the heuristic's per-card
//!    quality edge.
//! 4. **Food-deficit penalty (and surplus reward)** — the deficit
//!    penalty is scaled by how many standing survivors are at risk: a
//!    deficit fully covered by turning standing survivors Hungry is a
//!    mild, temporary penalty (Hungry survivors' stats still count); a
//!    deficit *exceeding* the standing-survivor count forces
//!    abandoning already-Hungry survivors back to the Current — a
//!    permanent hope loss — so that portion is weighted much more
//!    heavily. A small positive term also rewards *surplus* food,
//!    pulling the same lever as the engine bonuses above toward
//!    building Garden/Rain Trap/Fishing Rod.
//! 5. **Small tempo terms** — free raft spaces and hand size. (An
//!    earlier version also rewarded *unspent* `draws_remaining`/
//!    `actions_remaining` as a "tempo" signal; that's the same
//!    potential-vs-realized bug as (2)/(3) in disguise — for a one-ply
//!    lookahead comparing candidate actions *within the same
//!    decision*, rewarding leftover budget always penalizes the very
//!    action that spends it, so `FinishDrawing`/`FinishActions` looked
//!    artificially attractive over actually drawing or building. Also
//!    caught by the sanity benchmark; removed.)

use playtest_core::PlayerId;

use crate::card::{CardKind, EventKind, ModificationKind, SurvivorId};
use crate::public_view::GreatGyrePublicView;
use crate::turns::can_afford;

const W_BUILDABLE_HOPE_FRACTION: f64 = 0.15;
const W_BUILT_EXTENSION: f64 = 1.5;
/// Per built Net (draw-budget engine) or Athlete (same effect,
/// survivor form).
const W_DRAW_ENGINE: f64 = 2.5;
/// Per built Toolkit (action-budget engine) or First Mate (same
/// effect, survivor form).
const W_ACTION_ENGINE: f64 = 2.5;
const W_FOOD_DEFICIT_COVERABLE: f64 = -2.0;
const W_FOOD_DEFICIT_ABANDON: f64 = -12.0;
const W_FOOD_SURPLUS: f64 = 0.4;
const W_FREE_SPACES: f64 = 0.5;
const W_HAND_SIZE: f64 = 0.3;

/// Score the Great Gyre public view from `player`'s perspective.
///
/// `player` must be the view's own observer (`view.observer ==
/// player`) — `GreedyAgent`/`HeuristicAgent` always call it that way.
#[must_use]
pub fn greatgyre_eval(view: &GreatGyrePublicView, player: PlayerId) -> f64 {
    debug_assert_eq!(
        view.observer, player,
        "greatgyre_eval expects the view's own observer"
    );

    // ---------- 1. Raft hope (dominant) ---------------------------------
    let own_hope: u32 = view.own.seat.placed.iter().map(|p| p.card.kind.hope()).sum();
    let mut score = f64::from(own_hope);

    let holds_land_sighting = view
        .own
        .hand
        .iter()
        .any(|c| matches!(c.kind, CardKind::Event(EventKind::LandSighting)));
    if holds_land_sighting {
        score += 15.0;
    }

    let influencer_count = view
        .opponents
        .iter()
        .flatten()
        .filter(|opp| {
            opp.seat
                .placed
                .iter()
                .any(|p| matches!(p.card.kind, CardKind::Survivor(SurvivorId::Influencer)))
        })
        .count();
    score -= 5.0 * f64::from(u32::try_from(influencer_count).unwrap_or(u32::MAX));

    // ---------- 2 & 3. Buildable-hope potential + built extensions -----
    let hand = &view.own.hand;
    let best_affordable_hope = ModificationKind::ALL
        .into_iter()
        .filter(|m| can_afford(hand, m.cost()))
        .map(ModificationKind::hope)
        .max()
        .unwrap_or(0);
    score += f64::from(best_affordable_hope) * W_BUILDABLE_HOPE_FRACTION;
    score += W_BUILT_EXTENSION * f64::from(u32::try_from(view.own.seat.built_extensions.len()).unwrap_or(u32::MAX));

    let placed_count_of = |pred: fn(&crate::state::PlacedCard) -> bool| -> u32 {
        u32::try_from(view.own.seat.placed.iter().filter(|p| pred(p)).count()).unwrap_or(u32::MAX)
    };
    let draw_engines = placed_count_of(|p| {
        matches!(p.card.kind, CardKind::Modification(ModificationKind::Net))
            || matches!(p.card.kind, CardKind::Survivor(SurvivorId::Athlete))
    });
    let action_engines = placed_count_of(|p| {
        matches!(p.card.kind, CardKind::Modification(ModificationKind::Toolkit))
            || matches!(p.card.kind, CardKind::Survivor(SurvivorId::FirstMate))
    });
    score += W_DRAW_ENGINE * f64::from(draw_engines);
    score += W_ACTION_ENGINE * f64::from(action_engines);

    // ---------- 4. Food-deficit penalty / surplus reward ----------------
    let food = own_food(view);
    if food < 0 {
        let deficit = u32::try_from(-food).unwrap_or(u32::MAX);
        let standing = view
            .own
            .seat
            .placed
            .iter()
            .filter(|p| matches!(p.card.kind, CardKind::Survivor(_)) && !p.hungry)
            .count();
        let standing = u32::try_from(standing).unwrap_or(0);
        let coverable = deficit.min(standing);
        let abandon = deficit.saturating_sub(standing);
        score += W_FOOD_DEFICIT_COVERABLE * f64::from(coverable);
        score += W_FOOD_DEFICIT_ABANDON * f64::from(abandon);
    } else {
        score += W_FOOD_SURPLUS * f64::from(u32::try_from(food).unwrap_or(0));
    }

    // ---------- 5. Small tempo terms -------------------------------------
    score += W_FREE_SPACES * f64::from(free_spaces(view));
    score += W_HAND_SIZE * f64::from(u32::try_from(hand.len()).unwrap_or(u32::MAX));

    score
}

/// `Σ` food icons on `player`'s own face-up raft, mirroring
/// `crate::turns::compute_food` but reading from the (already public)
/// view instead of `PlayerState` — every input here is visible to the
/// owner, so this always matches the real value exactly.
fn own_food(view: &GreatGyrePublicView) -> i32 {
    let mut food = 1; // Raft Right.
    for p in &view.own.seat.placed {
        food += match p.card.kind {
            CardKind::Survivor(s) => i32::from(s.food()),
            CardKind::Modification(m) => i32::from(m.food()),
            _ => 0,
        };
    }
    food
}

/// Free raft spaces remaining, mirroring `crate::turns::free_spaces`.
fn free_spaces(view: &GreatGyrePublicView) -> u32 {
    let mut used = u32::try_from(view.own.seat.blocked_by_walrus.len()).unwrap_or(u32::MAX);
    let mut total =
        2 + 2 * u32::try_from(view.own.seat.built_extensions.len()).unwrap_or(u32::MAX);
    for p in &view.own.seat.placed {
        let occupies = match p.card.kind {
            CardKind::Survivor(s) => s.occupies_space(),
            CardKind::Modification(m) => m.occupies_space(),
            _ => false,
        };
        if occupies {
            used += 1;
        }
        if let CardKind::Modification(ModificationKind::Quarterdeck) = p.card.kind {
            total += u32::from(ModificationKind::Quarterdeck.provides_capacity());
        }
    }
    total.saturating_sub(used)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Card, CardInstanceId};
    use crate::public_view::{OpponentView, OwnView, PassDirection, SeatPublic};
    use crate::state::{PlacedCard, Phase};

    fn empty_seat(player: PlayerId) -> SeatPublic {
        let raft = Card::new(CardInstanceId(9000), CardKind::RaftLeft);
        SeatPublic {
            player,
            current: Vec::new(),
            raft_left: raft,
            raft_right: raft,
            built_extensions: Vec::new(),
            placed: Vec::new(),
            draws_remaining: 0,
            actions_remaining: 0,
            porter_used: false,
            swimmer_used: false,
            pirate_used: false,
            blocked_by_walrus: Vec::new(),
            work_day_active: false,
        }
    }

    fn view_with(own: OwnView) -> GreatGyrePublicView {
        GreatGyrePublicView {
            observer: own.seat.player,
            own,
            opponents: vec![None, None],
            undrafted_survivors: Vec::new(),
            deep_sea_deck_remaining: 0,
            final_round_deck_remaining: 0,
            event_deck_remaining: 0,
            discard_pile: Vec::new(),
            extension_pile_remaining: 0,
            phase: Phase::Actions,
            current_player: 0,
            first_player: 0,
            pass_direction: PassDirection::Left,
            final_round: false,
            pending_decisions: Vec::new(),
            pending_chance: None,
        }
    }

    fn placed(id: u32, kind: CardKind, hungry: bool) -> PlacedCard {
        PlacedCard {
            card: Card::new(CardInstanceId(id), kind),
            hungry,
        }
    }

    #[test]
    fn more_placed_hope_scores_higher() {
        let mut with_sail = OwnView {
            seat: empty_seat(0),
            hand: Vec::new(),
        };
        with_sail
            .seat
            .placed
            .push(placed(1, CardKind::Modification(ModificationKind::Sail), false));
        let without = OwnView {
            seat: empty_seat(0),
            hand: Vec::new(),
        };
        assert!(
            greatgyre_eval(&view_with(with_sail), 0) > greatgyre_eval(&view_with(without), 0)
        );
    }

    #[test]
    fn opponent_influencer_lowers_score() {
        let own = OwnView {
            seat: empty_seat(0),
            hand: Vec::new(),
        };
        let plain = view_with(own.clone());

        let mut with_influencer = view_with(own);
        let mut opp_seat = empty_seat(1);
        opp_seat
            .placed
            .push(placed(1, CardKind::Survivor(SurvivorId::Influencer), false));
        with_influencer.opponents[1] = Some(OpponentView {
            seat: opp_seat,
            hand_size: 0,
        });

        assert!(greatgyre_eval(&plain, 0) > greatgyre_eval(&with_influencer, 0));
    }

    #[test]
    fn affordable_modification_raises_buildable_potential() {
        let mut with_resources = OwnView {
            seat: empty_seat(0),
            hand: vec![
                Card::new(CardInstanceId(1), CardKind::Resource(crate::resource::Resource::Wood)),
                Card::new(CardInstanceId(2), CardKind::Resource(crate::resource::Resource::Wood)),
            ],
        };
        with_resources.seat.player = 0;
        let empty = OwnView {
            seat: empty_seat(0),
            hand: Vec::new(),
        };
        // 2 Wood affords Quarterdeck.
        assert!(
            greatgyre_eval(&view_with(with_resources), 0) > greatgyre_eval(&view_with(empty), 0)
        );
    }

    #[test]
    fn abandon_forcing_deficit_penalized_more_than_coverable_deficit() {
        // Same three survivors (same hope, same printed food — Hungry
        // survivors' stats still count per the spec's `[A]` ruling), so
        // the only difference between the two views is how many
        // *standing* survivors are available to absorb the food
        // deficit. All-standing can fully cover it (mild, temporary
        // penalty); all-Hungry cannot (forces abandoning a Hungry
        // survivor — a permanent hope loss, weighted much more
        // heavily).
        let mut standing = OwnView {
            seat: empty_seat(0),
            hand: Vec::new(),
        };
        let mut all_hungry = OwnView {
            seat: empty_seat(0),
            hand: Vec::new(),
        };
        for i in 0..3u32 {
            standing
                .seat
                .placed
                .push(placed(i, CardKind::Survivor(SurvivorId::FirstMate), false));
            all_hungry
                .seat
                .placed
                .push(placed(i, CardKind::Survivor(SurvivorId::FirstMate), true));
        }
        assert!(greatgyre_eval(&view_with(standing), 0) > greatgyre_eval(&view_with(all_hungry), 0));
    }
}
