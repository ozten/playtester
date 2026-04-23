---
title: "feat: Comparative and counterfactual analysis (Phase 6)"
type: feat
status: active
date: 2026-04-23
origin: playtest-roadmap.md § "Phase 6 — Comparative and counterfactual analysis"
supersedes_section: "Phase 6 in the roadmap; second plan of the re-ordered remaining sequence P5 → P6 → P4 → P8 (P7 dropped 2026-04-23)."
---

# feat: Comparative and counterfactual analysis (Phase 6)

## Overview

Introduce `playtest compare <baseline-dir> <variant-dir>` — a new CLI subcommand that ingests two previously-run log directories, enumerates every numeric metric and critique signal present in both, runs appropriate parametric tests, applies Benjamini–Hochberg FDR correction by default (Bonferroni on opt-in), and emits a "what changed" markdown report sorted by significance. A designer running compare after two `playtest play` invocations sees every statistically-distinguishable regression and improvement in one document, grouped by severity and lens (mechanical metrics, per-agent win rates, subjective critique).

Bradley–Terry ratings land as an extension to `playtest matchup` (`--bradley-terry` flag), turning the existing win-rate matrix into a ranked agent table with MLE strengths. Restricted-play analysis lands as a ShipWreck-only per-event-card toggle — ShipWreckConfig gains `shark_enabled / typhoon_enabled / flying_fish_enabled` fields so a designer can run the 4-cohort "each card disabled individually" recipe and feed each pair to `playtest compare`.

This is the second phase in the re-ordered remaining sequence (**P5 → P6 → P4 → P8**; P7 dropped 2026-04-23). It turns the tool from "produces reports" into "guides iteration" — the capability the roadmap calls out as the one that earns the tool its place in a design loop.

## Problem Frame

Phases 0–5 answered "what does this game look like, mechanically and subjectively, across N games." Phase 6 answers a strictly harder question: **"is this change an improvement over the previous build?"** Without that, a designer tweaking card cost A from 4 to 3 cannot see whether the resulting win-rate bump is statistically real or within the noise floor. Three gaps remain after Phase 5 ships:

1. **No cross-run diff.** Every `playtest report` today describes one run. There is no machinery that takes two runs and flags the metrics that moved.
2. **No significance gate.** A naive "baseline had X, variant has Y, delta = Y − X" report in a 50-metric regime drowns in noise — 5% of non-moving metrics fire spurious 5% alarms at uncorrected α=0.05.
3. **No card-contribution measure.** Restricted-play analysis (Jaffe et al. 2012) compares games where a card is available to games where it is not, isolating its contribution to win rate. The engine has no hook to "disable card X" per-run.

Phase 6 closes all three.

## Requirements Trace

Drawn from `playtest-roadmap.md` § "Phase 6" and the architectural invariants in `CLAUDE.md`.

