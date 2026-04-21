//! Cribbage-specific report sections.
//!
//! The Unit 15 reporter stitches these together with the generic
//! sections in `playtest_metrics::reporter`. All three sections read
//! from the SQLite rows that `ingest_directory` wrote via Cribbage's
//! `MetricRegistry` impl — the report never re-ingests or re-parses
//! event logs.
//!
//! The per-card (R1.5) section is the load-bearing one. It joins
//! numerator and denominator metrics (e.g., `card_kept_count` /
//! `card_dealt_count`) to produce a per-rank kept-rate table, which
//! is the explicit "design insight surface" the Phase 1 exit criteria
//! call for.

use playtest_metrics::{MarkdownBuilder, avg_numeric_metric, sum_count_metric};
use rusqlite::{Connection, Error as SqliteError};

use crate::card::Rank;
use crate::metrics::{game_shape, per_card, scoring as scoring_metrics};

/// Write the **Cribbage: game shape** section.
///
/// # Errors
/// Propagates any `rusqlite` error from the underlying queries.
pub fn write_game_shape_section(
    md: &mut MarkdownBuilder,
    conn: &Connection,
) -> Result<(), SqliteError> {
    md.h2("Cribbage: game shape");

    // Phase-of-game-end breakdown is a tag-valued metric — group by value.
    let phases = phase_of_game_end_breakdown(conn)?;
    if phases.is_empty() {
        md.paragraph("*No cribbage games ingested.*");
        return Ok(());
    }

    md.h3("Phase of game end");
    let total_phases: u64 = phases.iter().map(|(_, n)| *n).sum();
    let mut rows = Vec::with_capacity(phases.len());
    for (phase, n) in &phases {
        rows.push(vec![
            phase.clone(),
            n.to_string(),
            format_share(*n, total_phases),
        ]);
    }
    md.table(&["phase", "games", "share"], &rows);

    if let Some(avg_leads) = avg_numeric_metric(conn, game_shape::LEAD_CHANGES, None, None)? {
        md.bullet(&format!(
            "Average lead changes per game: **{avg_leads:.2}**"
        ));
    }
    if let Some(avg_margin) = avg_numeric_metric(conn, game_shape::FINAL_SCORE_MARGIN, None, None)?
    {
        md.bullet(&format!("Average final-score margin: **{avg_margin:.1}**"));
    }
    if let Some(avg_nibs_hands) = avg_nibs_hands_per_game(conn)? {
        md.bullet(&format!(
            "Average nibs hands per game (starter = jack): **{avg_nibs_hands:.3}**"
        ));
    }
    if let Some(dealer_rate) = dealer_win_rate(conn)? {
        md.bullet(&format!("Dealer win rate: **{:.1}%**", dealer_rate * 100.0));
    }
    md.end_block();

    Ok(())
}

/// Write the **Cribbage: scoring breakdown** section: per-player
/// average hand / crib / pegging points and pegging share.
///
/// # Errors
/// Propagates any `rusqlite` error.
pub fn write_scoring_breakdown_section(
    md: &mut MarkdownBuilder,
    conn: &Connection,
) -> Result<(), SqliteError> {
    md.h2("Cribbage: scoring breakdown");

    let headers = &[
        "player",
        "avg hand",
        "avg crib",
        "avg pegging",
        "avg pegging share",
        "avg nibs",
        "avg decisions",
    ];
    let mut rows = Vec::with_capacity(2);
    let mut any_data = false;
    for player in [0u8, 1u8] {
        let hand = avg_numeric_metric(conn, scoring_metrics::HAND_SCORE_TOTAL, Some(player), None)?;
        let crib = avg_numeric_metric(conn, scoring_metrics::CRIB_SCORE_TOTAL, Some(player), None)?;
        let peg = avg_numeric_metric(
            conn,
            scoring_metrics::PEGGING_SCORE_TOTAL,
            Some(player),
            None,
        )?;
        let share = avg_numeric_metric(
            conn,
            scoring_metrics::PEGGING_SHARE_OF_TOTAL,
            Some(player),
            None,
        )?;
        let nibs =
            avg_numeric_metric(conn, scoring_metrics::NIBS_CONTRIBUTION, Some(player), None)?;
        let decisions = avg_numeric_metric(
            conn,
            scoring_metrics::DECISIONS_PER_PLAYER,
            Some(player),
            None,
        )?;
        if [&hand, &crib, &peg, &share, &nibs, &decisions]
            .iter()
            .any(|v| v.is_some())
        {
            any_data = true;
        }
        rows.push(vec![
            format!("player_{player}"),
            fmt_opt(hand, 1),
            fmt_opt(crib, 1),
            fmt_opt(peg, 1),
            fmt_share_opt(share),
            fmt_opt(nibs, 2),
            fmt_opt(decisions, 1),
        ]);
    }
    if !any_data {
        md.paragraph("*No scoring data.*");
        return Ok(());
    }
    md.table(headers, &rows);
    Ok(())
}

