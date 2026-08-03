//! Card types for Great Gyre.
//!
//! Every physical card in the box gets a stable [`CardInstanceId`]
//! assigned once, at setup (see `crate::pool`). [`Card`] pairs that id
//! with a [`CardKind`] — the printed face. `CardKind` alone identifies
//! *what* a card is; `Card` identifies *which physical copy*, which is
//! what the wire protocol (actions, events) references so replay and
//! the web UI can point at an exact card even when duplicates exist
//! (e.g. six `Net`s).
//!
//! Data tables (hope / hand-tab / food / space rules / cost / print
//! counts) are sourced verbatim from `docs/greatgyre.md`'s survivor and
//! modification tables and its deck-composition section.

use serde::{Deserialize, Serialize};

use crate::resource::{Resource, ResourceCost};

/// Stable identity for one physical card, assigned once at setup.
/// Monotonic counter rather than a UUID — setup is deterministic given
/// `(seed, cfg)`, so a plain counter keeps ids stable across identical
/// replays without pulling in a random-uuid dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CardInstanceId(pub u32);

/// The twelve named survivors (1 copy each; chosen at the draft or
/// drawn later from the deck).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurvivorId {
    Captain,
    Purser,
    Fisher,
    FirstMate,
    Survivalist,
    Influencer,
    Stowaway,
    Pirate,
    Millionaire,
    Porter,
    Athlete,
    Swimmer,
}

impl SurvivorId {
    pub const ALL: [SurvivorId; 12] = [
        Self::Captain,
        Self::Purser,
        Self::Fisher,
        Self::FirstMate,
        Self::Survivalist,
        Self::Influencer,
        Self::Stowaway,
        Self::Pirate,
        Self::Millionaire,
        Self::Porter,
        Self::Athlete,
        Self::Swimmer,
    ];

    /// Hope icons printed on the card. `docs/greatgyre.md` survivors table.
    #[must_use]
    pub const fn hope(self) -> u8 {
        match self {
            Self::Captain => 20,
            Self::Purser | Self::Millionaire => 5,
            Self::Fisher | Self::FirstMate | Self::Stowaway | Self::Pirate | Self::Porter
            | Self::Swimmer | Self::Survivalist => 10,
            Self::Influencer | Self::Athlete => 0,
        }
    }

    /// Max-hand-size tab. Base +1 for every survivor except Purser,
    /// which grants +2 ("+2 hand size instead of +1").
    #[must_use]
    pub const fn hand_tab(self) -> i8 {
        match self {
            Self::Purser => 2,
            _ => 1,
        }
    }

    /// Food contribution. Base -1 for every survivor except
    /// Survivalist (needs no food, 0) and Swimmer (+1).
    #[must_use]
    pub const fn food(self) -> i8 {
        match self {
            Self::Survivalist => 0,
            Self::Swimmer => 1,
            _ => -1,
        }
    }

    /// Whether placing this survivor consumes a raft space. Only
    /// Stowaway is exempt ("occupies no space").
    #[must_use]
    pub const fn occupies_space(self) -> bool {
        !matches!(self, Self::Stowaway)
    }
}

/// The eight buildable modification kinds. Dead Fish is deliberately
/// excluded — it is never built (hand-only reaction card), so it gets
/// its own [`CardKind::DeadFish`] variant instead of living here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModificationKind {
    Quarterdeck,
    Sail,
    Garden,
    RainTrap,
    Toolkit,
    Net,
    FishingRod,
    Telescope,
}

impl ModificationKind {
    pub const ALL: [ModificationKind; 8] = [
        Self::Quarterdeck,
        Self::Sail,
        Self::Garden,
        Self::RainTrap,
        Self::Toolkit,
        Self::Net,
        Self::FishingRod,
        Self::Telescope,
    ];

    /// Print count. `docs/greatgyre.md` modifications table.
    #[must_use]
    pub const fn print_count(self) -> u8 {
        match self {
            Self::Quarterdeck | Self::Sail | Self::Garden | Self::RainTrap => 3,
            Self::Toolkit => 1,
            Self::Net => 6,
            Self::FishingRod | Self::Telescope => 2,
        }
    }

    #[must_use]
    pub const fn cost(self) -> ResourceCost {
        match self {
            Self::Quarterdeck => ResourceCost::new(0, 2, 0),
            Self::Sail | Self::FishingRod => ResourceCost::new(1, 1, 0),
            Self::Garden => ResourceCost::new(0, 1, 1),
            Self::RainTrap => ResourceCost::new(1, 0, 1),
            Self::Toolkit => ResourceCost::new(1, 1, 1),
            Self::Net => ResourceCost::new(2, 0, 0),
            Self::Telescope => ResourceCost::new(0, 0, 2),
        }
    }

    /// Whether building this modification consumes a raft space.
    /// Net, Fishing Rod, and Telescope do not.
    #[must_use]
    pub const fn occupies_space(self) -> bool {
        !matches!(self, Self::Net | Self::FishingRod | Self::Telescope)
    }

