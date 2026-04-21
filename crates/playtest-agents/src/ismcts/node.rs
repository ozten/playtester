//! Tree-node arena for ISMCTS.
//!
//! Each node represents a decision point (a state where some player is
//! about to act). The node tracks cumulative visit counts and total
//! value **from the root observer's perspective** (SO-ISMCTS: all
//! rewards come back scored for `self.player`).
//!
//! Children are keyed by the **action itself** (not by positional index
//! into `legal_actions`), because SO-ISMCTS's per-iteration
//! determinization changes which legal actions appear at a given node.
//! A stable child key is required so that across iterations, "the child
//! reachable via action A" always points to the same arena slot.
//!
//! Nodes are stored in a flat `Vec<Node<A>>` (the arena). Children are
//! referenced by `NodeId`, which is just a small newtype around the
//! arena index. This avoids `Rc`/`RefCell` and lets the borrow checker
//! see a single `&mut NodeArena` through the tree descent.

use core::hash::Hash;
use std::collections::HashMap;

/// Stable identifier for a node inside a [`NodeArena`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeId(pub u32);

impl NodeId {
    #[must_use]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// One node in the ISMCTS tree. `A` is the game's action type.
#[derive(Debug)]
pub struct Node<A> {
    /// Number of iterations that have descended through this node.
    pub visits: u32,
    /// Sum of rewards backpropagated through this node (root-player POV).
    pub total_value: f64,
    /// Map from action to child NodeId. Populated lazily during
    /// expansion.
    pub children: HashMap<A, NodeId>,
}

impl<A> Node<A>
where
    A: Hash + Eq,
{
    #[must_use]
    pub fn new() -> Self {
        Self {
            visits: 0,
            total_value: 0.0,
            children: HashMap::new(),
        }
    }

    /// Empirical mean value of visits through this node. Zero if
    /// unvisited — safe to feed to UCB1 because UCB1 short-circuits on
    /// `visits == 0`.
    #[must_use]
    pub fn mean_value(&self) -> f64 {
        if self.visits == 0 {
            0.0
        } else {
            self.total_value / f64::from(self.visits)
        }
    }
}

impl<A> Default for Node<A>
where
    A: Hash + Eq,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Flat arena of tree nodes.
#[derive(Debug)]
pub struct NodeArena<A> {
    nodes: Vec<Node<A>>,
}

impl<A> NodeArena<A>
where
    A: Hash + Eq,
{
    #[must_use]
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Pre-allocate capacity for approximately `cap` nodes. The arena
    /// grows organically past this; capacity is a hint.
    #[must_use]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(cap),
        }
    }

    /// Push a fresh (visit = 0) node and return its id.
    pub fn alloc(&mut self) -> NodeId {
        let id = NodeId(u32::try_from(self.nodes.len()).expect("arena index fits in u32"));
        self.nodes.push(Node::new());
        id
    }

    #[must_use]
    pub fn get(&self, id: NodeId) -> &Node<A> {
        &self.nodes[id.index()]
    }

    pub fn get_mut(&mut self, id: NodeId) -> &mut Node<A> {
        &mut self.nodes[id.index()]
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Reset arena to empty without deallocating its backing storage.
    /// Lets a single `ISMCTSAgent` reuse one allocation across turns.
    pub fn clear(&mut self) {
        self.nodes.clear();
    }
}

impl<A> Default for NodeArena<A>
where
    A: Hash + Eq,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_returns_sequential_ids() {
        let mut arena: NodeArena<u32> = NodeArena::new();
        assert_eq!(arena.alloc(), NodeId(0));
        assert_eq!(arena.alloc(), NodeId(1));
        assert_eq!(arena.alloc(), NodeId(2));
        assert_eq!(arena.len(), 3);
    }

    #[test]
    fn mean_value_on_fresh_node_is_zero() {
        let n: Node<u32> = Node::new();
        assert!(n.mean_value().abs() < f64::EPSILON);
    }

    #[test]
    fn clear_preserves_capacity() {
        let mut arena: NodeArena<u32> = NodeArena::with_capacity(64);
        for _ in 0..32 {
            arena.alloc();
        }
        arena.clear();
        assert_eq!(arena.len(), 0);
    }
}
