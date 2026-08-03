//! 1000-game random-vs-random soak test (Unit 2).
//!
//! Per the plan, this is a "small" soak (not `#[ignore]`'d, unlike
//! ShipWreck's 10K-game variant) — it runs in the default test pass.
//!
//! Targets:
//! - Zero panics, zero engine errors.
//! - Every game terminates in under 10,000 ticks.
//! - Card conservation holds at the end of every game (every printed
//!   card is still in exactly one zone, and the total count matches
//!   the catalog size for that player count).

use std::time::Instant;

use playtest_adapters::{ProductionRng, StubGameEventSink, StubRng};
use playtest_agents::RandomAgent;
use playtest_core::{Agent, Game, GameLoop};
use playtest_greatgyre::pool::total_catalog_size;
use playtest_greatgyre::{Card, GreatGyreConfig, GreatGyreGame};

const NUM_GAMES: u64 = 1_000;
const TICK_BUDGET: u64 = 10_000;

fn all_card_ids(state: &playtest_greatgyre::GameState) -> Vec<Card> {
    let mut out = Vec::new();
    for p in &state.players {
        out.push(p.raft_left);
        out.push(p.raft_right);
        out.extend(p.hand.iter().copied());
        out.extend(p.current.iter().map(|c| c.card));
        out.extend(p.built_extensions.iter().copied());
        out.extend(p.placed.iter().map(|pc| pc.card));
        // The Walrus-in-play zone (Unit 4): Walrus cards blocking a
        // space on this player's raft are neither in anyone's hand nor
        // the shared Discard Pile.
        out.extend(p.blocked_by_walrus.iter().copied());
    }
    out.extend(state.deep_sea_deck.iter().copied());
    out.extend(state.final_round_deck.iter().copied());
    out.extend(state.event_deck.iter().copied());
    out.extend(state.discard_pile.iter().copied());
    out.extend(state.extension_pile.iter().copied());
    out
}

#[tokio::test(flavor = "current_thread")]
async fn random_self_play_1000_games_terminate_and_conserve_cards() {
    let start = Instant::now();
    let mut games_completed: u64 = 0;
    let mut total_ticks: u64 = 0;
    let mut longest: u64 = 0;

    for seed in 0..NUM_GAMES {
        // Cycle player counts 2..=4 across seeds for broader coverage.
        let n = 2 + u8::try_from(seed % 3).expect("0..3 fits in u8");
        let game = GreatGyreGame::new();
        let cfg = GreatGyreConfig::new(n).expect("valid player count");
        let state = game.initial_state(seed, &cfg);
        let mut loop_ = GameLoop::new(&game, state);
        let mut chance_rng = ProductionRng::from_seed(seed ^ 0xC0FF_EE00);
        let mut sink = StubGameEventSink::new();
        let mut agents: Vec<Box<dyn Agent<GreatGyreGame>>> = (0..n)
            .map(|p| {
                Box::new(RandomAgent::<GreatGyreGame, _>::new(StubRng::seeded(
                    seed.wrapping_mul(101).wrapping_add(u64::from(p)),
                ))) as Box<dyn Agent<GreatGyreGame>>
            })
            .collect();

        let result = loop_
            .run(agents.as_mut_slice(), &mut chance_rng, &mut sink)
            .await
            .unwrap_or_else(|e| panic!("seed {seed} (n={n}): game loop errored: {e}"));

        assert_eq!(
            result.scores.len(),
            usize::from(n),
            "seed {seed}: wrong score count"
        );
        assert!(
            loop_.tick() < TICK_BUDGET,
            "seed {seed}: game took {} ticks, exceeding the {TICK_BUDGET} budget",
            loop_.tick()
        );

        // Card conservation: every printed card is in exactly one zone.
        let cards = all_card_ids(loop_.state());
        let expected = total_catalog_size(n);
        let actual = u32::try_from(cards.len()).expect("card count fits in u32");
        assert_eq!(
            actual, expected,
            "seed {seed} (n={n}): card count mismatch at game end"
        );
        let mut ids: Vec<_> = cards.iter().map(|c| c.id).collect();
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(
            ids.len(),
            before,
            "seed {seed} (n={n}): duplicate CardInstanceId at game end"
        );

        total_ticks += loop_.tick();
        longest = longest.max(loop_.tick());
        games_completed += 1;
    }

    let elapsed = start.elapsed();
    #[allow(clippy::cast_precision_loss)]
    let games_f = games_completed as f64;
    #[allow(clippy::cast_precision_loss)]
    let avg_ticks = total_ticks as f64 / games_f;
    println!(
        "\n=== greatgyre random_self_play soak ===\n\
         games: {games_completed} / {NUM_GAMES}\n\
         elapsed: {:.2}s\n\
         avg ticks/game: {avg_ticks:.1} (longest: {longest})\n",
        elapsed.as_secs_f64(),
    );

    assert_eq!(games_completed, NUM_GAMES, "not every game completed");
}