    /// Extra raft capacity this modification *provides* once built.
    /// Only Quarterdeck provides any ("occupies 1, provides 2" — a net
    /// +1 space).
    #[must_use]
    pub const fn provides_capacity(self) -> u8 {
        match self {
            Self::Quarterdeck => 2,
            _ => 0,
        }
    }

    /// Food contribution while built.
    #[must_use]
    pub const fn food(self) -> i8 {
        match self {
            Self::Garden | Self::RainTrap | Self::FishingRod => 2,
            Self::Net => 1,
            _ => 0,
        }
    }

    /// Max-hand-size adjustment while built. Fishing Rod and Telescope
    /// each subtract 1.
    #[must_use]
    pub const fn hand_tab(self) -> i8 {
        match self {
            Self::FishingRod | Self::Telescope => -1,
            _ => 0,
        }
    }

    #[must_use]
    pub const fn hope(self) -> u8 {
        match self {
            Self::Quarterdeck | Self::Toolkit | Self::FishingRod | Self::Telescope => 5,
            Self::Sail | Self::Garden | Self::RainTrap => 10,
            Self::Net => 0,
        }
    }
}

/// The seven event-card kinds. Effects are out of scope until Unit 4 —
/// this module only models the card identities and print counts so the
/// deck can be built and shuffled correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    SharkAttack,
    OctopusAttack,
    Walrus,
    LoveBoat,
    Storm,
    WorkDay,
    LandSighting,
}

impl EventKind {
    pub const ALL: [EventKind; 7] = [
        Self::SharkAttack,
        Self::OctopusAttack,
        Self::Walrus,
        Self::LoveBoat,
        Self::Storm,
        Self::WorkDay,
        Self::LandSighting,
    ];

    /// Print count. `docs/greatgyre.md` Event Deck (10): Shark×2,
    /// Storm×2, WorkDay×2, LandSighting×1, Octopus×1, Walrus×1,
    /// LoveBoat×1.
    #[must_use]
    pub const fn print_count(self) -> u8 {
        match self {
            Self::SharkAttack | Self::Storm | Self::WorkDay => 2,
            Self::OctopusAttack | Self::Walrus | Self::LoveBoat | Self::LandSighting => 1,
        }
    }
}

/// The printed face of a physical card — everything needed to look up
/// its rules-relevant stats. Distinct from [`Card`], which pairs a
/// `CardKind` with its stable instance id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CardKind {
    Survivor(SurvivorId),
    Modification(ModificationKind),
    /// Hand-only reaction card; never built, never occupies a space.
    DeadFish,
    Event(EventKind),
    Resource(Resource),
    /// Shared-pile raft extension; +2 spaces once built.
    RaftExtension,
    /// One half of a player's fixed base raft. Never destroyed or
    /// discarded; carries no hope.
    RaftLeft,
    /// The other half of a player's fixed base raft. Contributes +1
    /// Food (see `crate::turns::compute_food`); carries no hope.
    RaftRight,
}

impl CardKind {
    /// Hope icons contributed while this card is face-up on a raft.
    /// Only survivors and modifications carry hope; base raft cards,
    /// extensions, resources, Dead Fish, and event cards do not.
    #[must_use]
    pub fn hope(self) -> u32 {
        match self {
            Self::Survivor(s) => u32::from(s.hope()),
            Self::Modification(m) => u32::from(m.hope()),
            Self::DeadFish
            | Self::Event(_)
            | Self::Resource(_)
            | Self::RaftExtension
            | Self::RaftLeft
            | Self::RaftRight => 0,
        }
    }
}

/// One physical card: a stable identity plus its printed face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Card {
    pub id: CardInstanceId,
    pub kind: CardKind,
}

impl Card {
    #[must_use]
    pub const fn new(id: CardInstanceId, kind: CardKind) -> Self {
        Self { id, kind }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn survivor_hope_sum_matches_spec_table() {
        let total: u32 = SurvivorId::ALL.iter().map(|s| u32::from(s.hope())).sum();
        // 20+5+10+10+10+0+10+10+5+10+0+10 = 100
        assert_eq!(total, 100);
    }

    #[test]
    fn modification_print_counts_sum_to_twenty_three() {
        let total: u16 = ModificationKind::ALL
            .iter()
            .map(|m| u16::from(m.print_count()))
            .sum();
        assert_eq!(total, 23);
    }

    #[test]
    fn event_print_counts_sum_to_ten() {
        let total: u16 = EventKind::ALL.iter().map(|e| u16::from(e.print_count())).sum();
        assert_eq!(total, 10);
    }

    #[test]
    fn only_stowaway_is_spaceless() {
        for s in SurvivorId::ALL {
            assert_eq!(s.occupies_space(), s != SurvivorId::Stowaway, "{s:?}");
        }
    }

    #[test]
    fn quarterdeck_nets_one_extra_space() {
        let occupies = i16::from(ModificationKind::Quarterdeck.occupies_space());
        let provides = i16::from(ModificationKind::Quarterdeck.provides_capacity());
        assert_eq!(provides - occupies, 1);
    }
}
