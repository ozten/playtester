//! Integration tests for ShipWreck primitives (Unit 20).
//!
//! Covers every scenario listed in the Unit 20 plan. Organized into
//! modules by aspect so failures localize cleanly.

use std::collections::HashSet;

use playtest_shipwreck::{
    BaseRaftCard, Card, EquipmentCard, EquipmentKind, EventCard, ItemCard, PlayerCard,
    PlayerCardId, PlayerSkill, Raft, RaftError, RaftExtensionCard, Resource, ResourceCost, SlotId,
    all_equipment, all_player_cards, all_wreckage_cards,
};

// ---------- Player card pool --------------------------------------------

mod player_cards {
    use super::*;

    #[test]
    fn pool_has_exactly_seven_named_cards() {
        let pool = all_player_cards();
        assert_eq!(pool.len(), 7);
        let ids: HashSet<_> = pool.iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), 7, "duplicate player card id");
        for expected in [
            PlayerCardId::Professor,
            PlayerCardId::MaryAnne,
            PlayerCardId::MovieStar,
            PlayerCardId::Gillagun,
            PlayerCardId::Millionaire,
            PlayerCardId::MillionHeiress,
            PlayerCardId::Wilson,
        ] {
            assert!(ids.contains(&expected), "missing {expected:?}");
        }
    }

    fn find(id: PlayerCardId) -> PlayerCard {
        all_player_cards()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("missing {id:?}"))
    }

    #[test]
    fn food_costs_match_spec() {
        assert_eq!(find(PlayerCardId::Professor).food_cost, 1);
        assert_eq!(find(PlayerCardId::MaryAnne).food_cost, 1);
        assert_eq!(find(PlayerCardId::MovieStar).food_cost, 1);
        assert_eq!(find(PlayerCardId::Gillagun).food_cost, 1);
        assert_eq!(find(PlayerCardId::Millionaire).food_cost, 2);
        assert_eq!(find(PlayerCardId::MillionHeiress).food_cost, 2);
        assert_eq!(find(PlayerCardId::Wilson).food_cost, 0);
    }

    #[test]
    fn rescue_points_match_spec() {
        // 2-pointers: MovieStar + both Millionaires.
        assert_eq!(find(PlayerCardId::MovieStar).rescue_points, 2);
        assert_eq!(find(PlayerCardId::Millionaire).rescue_points, 2);
        assert_eq!(find(PlayerCardId::MillionHeiress).rescue_points, 2);
        // 1-pointers: the remaining four.
        for id in [
            PlayerCardId::Professor,
            PlayerCardId::MaryAnne,
            PlayerCardId::Gillagun,
            PlayerCardId::Wilson,
        ] {
            assert_eq!(find(id).rescue_points, 1, "{id:?}");
        }
    }

    #[test]
    fn skills_match_spec() {
        assert_eq!(
            find(PlayerCardId::Professor).skill,
            Some(PlayerSkill::ConstructWithOneFewerResource)
        );
        assert_eq!(
            find(PlayerCardId::MaryAnne).skill,
            Some(PlayerSkill::ExtraReachOne)
        );
        assert_eq!(
            find(PlayerCardId::Gillagun).skill,
            Some(PlayerSkill::FreeSkipFoodOncePerGame)
        );
        for id in [
            PlayerCardId::MovieStar,
            PlayerCardId::Millionaire,
            PlayerCardId::MillionHeiress,
            PlayerCardId::Wilson,
        ] {
            assert_eq!(find(id).skill, None, "{id:?}");
        }
    }
}

// ---------- Equipment pool ----------------------------------------------

mod equipment {
    use super::*;

    #[test]
    fn pool_has_thirteen_cards_with_correct_quantities() {
        // Plan text says "14 total" but its per-kind numbers (2/1/3/2/5)
        // sum to 13 and match docs/shipwreck.md verbatim.
        let pool = all_equipment();
        assert_eq!(pool.len(), 13);
        let count = |kind: EquipmentKind| pool.iter().filter(|e| e.kind == kind).count();
        assert_eq!(count(EquipmentKind::FishingPoles), 2);
        assert_eq!(count(EquipmentKind::Telescope), 1);
        assert_eq!(count(EquipmentKind::AutoNets), 3);
        assert_eq!(count(EquipmentKind::SteelCordage), 2);
        assert_eq!(count(EquipmentKind::RainCatcher), 5);
    }

