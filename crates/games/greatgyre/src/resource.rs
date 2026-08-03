//! The three raft-building resources + cost accounting.
//!
//! Unlike ShipWreck, resources in Great Gyre are not an abstract
//! inventory counter — they are literal cards that sit in a player's
//! hand (see `docs/greatgyre.md`: "pay its resource cost from hand to
//! the Discard Pile"). They count toward hand-size limits like any
//! other card. [`ResourceCost`] is only used to check affordability and
//! to select *which* resource cards a build consumes.

use serde::{Deserialize, Serialize};

/// One of the three raft-building resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    Rope,
    Wood,
    Plastic,
}

impl Resource {
    /// The three resources in canonical index order.
    pub const ALL: [Resource; 3] = [Self::Rope, Self::Wood, Self::Plastic];

    /// Print count in the deck (`docs/greatgyre.md` deck-composition
    /// table: Rope×31, Wood×25, Plastic×21).
    #[must_use]
    pub const fn print_count(self) -> u16 {
        match self {
            Self::Rope => 31,
            Self::Wood => 25,
            Self::Plastic => 21,
        }
    }

    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Rope => 0,
            Self::Wood => 1,
            Self::Plastic => 2,
        }
    }
}

/// A resource cost — how many of each [`Resource`] a build requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceCost {
    pub rope: u8,
    pub wood: u8,
    pub plastic: u8,
}

impl ResourceCost {
    #[must_use]
    pub const fn new(rope: u8, wood: u8, plastic: u8) -> Self {
        Self { rope, wood, plastic }
    }

    #[must_use]
    pub const fn amount_of(&self, resource: Resource) -> u8 {
        match resource {
            Resource::Rope => self.rope,
            Resource::Wood => self.wood,
            Resource::Plastic => self.plastic,
        }
    }

    #[must_use]
    pub const fn total(&self) -> u8 {
        self.rope + self.wood + self.plastic
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_counts_sum_to_seventy_seven() {
        let total: u16 = Resource::ALL.iter().map(|r| r.print_count()).sum();
        assert_eq!(total, 77);
    }

    #[test]
    fn resource_all_is_stable_order_matching_index() {
        for r in Resource::ALL {
            assert_eq!(Resource::ALL[r.index()], r, "{r:?}");
        }
    }
}
