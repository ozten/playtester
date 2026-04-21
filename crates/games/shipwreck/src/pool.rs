//! Static card-pool constructors.
//!
//! All quantities are sourced from `docs/shipwreck.md`:
//!
//! | Category            | Count                                    |
//! |---------------------|------------------------------------------|
//! | Player cards        | 7 named (Professor, MaryAnne, …)         |
//! | Equipment cards     | 13 (2 FishingPoles, 1 Telescope, 3 AutoNets, 2 SteelCordage, 5 RainCatcher) |
//! | Raft extensions     | 40 ("say 40 in the deck")                |
//! | Wreckage items      | 150 (30 × Plastic / Wood / Rope / Cloth / Wire) |
//! | Event cards         | 18 (6 Shark, 2 Typhoon, 10 FlyingFish — *pending playtesting*) |
//!
//! Event-card counts are a design choice; the spec does not give
//! exact numbers. The defaults are exposed as public consts so later
//! units can tune them without editing this module.

use crate::card::{
    BaseRaftCard, BaseRaftSide, Card, EquipmentCard, EquipmentKind, EventCard, ItemCard,
    PlayerCard, PlayerCardId, PlayerSkill, RaftExtensionCard,
};
use crate::resource::Resource;

/// Number of each resource in the wreckage-item pool (30 of each, per spec).
pub const ITEM_COUNT_PER_RESOURCE: usize = 30;

/// Number of raft extension cards in the deck ("say 40 in the deck").
pub const RAFT_EXTENSION_COUNT: usize = 40;

/// Default shark count — pending playtesting.
pub const DEFAULT_SHARK_COUNT: usize = 6;

/// Default typhoon count — pending playtesting.
pub const DEFAULT_TYPHOON_COUNT: usize = 2;

/// Default flying-fish count — pending playtesting.
pub const DEFAULT_FLYING_FISH_COUNT: usize = 10;

/// The seven named player cards, per `docs/shipwreck.md`. Returned in a
/// stable order so tests can index by position without shuffling.
///
/// Rescue-point and food-cost values match the spec exactly:
/// - Professor: 1 rp, 1 food, ConstructWithOneFewerResource
/// - MaryAnne: 1 rp, 1 food, ExtraReachOne
/// - MovieStar: 2 rp, 1 food, no skill
/// - Gillagun: 1 rp, 1 food, FreeSkipFoodOncePerGame
/// - Millionaire: 2 rp, 2 food, no skill
/// - MillionHeiress: 2 rp, 2 food, no skill
/// - Wilson: 1 rp, 0 food, no skill
#[must_use]
pub fn all_player_cards() -> Vec<PlayerCard> {
    vec![
        PlayerCard::new(
            PlayerCardId::Professor,
            1,
            1,
            Some(PlayerSkill::ConstructWithOneFewerResource),
        ),
        PlayerCard::new(
            PlayerCardId::MaryAnne,
            1,
            1,
            Some(PlayerSkill::ExtraReachOne),
        ),
        PlayerCard::new(PlayerCardId::MovieStar, 2, 1, None),
        PlayerCard::new(
            PlayerCardId::Gillagun,
            1,
            1,
            Some(PlayerSkill::FreeSkipFoodOncePerGame),
        ),
        PlayerCard::new(PlayerCardId::Millionaire, 2, 2, None),
        PlayerCard::new(PlayerCardId::MillionHeiress, 2, 2, None),
        PlayerCard::new(PlayerCardId::Wilson, 1, 0, None),
    ]
}

/// The thirteen equipment cards in the deck. Quantities per spec:
/// 2× FishingPoles, 1× Telescope, 3× AutoNets, 2× SteelCordage,
/// 5× RainCatcher (sums to 13 — the Unit 20 plan text says "14 total"
/// but its per-kind breakdown sums to 13; the per-kind numbers match
/// `docs/shipwreck.md` verbatim, so 13 is authoritative). Serials are
/// assigned 0..n within each kind.
#[must_use]
pub fn all_equipment() -> Vec<EquipmentCard> {
    const QUANTITIES: &[(EquipmentKind, u16)] = &[
        (EquipmentKind::FishingPoles, 2),
        (EquipmentKind::Telescope, 1),
        (EquipmentKind::AutoNets, 3),
        (EquipmentKind::SteelCordage, 2),
        (EquipmentKind::RainCatcher, 5),
    ];
    let mut out = Vec::with_capacity(13);
    for &(kind, n) in QUANTITIES {
        for serial in 0..n {
            out.push(EquipmentCard::new(kind, serial));
        }
    }
    out
}

/// The two base-raft cards that every player starts with. Returned as
/// `(left, right)` so callers don't have to remember which side goes first.
#[must_use]
pub fn base_raft_pair() -> (BaseRaftCard, BaseRaftCard) {
    (
        BaseRaftCard::new(BaseRaftSide::Left),
        BaseRaftCard::new(BaseRaftSide::Right),
    )
}

/// Every wreckage card that goes into the shuffled wreckage deck during
/// setup: raft extensions + items + equipment + events. Per spec the
/// remaining player cards are *also* shuffled into this pile, but that
/// depends on the dealt-player-card count, which is a setup-phase
/// concern and lives in the later unit.
///
/// Ordering: extensions (0..40), then items grouped by resource in
/// [`Resource::ALL`] order (30 each), then equipment (in
/// [`all_equipment`] order), then events (shark ×6, typhoon ×2,
/// flying-fish ×10). Callers that want randomness should shuffle via
/// the Rng port.
#[must_use]
pub fn all_wreckage_cards() -> Vec<Card> {
    let mut out = Vec::with_capacity(
        RAFT_EXTENSION_COUNT
            + ITEM_COUNT_PER_RESOURCE * Resource::ALL.len()
            + all_equipment().len()
            + DEFAULT_SHARK_COUNT
            + DEFAULT_TYPHOON_COUNT
            + DEFAULT_FLYING_FISH_COUNT,
    );

    // Raft extensions
    for serial in 0..u16::try_from(RAFT_EXTENSION_COUNT).expect("extension count fits in u16") {
        out.push(Card::RaftExtension(RaftExtensionCard::new(serial)));
    }

    // Items — 30 of each resource
    for r in Resource::ALL {
        for _ in 0..ITEM_COUNT_PER_RESOURCE {
            out.push(Card::Item(ItemCard::new(r)));
        }
    }

    // Equipment
    for eq in all_equipment() {
        out.push(Card::Equipment(eq));
    }

    // Events
    for _ in 0..DEFAULT_SHARK_COUNT {
        out.push(Card::Event(EventCard::Shark));
    }
    for _ in 0..DEFAULT_TYPHOON_COUNT {
        out.push(Card::Event(EventCard::Typhoon));
    }
    for _ in 0..DEFAULT_FLYING_FISH_COUNT {
        out.push(Card::Event(EventCard::FlyingFish));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_card_pool_has_seven_cards() {
        assert_eq!(all_player_cards().len(), 7);
    }

    #[test]
    fn equipment_pool_has_thirteen_cards() {
        // Per-kind breakdown from docs/shipwreck.md: 2 + 1 + 3 + 2 + 5 = 13.
        assert_eq!(all_equipment().len(), 13);
    }
}