    #[test]
    fn telescope_costs_2_wood_1_plastic_1_wire() {
        let cost = EquipmentKind::Telescope.cost();
        assert_eq!(cost.amount_of(Resource::Wood), 2);
        assert_eq!(cost.amount_of(Resource::Plastic), 1);
        assert_eq!(cost.amount_of(Resource::Wire), 1);
        assert_eq!(cost.amount_of(Resource::Rope), 0);
        assert_eq!(cost.amount_of(Resource::Cloth), 0);
    }

    #[test]
    fn fishing_poles_costs_1_wood_1_rope_1_wire() {
        let cost = EquipmentKind::FishingPoles.cost();
        assert_eq!(cost.amount_of(Resource::Wood), 1);
        assert_eq!(cost.amount_of(Resource::Rope), 1);
        assert_eq!(cost.amount_of(Resource::Wire), 1);
        assert_eq!(cost.amount_of(Resource::Plastic), 0);
        assert_eq!(cost.amount_of(Resource::Cloth), 0);
    }

    #[test]
    fn auto_nets_costs_1_cloth_1_rope_1_wood() {
        let cost = EquipmentKind::AutoNets.cost();
        assert_eq!(cost.amount_of(Resource::Cloth), 1);
        assert_eq!(cost.amount_of(Resource::Rope), 1);
        assert_eq!(cost.amount_of(Resource::Wood), 1);
    }

    #[test]
    fn steel_cordage_costs_2_wire_1_plastic() {
        let cost = EquipmentKind::SteelCordage.cost();
        assert_eq!(cost.amount_of(Resource::Wire), 2);
        assert_eq!(cost.amount_of(Resource::Plastic), 1);
    }

    #[test]
    fn rain_catcher_costs_2_cloth_1_wood() {
        let cost = EquipmentKind::RainCatcher.cost();
        assert_eq!(cost.amount_of(Resource::Cloth), 2);
        assert_eq!(cost.amount_of(Resource::Wood), 1);
    }
}

// ---------- Wreckage items ----------------------------------------------

mod wreckage_items {
    use super::*;

    #[test]
    fn item_pool_within_wreckage_has_150_cards_30_per_resource() {
        let wreckage = all_wreckage_cards();
        let items: Vec<ItemCard> = wreckage
            .iter()
            .filter_map(|c| if let Card::Item(it) = c { Some(*it) } else { None })
            .collect();
        assert_eq!(items.len(), 150);
        for r in Resource::ALL {
            let n = items.iter().filter(|it| it.resource() == r).count();
            assert_eq!(n, 30, "{r:?} count");
        }
    }
}

// ---------- Raft --------------------------------------------------------

mod raft_tests {
    use super::*;
    use playtest_shipwreck::BaseRaftCard;
    use playtest_shipwreck::card::BaseRaftSide;

    fn fresh_raft() -> Raft {
        Raft::new(
            BaseRaftCard::new(BaseRaftSide::Left),
            BaseRaftCard::new(BaseRaftSide::Right),
        )
    }

    #[test]
    fn starts_with_length_two() {
        let raft = fresh_raft();
        assert_eq!(raft.length(), 2);
        assert_eq!(raft.invention_count(), 0);
    }

    #[test]
    fn extend_after_base_left_grows_to_length_three() {
        let mut raft = fresh_raft();
        raft.extend(RaftExtensionCard::new(7), SlotId::BaseLeft)
            .unwrap();
        assert_eq!(raft.length(), 3);
        assert_eq!(raft.extensions.len(), 1);
        assert_eq!(raft.extensions[0].serial, 7);
    }

    #[test]
    fn extend_after_base_right_returns_error() {
        let mut raft = fresh_raft();
        let err = raft
            .extend(RaftExtensionCard::new(1), SlotId::BaseRight)
            .unwrap_err();
        assert_eq!(err, RaftError::CannotInsertAfterBaseRight);
        assert_eq!(raft.length(), 2, "raft unchanged on error");
    }

    #[test]
    fn extend_after_unknown_extension_returns_unknown_slot() {
        let mut raft = fresh_raft();
        let err = raft
            .extend(RaftExtensionCard::new(1), SlotId::Extension(999))
            .unwrap_err();
        assert_eq!(err, RaftError::UnknownSlot(SlotId::Extension(999)));
    }

    #[test]
    fn build_upgrade_on_empty_slot_succeeds() {
        let mut raft = fresh_raft();
        let eq = EquipmentCard::new(EquipmentKind::Telescope, 0);
        raft.build_upgrade(SlotId::BaseLeft, eq).unwrap();
        assert_eq!(raft.invention_count(), 1);
        assert_eq!(raft.upgrade_at(SlotId::BaseLeft), Some(&eq));
    }