/// Write the **Cribbage: per-card design insight** (R1.5) section.
///
/// One row per rank. The kept-rate column is the headline R1.5 signal:
/// with random agents the effective distribution is driven purely by
/// the *deal*, so we expect meaningful asymmetry across ranks (e.g.,
/// 5s kept more than 2s) even with no strategic play.
///
/// # Errors
/// Propagates any `rusqlite` error.
pub fn write_per_card_section(
    md: &mut MarkdownBuilder,
    conn: &Connection,
) -> Result<(), SqliteError> {
    md.h2("Cribbage: per-card design insight (R1.5)");
    md.paragraph(
        "Cross-game rates per rank. `kept` = fraction of deals where a card of that rank was kept in-hand. \
         `→ own crib` / `→ opp crib` = fraction of deals where a held card was sent to the dealer's / opponent's crib. \
         `win@hand` / `win@crib` = conditional win rates when the player held / owned-crib-contained the rank.",
    );

    let headers = &[
        "rank",
        "kept",
        "→ own crib",
        "→ opp crib",
        "win@hand",
        "win@crib",
    ];
    let mut rows = Vec::with_capacity(13);
    let mut any_data = false;
    for rank in Rank::ALL {
        let tag = rank.symbol().to_string();
        let tag_ref = Some(tag.as_str());
        // Aggregate per-rank across both players for the summary table.
        let mut dealt = 0i64;
        let mut kept = 0i64;
        let mut own_crib = 0i64;
        let mut opp_crib = 0i64;
        let mut held = 0i64;
        let mut crib_any = 0i64;
        let mut win_hand = 0i64;
        let mut win_crib = 0i64;
        for player in [0u8, 1u8] {
            dealt += sum_count_metric(conn, per_card::CARD_DEALT_COUNT, Some(player), tag_ref)?;
            kept += sum_count_metric(conn, per_card::CARD_KEPT_COUNT, Some(player), tag_ref)?;
            own_crib += sum_count_metric(
                conn,
                per_card::CARD_DISCARDED_TO_OWN_CRIB_COUNT,
                Some(player),
                tag_ref,
            )?;
            opp_crib += sum_count_metric(
                conn,
                per_card::CARD_DISCARDED_TO_OPP_CRIB_COUNT,
                Some(player),
                tag_ref,
            )?;
            held += sum_count_metric(
                conn,
                per_card::HAND_CONTAINED_CARD_COUNT,
                Some(player),
                tag_ref,
            )?;
            crib_any += sum_count_metric(
                conn,
                per_card::CRIB_CONTAINED_CARD_COUNT,
                Some(player),
                tag_ref,
            )?;
            win_hand += sum_count_metric(
                conn,
                per_card::WIN_WHEN_CARD_IN_HAND_COUNT,
                Some(player),
                tag_ref,
            )?;
            win_crib += sum_count_metric(
                conn,
                per_card::WIN_WHEN_CARD_IN_CRIB_COUNT,
                Some(player),
                tag_ref,
            )?;
        }
        if dealt == 0 && kept == 0 && held == 0 {
            rows.push(vec![
                tag.clone(),
                "-".into(),
                "-".into(),
                "-".into(),
                "-".into(),
                "-".into(),
            ]);
            continue;
        }
        any_data = true;
        rows.push(vec![
            tag,
            fmt_rate(kept, dealt),
            fmt_rate(own_crib, dealt),
            fmt_rate(opp_crib, dealt),
            fmt_rate(win_hand, held),
            fmt_rate(win_crib, crib_any),
        ]);
    }
    if !any_data {
        md.paragraph("*No per-card data.*");
        return Ok(());
    }
    md.table(headers, &rows);
    Ok(())
}

/// Group `game_ended_in_phase` tag values across all ingested games.
fn phase_of_game_end_breakdown(conn: &Connection) -> Result<Vec<(String, u64)>, SqliteError> {
    let mut stmt = conn.prepare(
        "SELECT value_text, COUNT(*) FROM game_metrics \
         WHERE metric_name = ?1 AND player = -1 AND value_kind = 'tag' \
         GROUP BY value_text ORDER BY value_text",
    )?;
    let rows = stmt.query_map([game_shape::GAME_ENDED_IN_PHASE], |row| {
        let phase: String = row.get(0)?;
        let n: i64 = row.get(1)?;
        Ok((phase, u64::try_from(n).unwrap_or(0)))
    })?;
    rows.collect()
}

fn avg_nibs_hands_per_game(conn: &Connection) -> Result<Option<f64>, SqliteError> {
    // `cuts_producing_nibs` counts "hands in which the starter was a
    // jack" per game. Averaged across games that's directly the
    // expected number of nibs-eligible hands per game — not a rate
    // per hand (we don't track hand count yet).
    avg_numeric_metric(conn, game_shape::CUTS_PRODUCING_NIBS, None, None)
}

fn dealer_win_rate(conn: &Connection) -> Result<Option<f64>, SqliteError> {
    // `game_winner_was_dealer` is a Bool metric, stored as 0/1 in
    // value_numeric. AVG(value_numeric) gives the fraction of games
    // where the dealer won.
    avg_numeric_metric(conn, game_shape::GAME_WINNER_WAS_DEALER, None, None)
}

fn format_share(games: u64, total: u64) -> String {
    if total == 0 {
        "-".into()
    } else {
        #[allow(
            clippy::cast_precision_loss,
            reason = "Phase 1 counts fit in f64 exactly"
        )]
        let num = games as f64;
        #[allow(
            clippy::cast_precision_loss,
            reason = "Phase 1 counts fit in f64 exactly"
        )]
        let den = total as f64;
        format!("{:.1}%", (num / den) * 100.0)
    }
}

fn fmt_opt(v: Option<f64>, precision: usize) -> String {
    v.map_or_else(|| "-".into(), |x| format!("{x:.precision$}"))
}

fn fmt_share_opt(v: Option<f64>) -> String {
    v.map_or_else(|| "-".into(), |x| format!("{:.1}%", x * 100.0))
}

fn fmt_rate(num: i64, den: i64) -> String {
    if den <= 0 {
        "-".into()
    } else {
        #[allow(
            clippy::cast_precision_loss,
            reason = "Phase 1 counts fit in f64 exactly"
        )]
        let n = num as f64;
        #[allow(
            clippy::cast_precision_loss,
            reason = "Phase 1 counts fit in f64 exactly"
        )]
        let d = den as f64;
        format!("{:.1}%", (n / d) * 100.0)
    }
}
