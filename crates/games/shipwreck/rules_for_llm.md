# ShipWreck — Rules for an LLM Player

You are playing ShipWreck, a 2–4 player raft-building card game. Each
player starts on a small raft and works to grab floating wreckage,
feed themselves, extend their raft, and build equipment upgrades. The
player with the most Rescue Points when the wreckage runs out wins.

## Win condition

When all wreckage cards have been claimed, the game ends. Rescue
points are totaled. Highest total wins.

Tie-breakers, in order:

1. Longer raft (more cards in your raft line).
2. More equipment upgrades ("invensions") on your raft.
3. Tie remains a tie.

## Cards

### Player cards (seven characters)

Each character has a unique ability, a rescue-point value, and an
ongoing food cost. Food cost is paid per round; skipping food has
consequences set by the engine.

- **Professor** — 1 Rescue Point. Can construct any upgrade with one
  fewer resource of any type. Costs 1 food per round.
- **Mary Anne** — 1 Rescue Point. Reaches one space further when
  grabbing wreckage. Costs 1 food per round.
- **Movie Star** — 2 Rescue Points. Costs 1 food per round.
- **Gillagun** — 1 Rescue Point. May skip a food payment once per
  game without penalty. Normally costs 1 food per round.
- **Millionaire** — 2 Rescue Points. Costs 2 food per round.
- **Million Heiress** — 2 Rescue Points. Costs 2 food per round.
- **Wilson (Volleyball)** — 1 Rescue Point. No food cost.

At setup, player cards are shuffled and dealt. Remaining player cards
are mixed into the wreckage deck — they may surface later and be
placed onto your raft as additional crew.

### Raft cards

Every player starts with two **base raft cards** (left and right). The
left and right cards always stay at the outer edges of the raft.

The raft is extended by splitting two adjacent raft cards and
inserting a new **raft extension card** between them. Every base and
extension card has exactly one upgrade slot. There is effectively an
unlimited supply of extension cards (nominally 40 in the deck).

### Equipment upgrades

You may construct an equipment upgrade onto any open raft slot (base
or extension) by paying its exact resource cost from your hand. The
resources go back to the box — they are consumed permanently. You
must have an open slot to build.

Available equipment upgrades:

- **Fishing Poles** (x2 available) — Gives 2 food per round. Cost:
  1 wood, 1 rope, 1 wire.
- **Telescope** (x1) — Reach one extra space when grabbing wreckage.
  Cost: 2 wood, 1 plastic, 1 wire.
- **Auto Nets** (x3) — Grab two wreckage cards each turn instead of
  one. Cost: 1 cloth, 1 rope, 1 wood.
- **Steel Cordage** (x2) — Defends against a Shark attack. Cost: 2
  wire, 1 plastic.
- **Rain Catchers** (x5) — Gives 1 food per round. Cost: 2 cloth,
  1 wood.

### Wreckage resource cards

Five resource types, 30 of each:

- Plastic (30)
- Wood (30)
- Rope (30)
- Cloth (30)
- Wire (30)

### Wreckage event cards

- **Shark** — Choose a target player and an invention or raft
  extension to attack.
- **Typhoon** — All players lose an invention or raft extension.
- **Flying fish** — Counts as extra food for one turn only.

## Setup

1. Deal player cards — one or more per player depending on variant.
2. Mix remaining player cards into the wreckage deck and shuffle.
3. Deal six wreckage cards face-down to each player as a starting
   hand.
4. Deal the remaining wreckage cards face-up in a line in front of
   each player, going around the table until the deck is exhausted.
   These are the floating wreckage you can reach.

## Gameplay

The turn order proceeds clockwise. On your turn, take one of the
following actions:

- **Place a player card** from your hand onto an open raft slot. The
  player card's passive ability becomes active.
- **Pick up a wreckage card** within your reach and add it to your
  hand. Default reach is one space; Mary Anne and Telescope increase
  reach. Auto Nets lets you grab two wreckage cards instead of one.
- **Play an event card** from your hand (Shark / Typhoon / Flying
  fish). Resolve its effect immediately.
- **Build the current equipment upgrade card** by paying its exact
  resource cost and placing it onto an open raft slot.

At any time (not just on your turn), you may extend your raft by one
card — this is a zero-action option. It creates a new open slot.

Each round, every player pays the food cost of every player card on
their raft. Skipping food has consequences defined by the engine.

## What you will receive each turn

A JSON object with three keys:

- `public_view`: the current game state as visible to you. Contains
  your hand, the wreckage line you can reach, the current equipment
  upgrade card on offer, every player's raft (cards, occupied slots,
  upgrades), food counters, and whose turn it is.
- `legal_actions`: an array of actions you may take right now. Choose
  by integer index into this array (0-based).
- `scratch`: your persistent memory from prior turns: `plan`, `notes`,
  `turn_log`.

## How to reply

Respond with a single JSON object and nothing else. No markdown
fences, no commentary. Exactly these keys:

- `action_index`: integer, 0-based index into the `legal_actions`
  array you received this turn.
- `plan`: short string capturing your current strategic intent.
- `notes`: short string with tactical observations to carry forward.

Do not invent actions. Do not return an index past the end of the
`legal_actions` array. If the engine offered you one action, choose
it.

## Strategy notes

- Early game: grab resources you can combine into a concrete upgrade.
  Auto Nets + Telescope + Fishing Poles are the strongest synergies.
- Mid-game: extend your raft when you have upgrades to place. Extra
  raft length is also a tie-breaker.
- Watch food costs. Two high-point characters (Millionaire, Million
  Heiress) each cost 2 food per round. Without Fishing Poles or Rain
  Catchers you will go hungry quickly.
- Event cards are single-use weapons. Save Shark for a player who
  has committed to a fragile upgrade; save Typhoon for the lead.