    #[test]
    fn build_upgrade_on_occupied_slot_returns_slot_occupied() {
        let mut raft = fresh_raft();
        let eq1 = EquipmentCard::new(EquipmentKind::Telescope, 0);
        let eq2 = EquipmentCard::new(EquipmentKind::FishingPoles, 0);
        raft.build_upgrade(SlotId::BaseLeft, eq1).unwrap();
        let err = raft.build_upgrade(SlotId::BaseLeft, eq2).unwrap_err();
        assert_eq!(err, RaftError::SlotOccupied(SlotId::BaseLeft));
        // Original upgrade still in place.
        assert_eq!(raft.upgrade_at(SlotId::BaseLeft), Some(&eq1));
    }

    #[test]
    fn build_upgrade_on_nonexistent_slot_returns_unknown_slot() {
        let mut raft = fresh_raft();
        let eq = EquipmentCard::new(EquipmentKind::Telescope, 0);
        let err = raft
            .build_upgrade(SlotId::Extension(999), eq)
            .unwrap_err();
        assert_eq!(err, RaftError::UnknownSlot(SlotId::Extension(999)));
    }
}

// ---------- ResourceCost ------------------------------------------------

mod cost {
    use super::*;
    use playtest_shipwreck::InsufficientResources;

    #[test]
    fn can_pay_true_with_sufficient_inventory() {
        let cost = ResourceCost::new([1, 2, 0, 0, 1]);
        let inv = [2, 2, 0, 0, 1];
        assert!(cost.can_pay(&inv));
    }

    #[test]
    fn can_pay_false_with_insufficient_inventory() {
        let cost = ResourceCost::new([1, 2, 0, 0, 1]);
        let inv = [0, 2, 0, 0, 1];
        assert!(!cost.can_pay(&inv));
    }

    #[test]
    fn pay_decrements_inventory_on_success() {
        let cost = ResourceCost::new([0, 2, 1, 0, 1]);
        let mut inv = [3, 2, 1, 5, 4];
        cost.pay(&mut inv).unwrap();
        assert_eq!(inv, [3, 0, 0, 5, 3]);
    }

    #[test]
    fn pay_leaves_inventory_untouched_on_failure() {
        let cost = ResourceCost::new([0, 10, 0, 0, 0]);
        let mut inv = [3, 2, 1, 5, 4];
        let err: InsufficientResources = cost.pay(&mut inv).unwrap_err();
        assert_eq!(err.cost, cost);
        assert_eq!(inv, [3, 2, 1, 5, 4], "inventory untouched on error");
    }

    #[test]
    fn resource_all_matches_index() {
        for r in Resource::ALL {
            assert_eq!(Resource::ALL[r.index()], r);
        }
    }
}

// ---------- Serde round-trip --------------------------------------------

mod serde_tests {
    use super::*;
    use playtest_shipwreck::card::BaseRaftSide;

    fn roundtrip(card: Card) {
        let json = serde_json::to_string(&card).expect("serialize");
        let back: Card = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(card, back, "roundtrip failed: {json}");
    }

    #[test]
    fn player_card_roundtrips() {
        roundtrip(Card::Player(PlayerCard::new(
            PlayerCardId::Professor,
            1,
            1,
            Some(PlayerSkill::ConstructWithOneFewerResource),
        )));
    }

    #[test]
    fn base_raft_roundtrips() {
        roundtrip(Card::BaseRaft(BaseRaftCard::new(BaseRaftSide::Left)));
        roundtrip(Card::BaseRaft(BaseRaftCard::new(BaseRaftSide::Right)));
    }

    #[test]
    fn raft_extension_roundtrips() {
        roundtrip(Card::RaftExtension(RaftExtensionCard::new(17)));
    }

    #[test]
    fn equipment_roundtrips() {
        roundtrip(Card::Equipment(EquipmentCard::new(
            EquipmentKind::Telescope,
            0,
        )));
    }

    #[test]
    fn item_roundtrips() {
        for r in Resource::ALL {
            roundtrip(Card::Item(ItemCard::new(r)));
        }
    }

    #[test]
    fn event_roundtrips() {
        roundtrip(Card::Event(EventCard::Shark));
        roundtrip(Card::Event(EventCard::Typhoon));
        roundtrip(Card::Event(EventCard::FlyingFish));
    }

    #[test]
    fn tagged_enum_emits_card_type_field() {
        let json = serde_json::to_string(&Card::Event(EventCard::Shark)).unwrap();
        assert!(
            json.contains("\"card_type\""),
            "expected tag field: {json}"
        );
        assert!(json.contains("Event"), "expected variant name: {json}");
    }
}
