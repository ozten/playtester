//! The five wreckage resources + cost accounting.
//!
//! Resources are stored as a fixed-size `[u8; 5]` array — one entry per
//! [`Resource`] variant, in the canonical order given by [`Resource::ALL`].
//! [`ResourceCost`] wraps the same shape but carries cost semantics:
//! it can be checked against an inventory ([`ResourceCost::can_pay`]) or
//! deducted from one ([`ResourceCost::pay`]).
//!
//! The raw `[u8; 5]` is used for the inventory because it plays nicely
//! with `[u8; 5]`-literal construction in tests and serde; the newtype
//! goes around `ResourceCost` because that's where the semantic weight
//! lives — a cost is not an inventory.

use serde::{Deserialize, Serialize};

/// Error returned by [`ResourceCost::pay`] when the inventory cannot
/// cover the cost. The inventory is left unchanged on failure.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("insufficient resources to pay cost {cost:?}")]
pub struct InsufficientResources {
    pub cost: ResourceCost,
}

/// One of the five wreckage resources.
///
/// Canonical order is given by [`Resource::ALL`]; [`Resource::index`]
/// returns the matching array index. These two must agree — the
/// invariant `Resource::ALL[r.index()] == r` is enforced by test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Resource {
    Plastic,
    Wood,
    Rope,
    Cloth,
    Wire,
}

impl Resource {
    /// The five resources in canonical index order.
    ///
    /// Changing this order is a breaking change for any serialized
    /// [`ResourceCost`] or inventory snapshot.
    pub const ALL: [Resource; 5] = [
        Self::Plastic,
        Self::Wood,
        Self::Rope,
        Self::Cloth,
        Self::Wire,
    ];

    /// The index into a `[u8; 5]` inventory for this resource.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Plastic => 0,
            Self::Wood => 1,
            Self::Rope => 2,
            Self::Cloth => 3,
            Self::Wire => 4,
        }
    }
}

/// A cost expressed as per-resource counts. Indexed in the same order
/// as [`Resource::ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceCost(pub [u8; 5]);

impl ResourceCost {
    /// A cost with the given per-resource amounts. The argument is
    /// indexed by [`Resource::index`].
    #[must_use]
    pub const fn new(counts: [u8; 5]) -> Self {
        Self(counts)
    }

    /// A zero-cost (nothing required).
    #[must_use]
    pub const fn free() -> Self {
        Self([0; 5])
    }

    /// Elementwise per-resource amounts.
    #[must_use]
    pub const fn amounts(&self) -> &[u8; 5] {
        &self.0
    }

    /// The amount of `resource` required.
    #[must_use]
    pub const fn amount_of(&self, resource: Resource) -> u8 {
        self.0[resource.index()]
    }

    /// True if `inventory` contains at least this cost's amount of each
    /// resource.
    #[must_use]
    pub fn can_pay(&self, inventory: &[u8; 5]) -> bool {
        self.0
            .iter()
            .zip(inventory.iter())
            .all(|(need, have)| have >= need)
    }

    /// Deduct this cost from `inventory`. On success, `inventory` is
    /// mutated in place.
    ///
    /// # Errors
    /// Returns [`InsufficientResources`] if any resource is short.
    /// The inventory is left unchanged on failure — no partial payment.
    pub fn pay(&self, inventory: &mut [u8; 5]) -> Result<(), InsufficientResources> {
        if !self.can_pay(inventory) {
            return Err(InsufficientResources { cost: *self });
        }
        for (slot, need) in inventory.iter_mut().zip(self.0.iter()) {
            *slot -= *need;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_all_is_stable_order_matching_index() {
        for r in Resource::ALL {
            assert_eq!(Resource::ALL[r.index()], r, "{r:?}");
        }
    }

    #[test]
    fn can_pay_true_when_all_resources_sufficient() {
        let cost = ResourceCost::new([1, 2, 0, 0, 1]);
        let inv = [1, 2, 0, 0, 1];
        assert!(cost.can_pay(&inv));
    }

    #[test]
    fn can_pay_false_when_any_resource_short() {
        let cost = ResourceCost::new([1, 2, 0, 0, 1]);
        let inv = [1, 1, 0, 0, 1];
        assert!(!cost.can_pay(&inv));
    }

    #[test]
    fn pay_decrements_inventory_on_success() {
        let cost = ResourceCost::new([0, 1, 1, 0, 1]);
        let mut inv = [3, 2, 1, 0, 1];
        cost.pay(&mut inv).unwrap();
        assert_eq!(inv, [3, 1, 0, 0, 0]);
    }

    #[test]
    fn pay_leaves_inventory_untouched_on_failure() {
        let cost = ResourceCost::new([0, 5, 0, 0, 0]);
        let mut inv = [3, 1, 0, 0, 0];
        let err = cost.pay(&mut inv).unwrap_err();
        assert_eq!(err.cost, cost);
        assert_eq!(inv, [3, 1, 0, 0, 0], "inventory must be untouched");
    }
}