- **R6.1** — `playtest compare --baseline <dir> --variant <dir> --out <path>` ingests both log dirs, computes statistical deltas across every metric that appears in both, and writes a "what changed" markdown report to `<path>`. The two ingests use separate in-memory SQLite DBs so the compare pass never contaminates operator-visible ingest artifacts.
- **R6.2** — Mechanical deltas cover: (a) per-metric numeric means with 95% CI (Welch's t-test); (b) per-agent win rates with 95% CI (two-proportion z-test); (c) game-outcome distribution (winner / end_reason / average event count). Each test emits a p-value.
- **R6.3** — Subjective deltas (from Phase 5 data) cover: (a) per-question Likert mean deltas (Welch's t-test per question); (b) coded-tag frequency deltas (two-proportion z-test per tag). Runs with critique data in only one of the two dirs surface a note in the report rather than silently dropping the subjective section.
- **R6.4** — Multiple-comparison correction is **Benjamini–Hochberg** (FDR) by default; `--correction bonferroni` switches to Bonferroni. Significance flagging threads through the report — only corrected-significant metrics land in the "Flagged" section; uncorrected-significant-but-BH-rejected land in "Noise (rejected)" for auditability.
- **R6.5** — `playtest matchup --bradley-terry` extends the existing matchup subcommand with MLE Bradley–Terry ratings. Output gains a ranked table of agents by θ̂ + a log-odds 95% CI column. Existing matchup output is unchanged when the flag is absent.
- **R6.6** — `ShipWreckConfig` gains per-event-card toggles: `shark_enabled`, `typhoon_enabled`, `flying_fish_enabled`. Each defaults to `true`. The existing Phase-5 `events_enabled: bool` becomes a convenience meta-flag that ANDs with the per-card fields at setup time. `playtest play` gains `--shipwreck-disable-event <shark|typhoon|flying_fish>` (repeatable) so the restricted-play recipe can run without code edits.
- **R6.7** — Determinism: compare is a pure reader — no new events, no new sink behavior. Main JSONL log invariant unchanged. Per-card toggles compose cleanly with the existing config hash (changing any toggle produces a different `config_hash`, so the SQLite bucketing already separates cohorts).
- **R6.8** — Exit criterion A: a ShipWreck run with Shark doubled (or a similarly buffed variant) versus a baseline cohort produces at least one flagged regression or improvement in ≤ 10,000 games per cohort with corrected p < 0.05.
- **R6.9** — Exit criterion B: a "cosmetic" change (renamed card label, unchanged mechanics) between two cohorts produces zero flagged regressions across all mechanical metrics and critique signals. Because cosmetic changes don't change game behavior and the engine doesn't use display labels for hashing, this test is structurally passed once R6.4's FDR control is correctly wired.

## Scope Boundaries

- **No inline runner.** `playtest compare` does not spawn its own games. The user runs `playtest play` twice (with whatever config deltas they want) and feeds the two log dirs to compare.
- **No TOML game configs.** The roadmap's "config.toml" is out of scope for Phase 6. Users express config deltas via CLI flags on `playtest play` (existing flags + the new `--shipwreck-disable-event`).
- **No inferential tests beyond Welch/two-proportion.** No bootstrapping, no Bayesian posteriors, no sequential analysis. Keep the stats layer tight; add machinery only if R6.8 fails with the basic tests.
- **No Bradley–Terry over configs.** With only two subjects per compare pair, plain win-rate delta suffices. BT lives only in matchup where N ≥ 3 agents exist.
- **No restricted-play for Cribbage.** Cribbage's 52-card deck has no pre-game composition story. Attempting per-card restriction in Cribbage would require designing a synthetic "draw without X" that doesn't exist in the real rules.
- **No CFR or combo validation.** That's Phase 9b, and the roadmap marks it as expensive-and-optional.
- **No MAP-Elites or automated config search.** Phase 7 was dropped 2026-04-23.

### Deferred to Separate Tasks

- **Bayesian / sequential analysis**: separate plan if R6.8 reveals false-negative issues with Welch under small N.
- **Restricted-play for Cribbage**: needs a product decision first ("what does 'disable card X' mean for a 52-card deck?"). Separate plan, if any game ever gains a deck-construction mechanic.
- **TOML game configs + inline runner**: separate plan. Useful later when the surface area of `ShipWreckConfig` grows past a handful of fields.
- **Bradley–Terry diagnostics** (model fit χ², leverage plots): deferred. MLE point estimates + CIs are enough for the roadmap's call for "Bradley–Terry ratings over decks."
- **UI rendering of compare reports**: Phase 8. Phase 6 is CLI-only markdown.

## Context & Research

### Relevant Code and Patterns

- **`playtest-metrics/src/ingest.rs`** — `ingest_directory<G, R>(&mut Connection, &Path, game_name, &R)`. Compare calls this twice, once per input dir, against two separate `Connection::open_in_memory()` handles. Existing ingest already reads Phase 5's `<gid>.critique.jsonl` sidecars into `critique_likert` / `critique_tags` — no new ingest work.
- **`playtest-metrics/src/query.rs`** — canned queries (`agent_summaries`, `avg_numeric_metric`, `critique_likert_means`, etc.). Compare queries are structurally similar but cross-DB: they take two `Connection` handles and emit paired samples.
- **`playtest-metrics/src/markdown.rs::MarkdownBuilder`** — same builder the Phase-5 reporter uses. Compare's report is just another consumer.
- **`playtest-cli/src/commands/matchup.rs`** — Bradley–Terry lands as a new section + flag here. The matchup subcommand already runs the matches and produces a win-rate matrix; BT reads from the same tally.
- **`playtest-cli/src/commands/play.rs`** — the per-card `--shipwreck-disable-event` flag slots into the existing ShipWreck config-construction path (`ShipWreckConfig::new(n)` call site).
- **`crates/games/shipwreck/src/config.rs`** — home of `events_enabled` from Phase 5. The per-card fields go here. The setup step that retains event cards already reads the flag; the new flags compose with a small filter change in `setup.rs`.
- **`playtest-log/src/header::compute_config_hash`** — the hash already includes `ShipWreckConfig` in full, so adding fields automatically partitions the SQLite bucketing. No schema migration needed for compare's two-DB layout.

### Institutional Learnings

- **`docs/solutions/architecture-patterns/ephemeral-coordination-frame-vs-logged-event-2026-04-22.md`** — compare is a pure reader with no new event stream; the "coordination vs logged event" discipline applies by omission (we never write anything into the main JSONL log).
- **Phase 5's manual-benchmark discipline** (`docs/BENCHMARKS.md` R5.9) — real-LLM benchmarks live in BENCHMARKS.md as `#[ignore]` + documented recipe. R6.8 will follow the same pattern: a stubbed CI test proves the pipeline end-to-end; the manual recipe in BENCHMARKS.md drives the real cohort.
- **Phase 5's "don't bleed new signal into the main log" discipline** — compare reads from SQLite only, never writes back. The two ingests land in ephemeral in-memory DBs that die with the subcommand.

### External References

- **Benjamini & Hochberg (1995)** — "Controlling the False Discovery Rate: A Practical and Powerful Approach to Multiple Testing." Standard FDR procedure: rank p-values ascending, find the largest `k` such that `p_{(k)} ≤ k·α/m`, reject all `p_{(i)}` for `i ≤ k`.
- **Welch's t-test** — unequal-variance two-sample test. Appropriate when variance may differ between baseline and variant (game-metric distributions often shift in shape, not just location). Implemented in Rust from scratch; no external stats crate needed for the t-distribution CDF if we stay with a Normal approximation for n ≥ 30 per cohort, which holds at 10K games.
- **Bradley & Terry (1952)** / **Hunter (2004) MM algorithm** — iterative majorization-minimization for MLE of Bradley–Terry strengths. Converges in ~10–20 iterations for typical matchup-matrix sizes.
- **Jaffe, Miller, Andersen, Liu, Rafter, Zupko, Riedl (2012)** — "Evaluating Competitive Game Balance with Restricted Play." Card contribution to win rate ≈ (win-rate-with) − (win-rate-without), with each sampled in independent cohorts.

## Key Technical Decisions

- **Compare takes two log dirs, not two configs.** Smallest surface area. `playtest play` already writes deterministic-named log dirs; compare is a pure reader. (User selection.)
- **Benjamini–Hochberg FDR default, Bonferroni via flag.** BH has more power for many-metric comparison; Bonferroni stays available for conservative audits. (User selection.)
- **Bradley–Terry extends matchup, not compare.** Two subjects in a compare pair make BT degenerate. Matchup already has the N-way agent tally BT needs. (Planner decision, baked in.)
- **Restricted-play is ShipWreck-only via per-event-card toggles.** Cribbage has no restriction story; a per-game trait hook would be over-engineering for one game's signal. (User selection.)
- **Critique signals are part of compare.** Likert means + coded-tag frequencies flow through the same Welch / z-test pipeline; compare reports carry a "Subjective deltas" subsection. (User selection.)
- **Stats layer is in-house, not an external crate.** Welch's t-test, two-proportion z-test, BH, and Bonferroni are ~100 lines of Rust each. Adding a dependency (`statrs` or similar) for this scope is Cargo-graph weight without benefit; the unit tests pin every test against known reference values.
- **Normal approximation for t-distribution CDF** at n ≥ 30 per cohort. R6.8 requires 10K games per cohort, so the approximation error is well below the significance threshold. If later phases want exact t-distribution CDFs, add `statrs` then.
- **Two in-memory SQLite DBs during compare.** `Connection::open_in_memory()` twice, one per input dir. No on-disk artifact. Keeps compare idempotent and restartable without inventing ingest snapshotting.
- **`--shipwreck-disable-event <card>` on `playtest play`.** The per-card restricted-play recipe doesn't require a new subcommand — the user loops over the three event cards in a shell script, runs `playtest play` four times, then runs `playtest compare` three times. Documented in BENCHMARKS.md.
- **Game-scoped significance tests on per-game samples, not on aggregated numbers.** A metric like "average pegging points per game" is sampled per game; Welch runs on the vector `[game_1_value, game_2_value, …]` from each cohort. This is what makes n = number-of-games and gives BH the denominator the roadmap's 10K figure assumes.

## Open Questions

### Resolved During Planning

- **Compare subject?** → Two log dirs (user selection).
- **MC correction?** → BH default, Bonferroni optional (user selection).
- **Bradley–Terry scope?** → Extends matchup only (planner decision).
- **Restricted-play scope?** → ShipWreck-only via per-event-card toggles (user selection).
- **Critique deltas included?** → Yes (user selection).
- **External stats crate?** → No, in-house primitives (Key Technical Decision).
- **t-distribution CDF approximation?** → Normal approx at n ≥ 30 (Key Technical Decision).
- **Per-card toggle interaction with Phase 5's `events_enabled`?** → `events_enabled: false` ANDs down all three per-card toggles; `events_enabled: true` lets each per-card toggle make the final decision per-card.

### Deferred to Implementation

- **Exact Welch-to-Z threshold**: the plan commits to "Normal approx at n ≥ 30," but whether to always use Normal vs. fall back to t-distribution below some n is an Unit 1 decision once the reference-value tests are in place.
- **BT convergence tolerance**: typical MM iterations converge in 10–20 steps; the exact tolerance + max-iterations pair is Unit 6's concern.
- **Per-card toggle default ordering in `ShipWreckConfig::new`**: whether to put the new fields alongside `num_players` / `events_enabled` or in a nested `EventsConfig` struct depends on how the serde default attributes shake out. Unit 7's decision.
- **"What changed" report's sort key**: sort by corrected p-value, by absolute delta size, or by absolute-delta-scaled-by-pooled-stddev (effect size)? The plan commits to "significance-first, then effect size within significance buckets" — the exact tie-break is Unit 4's concern.
- **How to render "only one side has critique data"**: ship the sane default (note in the Subjective deltas section + skip the t-tests); refine wording in Unit 4.

## High-Level Technical Design

> *This illustrates the intended data flow and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
  PRIOR SETUP (user side)                       COMPARE SUBCOMMAND (Phase 6)

  playtest play --out baseline-dir   ┐          playtest compare
    (plus --critique, --games 10000) │            --baseline baseline-dir
                                     │            --variant  variant-dir
  playtest play --out variant-dir    ┘            --out changes.md
    (plus --critique, --games 10000)              [--correction bh|bonferroni]
     + config delta (e.g.                         [--alpha 0.05]
     --shipwreck-disable-event typhoon)           [--game shipwreck]

                                                   │
                                                   ▼
                                               ingest_directory(baseline-dir)
                                                   into Connection::open_in_memory() #1
                                               ingest_directory(variant-dir)
                                                   into Connection::open_in_memory() #2
                                                   │
                                                   ▼
                                               enumerate_comparable_metrics(conn_a, conn_b)
                                                   │                      │
                                                   ▼                      ▼
                                               for each metric:        for each agent:
                                                 fetch samples_a         fetch wins_a/N_a
                                                 fetch samples_b         fetch wins_b/N_b
                                                 welch_t_test            two_proportion_z
                                                                      │
                                               same for critique_likert (Welch per question)
                                               same for critique_tags  (two-proportion per tag)
                                                   │
                                                   ▼
                                               collect all p-values → apply BH / Bonferroni
                                                   │
                                                   ▼
                                               CompareResult { flagged, rejected, unchanged }
                                                   │
                                                   ▼
                                               write_compare_report → changes.md
```

"What changed" report layout:

```
## Compare: variant-dir vs baseline-dir

- Games: baseline=10000, variant=10000
- Correction: BH @ α=0.05
- Flagged findings: 7  |  Rejected (noise): 0  |  Unchanged: 42

### Flagged regressions
| metric | baseline | variant | delta | p (raw) | p (BH) |
| ...    |          |         |       |         |        |

### Flagged improvements
| ... |

### Per-agent win-rate deltas
| agent | baseline wr | variant wr | delta | p |

### Subjective deltas (Likert + coded tags)
| question | baseline mean | variant mean | delta | p |
| tag      | baseline freq | variant freq | delta | p |

### Unchanged (folded summary)
- 42 metrics within noise. Top-5 nearest-to-significant listed below.
```

## Implementation Units

- [ ] **Unit 1: Statistical primitives — Welch's t-test, two-proportion z-test, BH, Bonferroni**

**Goal:** An in-house stats module with four tested primitives: `welch_t_test`, `two_proportion_z_test`, `benjamini_hochberg`, `bonferroni`. Each is a pure function with reference-tested output.

**Requirements:** R6.2, R6.3, R6.4

**Dependencies:** None.

**Files:**
- Create: `crates/playtest-metrics/src/stats.rs`
- Modify: `crates/playtest-metrics/src/lib.rs` (expose module)
- Test: `crates/playtest-metrics/src/stats.rs` (in-module unit tests)

**Approach:**
- `TestOutcome { mean_a, mean_b, delta, std_err, z_or_t, p_value, ci_95_low, ci_95_high, n_a, n_b }` — uniform return shape for both Welch and z-test.
- Welch: t = (x̄_a − x̄_b) / sqrt(s²_a/n_a + s²_b/n_b); at n ≥ 30 per side use `erf`-based Normal CDF for the p-value (two-sided). Implement `standard_normal_cdf` via the rational approximation in Abramowitz & Stegun 26.2.17 (Φ(x) = 1 − φ(x)·(a₁k + a₂k² + a₃k³ + a₄k⁴ + a₅k⁵), max error 7.5e-8).
- Two-proportion z: pooled standard error under the null; Normal CDF from the same helper.
- `benjamini_hochberg(p_values, alpha) -> Vec<bool>` — returns "is significant" aligned with input order. Sort ascending, find largest k with `p_{(k)} ≤ k·α/m`, reject all ranks ≤ k.
- `bonferroni(p_values, alpha)` — `p_i < α/m` per test. Simple.

**Execution note:** Start test-first. Every primitive has a known reference value (t-statistic from a 2-group hand-computed example, BH example from the 1995 paper's Table 1). Write the reference tests before the implementation.

**Patterns to follow:**
- `crates/playtest-metrics/src/query.rs` — in-module organization of small pure functions.
- `crates/playtest-agents/src/llm/critique/coder.rs::parse_coder_reply` — returns-a-result-struct pattern.

**Test scenarios:**
- Happy path: Welch t-test on two 5-element vectors matches a reference Python `scipy.stats.ttest_ind(equal_var=False)` output to 3 sig figs.
- Happy path: two-proportion z on (wins=60, n=100) vs (wins=80, n=100) matches reference.
- Happy path: BH on the p-value vector `[0.001, 0.008, 0.039, 0.041, 0.042, 0.06, 0.074, 0.205]` at α=0.05 rejects the first 4 (matches BH 1995 Table 1).
- Happy path: Bonferroni on the same vector at α=0.05 rejects only the first (`p < 0.05/8 = 0.00625`).
- Edge case: Welch on identical vectors returns p≈1 and delta=0.
- Edge case: Welch with n<5 on one side returns `None` in CI fields (too-small signal — reporter renders "—").
- Edge case: two-proportion z with n=0 on either side returns an error, not a NaN.
- Edge case: BH on an empty vector returns an empty vector.
- Edge case: BH where no p-value passes the smallest threshold rejects zero.
- Error path: welch with both vectors empty returns an error.

**Verification:**
- Reference values match scipy / 1995-paper outputs to 3 sig figs or better.
- No external crate added for stats.

---

- [ ] **Unit 2: Cross-DB compare primitives — enumerate, fetch, pair**

**Goal:** Query helpers that take two independent `Connection` handles and produce the paired sample vectors Unit 3 feeds into the stats primitives.

**Requirements:** R6.1, R6.2, R6.3

**Dependencies:** Unit 1 (for `TestOutcome` struct reuse).

**Files:**
- Create: `crates/playtest-metrics/src/compare.rs`
- Modify: `crates/playtest-metrics/src/lib.rs` (expose module + re-exports)
- Test: `crates/playtest-metrics/tests/compare_primitives.rs`

**Approach:**
- `MetricKey { name: String, player: Option<u8>, tag: Option<String> }` — normalized metric identifier used for pairing.
- `enumerate_paired_metrics(&Connection, &Connection) -> Vec<MetricKey>` — returns metrics present in both DBs. Metrics present in only one side are returned in a separate `only_baseline` / `only_variant` pair (so the report can surface "new metric appeared").
- `fetch_numeric_samples(&Connection, &MetricKey) -> Vec<f64>` — pulls `value_numeric` per-game across all games in the DB. One row per game per metric; sample size = game count.
- `fetch_agent_outcomes(&Connection) -> Vec<(agent_name, wins, games)>` — already exists in `query::agent_summaries`; reuse.
- `fetch_likert_samples(&Connection, &str) -> Vec<f64>` — per-question Likert samples across all critiqued games.
- `fetch_tag_counts(&Connection) -> (total_games, Vec<(tag, count)>)` — already covered by `critique_tag_counts_overall`; extend to return total critique-response count for z-test denominator.

**Patterns to follow:**
- `crates/playtest-metrics/src/query.rs` — prepared-statement + row-mapping pattern.
- Compare's queries are single-DB per call (the caller threads two DBs); don't invent a cross-DB join.

**Test scenarios:**
- Happy path: two in-memory DBs with identical metric rows → `enumerate_paired_metrics` returns every metric; `only_*` vectors empty.
- Happy path: baseline has metric X, variant lacks it → `only_baseline = [X]`; `paired` omits X.
- Happy path: `fetch_numeric_samples` on a DB with 100 games of metric M returns a 100-element vector in deterministic order (ORDER BY game_id).
- Happy path: `fetch_likert_samples(conn, "agency")` returns one value per (game, seat) critique row.
- Edge case: DB with no critique data → `fetch_likert_samples` returns an empty vector; caller handles as "no subjective signal."
- Edge case: metric row with `value_text` (tag-kind metric, no numeric) is skipped by `fetch_numeric_samples`.
- Integration: two ingested log dirs (via `ingest_directory`) feed cleanly into `enumerate_paired_metrics` — confirms the primitives compose with the real ingest, not just with synthesized SQL.

**Verification:**
- Every paired metric has deterministic ordering across runs.
- No cross-DB SQL statements (each query binds to one `Connection`).

---

- [ ] **Unit 3: Compare engine — orchestrates ingest, tests, correction**

**Goal:** A single `run_compare(baseline_dir, variant_dir, opts) -> CompareResult` function that ties Units 1 and 2 together and returns a structured, report-ready result.

**Requirements:** R6.1, R6.2, R6.3, R6.4

**Dependencies:** Units 1 and 2.

**Files:**
- Create: `crates/playtest-metrics/src/compare/engine.rs`
- Modify: `crates/playtest-metrics/src/compare.rs` (declare submodule + re-exports)
- Modify: `crates/playtest-metrics/src/lib.rs` (re-export `run_compare`, `CompareResult`, `Correction` enum)
- Test: `crates/playtest-metrics/tests/compare_engine.rs`

**Approach:**
- `CompareOpts { alpha: f64, correction: Correction, game_name: String }`. `Correction::BenjaminiHochberg` / `Correction::Bonferroni`.
- `CompareResult { flagged: Vec<Finding>, rejected: Vec<Finding>, unchanged: Vec<Finding>, only_baseline: Vec<MetricKey>, only_variant: Vec<MetricKey>, note_no_critique_on_one_side: Option<Side> }`.
- `Finding { kind: FindingKind, name, mean_a, mean_b, delta, p_raw, p_corrected, significant: bool }`. `FindingKind::NumericMetric | AgentWinRate | LikertQuestion | CodedTag`.
- Orchestration: create two `Connection::open_in_memory()`; `ingest_directory` both; run each family of queries; run the test per family; collect p-values into a single vector; apply correction once across the full p-value set (the standard practice for cross-family FDR control).
- Sort: flagged first by absolute delta magnitude descending within sign buckets (regressions vs improvements), rejected by raw p ascending, unchanged by raw p descending.

**Patterns to follow:**
- `crates/playtest-metrics/src/reporter.rs::write_summary_section` — structured `Result`-returning function that builds a pure value (no side effects except SQLite reads).
- `crates/playtest-cli/src/commands/report.rs` — existing two-phase (ingest → render) shape.

**Test scenarios:**
- Happy path: two log dirs with identical events produce zero flagged findings.
- Happy path: a deliberately larger `event_count` in the variant cohort produces a flagged numeric finding whose delta and sign are correct.
- Happy path: agent win rate shifts from 50/50 in baseline to 60/40 in variant with n=10,000 is flagged at α=0.05 under BH.
- Happy path: Likert "agency" mean shifts from 4.0 to 3.0 across 10,000 critiqued games lands in `flagged` under BH.
- Edge case: variant DB has a new metric absent from baseline → lands in `only_variant`, not in `flagged` (delta is undefined, not zero).
- Edge case: baseline has critique data, variant doesn't → `note_no_critique_on_one_side = Some(Variant)`; no Likert or tag Findings in the result.
- Edge case: both DBs empty → `CompareResult::default()` with zero findings and no panic.
- Edge case: correction switches between BH and Bonferroni change the set of `flagged` findings deterministically.
- Integration: ingest two tempdir-sourced log dirs end-to-end through `run_compare`.

**Verification:**
- Running compare twice on the same pair returns identical findings byte-for-byte.
- Switching correction changes only the `flagged` / `rejected` split, not the total finding count.

---

- [ ] **Unit 4: "What changed" markdown reporter**

**Goal:** Convert a `CompareResult` into the markdown report sketched in the High-Level Technical Design section.

**Requirements:** R6.1, R6.2, R6.3, R6.4

**Dependencies:** Unit 3.

**Files:**
- Create: `crates/playtest-metrics/src/compare/report.rs`
- Modify: `crates/playtest-metrics/src/compare.rs` (declare submodule + re-exports)
- Modify: `crates/playtest-metrics/src/lib.rs` (re-export `write_compare_report`)
- Test: `crates/playtest-metrics/tests/compare_report.rs`

**Approach:**
- `write_compare_report(md: &mut MarkdownBuilder, result: &CompareResult, opts_summary: &OptsSummary)` — mirrors the `write_subjective_critique_section` signature from Phase 5 Unit 7.
- Sections in order: (1) header with sample sizes + correction + alpha + total counts; (2) Flagged regressions table (sort: effect size desc); (3) Flagged improvements table; (4) Per-agent win-rate deltas table; (5) Subjective deltas tables (Likert + coded tags), each with a note if one side lacks data; (6) Rejected (noise) folded summary (top-5 nearest-to-significant); (7) Only-in-one-side metrics as a short bullet list.
- Empty sections omit cleanly (no empty headings).
- "Games" line shows per-cohort game counts; if the two cohorts have very different Ns (ratio > 2×), emit a warning banner like the Phase 5 spec-version-mixed banner.

**Patterns to follow:**
- `crates/playtest-metrics/src/reporter.rs::write_subjective_critique_section` — Phase 5's gated-section pattern.
- `crates/playtest-metrics/src/markdown.rs::MarkdownBuilder` — existing table/heading helpers.

**Test scenarios:**
- Happy path: a CompareResult with 3 flagged findings produces a report with `## Compare:` heading, the right table row counts, and the three metrics in descending-delta order.
- Happy path: a CompareResult with zero findings renders a report with a "No statistically significant changes" paragraph (not an empty "Flagged" heading).
- Happy path: a CompareResult with `only_variant` non-empty renders a "Metrics only in variant" bullet list.
- Happy path: Subjective Likert delta on "agency" renders with mean-mean-delta-p columns.
- Edge case: `note_no_critique_on_one_side = Some(Baseline)` renders a note line in the Subjective section rather than a table.
- Edge case: baseline games ÷ variant games ≥ 2 emits a sample-size-imbalance warning.
- Edge case: report rendering is deterministic byte-for-byte across repeated calls on the same input.
- Edge case: tables with very long tag names or metric names render without breaking alignment (soft — verify via substring presence).

**Verification:**
- Markdown output is deterministic and stable byte-order.
- Existing reporter sections are not altered.

---

- [ ] **Unit 5: `playtest compare` CLI subcommand**

**Goal:** The user-facing CLI. Threads `--baseline`, `--variant`, `--out`, `--game`, `--alpha`, `--correction` into `run_compare` and `write_compare_report`.

**Requirements:** R6.1, R6.4

**Dependencies:** Units 3 and 4.

**Files:**
- Create: `crates/playtest-cli/src/commands/compare.rs`
- Modify: `crates/playtest-cli/src/commands/mod.rs`
- Modify: `crates/playtest-cli/src/main.rs` (register subcommand)
- Test: `crates/playtest-cli/tests/e2e_compare_stubbed.rs`

**Approach:**
- `CompareArgs { baseline: PathBuf, variant: PathBuf, out: PathBuf, game: String, alpha: f64 (default 0.05), correction: Correction (default BH) }`.
- `run(&CompareArgs)`: resolves `RegisteredGame` via `lookup_game`, runs `run_compare`, pipes through `write_compare_report`, writes to `--out`.
- Error cases: missing baseline / variant dir → clap-level `exists_in_filesystem` validator; no .jsonl files in a dir → bail with pointer to `playtest play --out <dir>`.

**Patterns to follow:**
- `crates/playtest-cli/src/commands/report.rs` — subcommand shape for "ingest two dirs and write markdown."
- `crates/playtest-cli/src/commands/critique_code.rs` — Phase 5's subcommand registration pattern.

**Test scenarios:**
- Happy path: two tempdir-sourced log dirs (produced by `playtest play` via the stubbed test harness) feed `playtest compare` and produce a valid markdown file with the expected section headings.
- Happy path: running with `--correction bonferroni` produces a report whose flagged count ≤ the BH flagged count on the same input.
- Happy path: `--alpha 0.01` produces a report with strictly ≤ flagged count compared to `--alpha 0.05`.
- Edge case: missing baseline dir → non-zero exit with clap-level "does not exist" message.
- Edge case: baseline dir exists but contains no `.jsonl` files → friendly "no games ingested" message in the report + exit 0.
- Edge case: baseline game != --game argument → report contains an ingestion-error row per mismatched game.
- Integration: end-to-end `cargo test --release -p playtest-cli --test e2e_compare_stubbed` covers the full stubbed path.

**Verification:**
- `--help` lists the new subcommand and all four flags.
- `playtest compare --baseline a/ --variant b/ --out c.md --game cribbage` on a matching-pair produces `c.md`.

---

- [ ] **Unit 6: Bradley–Terry ratings + `playtest matchup --bradley-terry`**

**Goal:** MLE Bradley–Terry strengths via the MM algorithm (Hunter 2004), exposed as a new `playtest matchup` flag. The existing matchup output is unchanged when the flag is absent.

**Requirements:** R6.5

**Dependencies:** None (Unit 1's primitives aren't needed — MM iteration is its own algorithm).

**Files:**
- Create: `crates/playtest-metrics/src/stats/bradley_terry.rs` (module inside the existing `stats` file — promote to directory form if the file grows)
- Modify: `crates/playtest-metrics/src/lib.rs` (expose `bradley_terry_mle`)
- Modify: `crates/playtest-cli/src/commands/matchup.rs` (new `--bradley-terry` flag + rendering)
- Test: `crates/playtest-metrics/src/stats/bradley_terry.rs` (in-module unit tests)
- Test: `crates/playtest-cli/tests/matchup_bradley_terry.rs`

**Approach:**
- `MatchupTally { agent_names: Vec<String>, wins: Matrix<u64> }` where `wins[i][j]` is the count of games agent i beat agent j. Matchup already has this under the hood.
- `bradley_terry_mle(tally, tol, max_iter) -> Vec<(name, theta, log_theta_ci_95_low, log_theta_ci_95_high)>`. Hunter 2004's MM update: `θ_i ← W_i / Σ_{j≠i} N_{ij} / (θ_i + θ_j)`. Normalize (e.g. θ_0 = 1) for identifiability. Converges in ~10–20 iterations at `tol = 1e-6`.
- Approximate CI via Fisher information at the MLE, or a bootstrap over the tally matrix. Pick bootstrap (simpler, distribution-free); 200 resamples is enough at the precision we care about.
- `playtest matchup --bradley-terry` renders a new section below the win-rate matrix: `| agent | θ̂ | 95% CI (log-θ) |` sorted by θ̂ descending.

**Patterns to follow:**
- `crates/playtest-cli/src/commands/matchup.rs` — additive flag + section pattern.

**Test scenarios:**
- Happy path: 3 agents with symmetric wins (each beats each other 50/50 across equal games) produce `θ̂` ratios of 1:1:1 (within tolerance).
- Happy path: a strict dominance chain (A beats B, B beats C, A beats C) produces monotonically decreasing θ̂.
- Happy path: 4-agent tally with known MLE from a textbook example matches to 3 sig figs.
- Edge case: convergence tolerance reached before `max_iter` — algorithm halts early and returns.
- Edge case: one agent that never played any match returns θ̂ = NaN (or is omitted with a warning).
- Edge case: all agents tied at 0 wins → θ̂ all equal.
- Integration: `playtest matchup --agents random,heuristic-cribbage,ismcts-cribbage --game cribbage --bradley-terry` on a stubbed short run produces both the matrix and the BT table in the rendered output.

**Verification:**
- Rendering when `--bradley-terry` is absent is byte-identical to pre-Unit-6 matchup output.
- BT table sorts deterministically (θ̂ desc, tie-break agent name asc).

---

- [ ] **Unit 7: Per-event-card ShipWreck toggles + `--shipwreck-disable-event` CLI flag**

**Goal:** Make restricted-play runnable without code edits. ShipWreckConfig gains three per-card bool fields; `playtest play --shipwreck-disable-event <card>` flips them.

**Requirements:** R6.6, R6.7

**Dependencies:** None.

**Files:**
- Modify: `crates/games/shipwreck/src/config.rs`
- Modify: `crates/games/shipwreck/src/setup.rs` (retention filter reads per-card flags)
- Modify: `crates/playtest-cli/src/commands/play.rs` (new repeatable flag)
- Modify: `crates/playtest-registry/src/play.rs` (thread flag value into `ShipWreckConfig::new(n)` call site)
- Test: `crates/games/shipwreck/tests/event_card_toggles.rs`
- Test: `crates/playtest-cli/tests/e2e_disable_event_flag.rs`

**Approach:**
- `ShipWreckConfig { num_players, events_enabled, shark_enabled, typhoon_enabled, flying_fish_enabled }`. Each per-card bool uses `#[serde(default = "return_true")]` for back-compat with Phase 5 configs.
- `events_enabled: false` forces all three per-card fields to effectively disabled at setup time (AND logic). `events_enabled: true` lets each per-card flag make the call.
- Setup filter: `wreckage.retain(|c| keep_event_card(c, &cfg))` where `keep_event_card` matches on `Card::Event(…)` and consults the relevant flag.
- CLI: `--shipwreck-disable-event <card>` (repeatable, values = `shark` | `typhoon` | `flying_fish`). `playtest play` constructs the config with the right fields flipped.

**Patterns to follow:**
- Phase 5 Unit 8's `events_enabled` shape (`crates/games/shipwreck/src/config.rs`).
- `crates/playtest-cli/src/commands/play.rs::PlayArgs` — clap flag declaration pattern (see `stdio_args: Vec<String>` for repeatable flag).

**Test scenarios:**
- Happy path: config with `shark_enabled: false` produces a setup state with zero Shark cards, non-zero Typhoon and FlyingFish.
- Happy path: config with all three per-card flags false but `events_enabled: true` is identical in behavior to `events_enabled: false`.
- Happy path: config with `events_enabled: false` ignores per-card flags (AND logic) — zero event cards anywhere.
- Happy path: serde round-trip preserves all fields; legacy `{"num_players":2}` deserializes with all toggles = true.
- Happy path: `config_hash` differs between each per-card-toggle combination (so SQLite bucketing separates cohorts).
- Edge case: CLI with `--shipwreck-disable-event unknown` → clap-level value validation error.
- Integration: `playtest play --game shipwreck --agents random,random --shipwreck-disable-event typhoon --games 2 --out <tmp>` completes and writes logs whose config_hash reflects the disabled flag.

**Verification:**
- Existing Phase 5 `events_enabled` behavior is preserved verbatim.
- Repeatable flag accumulates across invocations.

---

- [ ] **Unit 8: R6.8 + R6.9 exit-criterion fixture + BENCHMARKS recipe**

**Goal:** Stubbed-in-CI proof that the pipeline detects a real regression and does not false-alarm on a cosmetic change, plus a documented real-LLM recipe for the exit-criterion cohort.

**Requirements:** R6.8, R6.9

**Dependencies:** Units 3, 5, and 7.

**Files:**
- Create: `crates/playtest-cli/tests/r6_exit_criteria_stubbed.rs`
- Modify: `docs/BENCHMARKS.md` (add R6.8 and R6.9 sections)

**Approach:**
- **R6.8 stubbed**: synthesize two SQLite DBs via `ingest_directory` against two tempdir-sourced log dirs — one "baseline" with `shark_enabled: false`, one "buffed" where we manually append per-game metric rows that reflect the effect of a shark buff (e.g. doubled food loss). Run `run_compare`. Assert at least one flagged finding and its delta sign aligns with the buff direction. No real LLMs, no 10K games — the stub proves the pipeline wires correctly, and the real-run recipe in BENCHMARKS.md validates the statistical behavior on real data.
- **R6.9 stubbed**: synthesize two identical DBs (byte-identical ingest). Run `run_compare`. Assert `flagged` is empty at both BH and Bonferroni correction.
- **BENCHMARKS.md R6.8 recipe**: four `playtest play` invocations (all-events baseline, shark-only buffed via a small "manual buff" hook or acceptance of the shark-disabled baseline as contrast), `playtest compare` call, expected flagged count, expected sign of the delta. Cost estimate if run with LLM agents; $0 if run with heuristic agents (preferred for the 10K-game cohort).
- **BENCHMARKS.md R6.9 recipe**: same two log dirs produced by cosmetic change (e.g. edit a rules_for_llm.md comment, leaving card mechanics unchanged). Expected: zero flagged findings.

**Patterns to follow:**
- `docs/BENCHMARKS.md` R3.8/R3.9/R5.9 — manual-recipe template + automated-stub-in-CI discipline.
- `crates/games/shipwreck/tests/events_enabled.rs` — Phase 5's exit-criterion fixture shape.

**Test scenarios:**
- Happy path (stubbed): a synthetic "buffed" DB produces ≥ 1 flagged finding; the finding's delta sign is consistent with the buff.
- Happy path (stubbed): two byte-identical DBs produce zero flagged findings under both BH and Bonferroni (R6.9).
- Happy path (stubbed): changing only `agents: []` in the log header between two otherwise-identical DBs (the cosmetic-like test) produces zero flagged mechanical findings.
- Edge case: compare with small N (say 20 games per cohort) still runs without panic; flagged count depends on effect size.

**Verification:**
- `cargo test --release --test r6_exit_criteria_stubbed` passes in CI.
- `docs/BENCHMARKS.md` R6.8 + R6.9 sections list concrete CLI incantations for the manual run.

## System-Wide Impact

- **Interaction graph:** `playtest compare` is a new subcommand; `playtest matchup --bradley-terry` is a new section gated by a flag; `playtest play --shipwreck-disable-event` is a new flag on an existing subcommand. No existing behavior changes when the new flags are absent.
- **Error propagation:** Compare failures surface as non-zero CLI exits; per-file ingest errors flow through the existing `IngestReport::errors` path. BT failures (non-convergence) emit a warning and skip the BT section — matchup's win-rate matrix still renders.
- **State lifecycle risks:** Compare uses in-memory SQLite DBs that die with the subcommand — no disk artifact leaks. The config-hash mechanism already partitions per-config cohorts, so the new per-card toggles don't require ingest-level schema changes.
- **API surface parity:** Programmatic API — `run_compare` + `CompareResult` + `write_compare_report` — is usable by any future UI (Phase 8) without CLI scraping. `bradley_terry_mle` is usable outside matchup (e.g., future tournament tooling).
- **Integration coverage:** Stubbed e2e for compare (Unit 5), for matchup BT (Unit 6), for per-card toggles (Unit 7), and for exit criteria (Unit 8). Real-LLM cohort is the manual BENCHMARKS recipe.
- **Unchanged invariants:**
  - Main JSONL log invariant: compare is a pure reader.
  - `Game` trait shape: no new methods added. Per-card toggles live on `ShipWreckConfig`, not on the trait.
  - `MetricRegistry<G>` unchanged.
  - Phase 5's critique ingest unchanged — compare reads from the existing tables.
  - Determinism audit unchanged — no new event kinds, no new sink writes.

## Alternative Approaches Considered

- **External stats crate (`statrs`, `ndarray-stats`)** — rejected. Phase 6's stats surface is ~300 lines of pure Rust with zero transitive deps; `statrs` brings in `nalgebra` and a long graph. In-house is cheaper across the build and easier to pin against reference values.
- **Inline runner on `playtest compare`** — rejected per user selection. Two-log-dir input is smaller, simpler, and composes with the existing `playtest play` surface. The inline runner becomes a separate plan if the UX friction shows up in practice.
- **Bradley–Terry in compare** — rejected. Two subjects per compare pair make BT degenerate (trivial 2-way MLE = win rate). Matchup's N-way tally is where BT earns its keep.
- **Per-game Game-trait `restricted_cards()` hook** — rejected per user selection. One game's worth of signal isn't enough to justify a trait change; ShipWreck-local machinery is right-sized.
- **Welch via exact t-distribution CDF** — deferred. Normal approximation at n ≥ 30 per cohort has error well below the significance threshold at R6.8's 10K-game scale. Add exact CDF only if later phases want tight control at smaller N.

## Risks & Dependencies

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| BH produces too few / too many flagged findings in real-LLM cohorts | Med | Med | Unit 1's reference tests guard the arithmetic. R6.8's manual recipe validates calibration on real data; if it misfires, swap in an exact t-distribution CDF and/or tune α default before shipping. |
| BT MM algorithm fails to converge on sparse matchup tallies | Low | Low | Unit 6's bootstrap CI is robust to modest numerical slop; `max_iter = 200` with `tol = 1e-6` handles typical cases. Emit a warning on non-convergence and skip the BT section. |
| Per-card toggle serde default drifts between Phase 5 and Phase 6 configs | Low | Med | Unit 7's legacy-config test (`{"num_players":2}` → all toggles true) locks the back-compat behavior. |
| Compare report is dense / unreadable with 50+ metrics | Med | Low | Unit 4's "fold unchanged into top-5-nearest" design keeps the report length bounded. Sort by effect size within significance buckets. |
| Sample-size imbalance (baseline 10k, variant 1k) masks real deltas | Med | Med | Unit 4 emits a warning banner at ≥ 2× size ratio so operators notice before drawing conclusions. |
| Restricted-play recipe is tedious (4 runs + 3 compares) for any ShipWreck study | Med | Low | Accept in Phase 6 — if usage shows the friction is real, add a `playtest restricted-play` wrapper in a follow-up. |

## Documentation / Operational Notes

- `docs/BENCHMARKS.md` gains R6.8 and R6.9 sections following the R5.9 template (automated stubbed test + manual real-LLM recipe).
- `CLAUDE.md` does not need an architectural-invariant update — compare adds no new discipline, just a new read path.
- `docs/api-contract.md` is not touched — compare is CLI-only.

## Sources & References

- **Roadmap:** `playtest-roadmap.md` § "Phase 6 — Comparative and counterfactual analysis"
- **Prior plan (Phase 5):** `docs/plans/2026-04-23-001-feat-phase-5-post-game-critique-plan.md` (shipped) — the critique-diff surface compare consumes
- **Architecture discipline:** `docs/solutions/architecture-patterns/ephemeral-coordination-frame-vs-logged-event-2026-04-22.md`
- **Project invariants:** `CLAUDE.md`
- **External stats references:**
  - Benjamini & Hochberg 1995, "Controlling the False Discovery Rate" — BH procedure + Table 1 reference values
  - Hunter 2004, "MM algorithms for generalized Bradley–Terry models" — MM iteration
  - Jaffe et al. 2012, "Evaluating Competitive Game Balance with Restricted Play" — card-contribution methodology
  - Abramowitz & Stegun 26.2.17 — rational approximation for the standard-normal CDF
