# Cribbage — Rules for an LLM Player

You are playing two-player standard Cribbage. First player to 121 points
wins. This document is the complete rules reference you need to make
legal decisions on every turn.

## The deck

Standard 52-card deck. Four suits: Clubs, Diamonds, Hearts, Spades.
Thirteen ranks: Ace, 2, 3, 4, 5, 6, 7, 8, 9, 10, Jack, Queen, King.

Two values matter and they differ for face cards:

- **Pegging value** (used for the 15/31 target during pegging):
  Ace = 1, 2 through 10 = face, Jack = 10, Queen = 10, King = 10.
- **Rank ordering** (used for runs, pairs, and comparing cards):
  Ace = 1, 2 through 10 = 2–10, Jack = 11, Queen = 12, King = 13.

A pair requires equal rank ordering, so a Jack pairs only with another
Jack, not with a 10.

## The turn flow for a single hand

A Cribbage game is a sequence of hands. Each hand proceeds through
phases in this fixed order: Deal, Discard, Cut, Pegging, Show, then the
dealer switches and the next hand starts. The first dealer is seat 0.

### Deal

Six cards to each player. This is a chance event; you will never be
asked to act during Deal.

### Discard

Each player must discard exactly two cards from their six-card hand
into the crib. The crib belongs to the dealer and will be scored as a
fourth hand for the dealer after the main show.

- Non-dealer discards first, then dealer.
- Keep four cards in hand for pegging and the show.
- Strategy: keep cards that make pairs, runs, fifteens, and flushes
  with each other; avoid giving good cards to the opponent's crib if
  you are non-dealer; feed the crib if you are dealer.

Action type during this phase: `DiscardToCrib(card_a, card_b)`. The two
cards must both be in your current hand.

### Cut (chance)

The top card of the remaining deck is turned up as the **starter**.
You will never be asked to act during Cut.

- **Nibs (his heels)**: if the starter is a Jack, the dealer scores 2
  points immediately. The engine handles this automatically; you do
  not select it.

### Pegging

The starter is set aside (it is not played from anyone's hand during
pegging; it only counts in the Show). Non-dealer leads the first card.
Players alternate turns.

The running total is the sum of pegging values of all cards currently
on the stack. The running total must not exceed 31. Score as you play.

Legal actions during pegging:

- `PlayCard(card)` — play a card from your remaining hand. Legal only
  if the card is in your hand and if adding its pegging value leaves
  the running total at 31 or below.
- `SayGo` — legal only when you cannot play any card legally (every
  card in your hand would push the total past 31). You may not say Go
  if a legal play exists; the engine rejects that as an illegal
  action.

Scoring during pegging, awarded the moment the play resolves:

- **Fifteen**: running total becomes exactly 15 → +2.
- **Thirty-one**: running total becomes exactly 31 → +2 and the stack
  resets.
- **Pair**: the last two cards on the stack share rank ordering → +2.
- **Triple** (three of a rank): last three cards → +6.
- **Quadruple** (four of a rank): last four cards → +12.
- **Run**: the last N cards on the stack form a run of N consecutive
  ranks in any order, N ≥ 3. Score +N. The run must include the card
  you just played and continue consecutively back through the top of
  the stack; an earlier card of the run can be "re-started" only by
  playing cards that extend the sequence from the most recent end.
- **Go / Last card**: when the current player cannot play (says Go)
  and the opposing player also cannot or chooses not to continue, the
  last player to have played scores +1 for "last card" — but only if
  the running total was below 31. If the last card hit exactly 31,
  the 31 itself already scored; no extra last-card point.

After a stack reset (a 31, or both players said Go), the player who
did NOT play the last card leads the next sub-round. When both players
have exhausted their hands, pegging ends and the Show begins.

### Show

Hands are counted in this order: non-dealer's hand, dealer's hand,
dealer's crib. For each hand (plus crib), combine the four cards in
hand with the starter to form a five-card group and score it.

Scoring categories (exhaustive — the engine computes them, you do not
need to):

- **Fifteens**: every distinct subset of the five cards whose pegging
  values sum to exactly 15 scores +2. Multiple non-disjoint subsets
  all count.
- **Pairs**: every pair of equal-rank cards scores +2. Three of a
  kind = three pairs = +6. Four of a kind = six pairs = +12.
- **Runs**: every distinct run of three or more consecutive ranks
  scores +N. Duplicate ranks produce "double runs" (e.g., 4-5-6-6 is
  two runs of three = +6) and "triple runs" (4-4-4-5-6 = three runs
  of three = +9) and "double-double runs" (4-4-5-5-6 = four runs of
  three = +12).
- **Flush**: four cards in hand all the same suit = +4. If the
  starter is also that suit, +5. Note: crib flush requires the
  starter to match too — four-card flush alone does not score in the
  crib.
- **Nobs**: if your hand (or the crib, for the dealer) contains the
  Jack whose suit matches the starter, +1. Nobs is only for the Jack
  in hand, not for a Jack starter (that's nibs, scored earlier).

### End of hand

The dealer rotates to the other player; the non-dealer for this hand
is dealer for the next. Re-shuffle, deal, repeat.

## Winning condition

The first player whose running score reaches or passes 121 wins. The
engine terminates the game the moment either score crosses the line,
even mid-pegging or mid-show. You cannot "skunk-save" by declining a
score; the engine always applies points in strict order.

## What you will receive each turn

A JSON object with three keys:

- `public_view`: the current game state as visible to you. Fields
  include `player` (your seat: 0 or 1), `own_hand` (the cards in your
  hand right now), `crib_size` (0–4), `starter` (the cut card, once
  revealed), `pegging_stack`, `running_total` (0–31), `board` (both
  players' scores), `phase` (one of `deal`, `discard`, `cut`,
  `pegging`, `show`, `score_crib`, `finished`), and `to_act` (whose
  turn it is — always you when you are being asked).
- `legal_actions`: an array of every action you may legally take this
  turn. Choose by integer index into this array (0-based).
- `scratch`: your persistent memory from prior turns: `plan`, `notes`,
  `turn_log`.

## How to reply

Respond with a single JSON object and nothing else. No markdown fences,
no commentary before or after. Exactly these keys:

- `action_index`: integer, 0-based index into the `legal_actions` array
  you received this turn.
- `plan`: short string capturing your current strategic intent (kept
  until you rewrite it).
- `notes`: short string with tactical observations to carry forward.

Do not invent actions. Do not return an index past the end of the
legal_actions array. If the engine offered you one action, choose it.
