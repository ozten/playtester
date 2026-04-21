//! Game-agnostic reporter sections.
//!
//! These sections consume whatever registries were ingested and don't
//! know anything about Cribbage. The Unit 15 CLI stitches them
//! together with game-specific sections (see the game crates' own
//! `report` modules) to produce the final markdown.
//!
//! Each function appends to a [`MarkdownBuilder`] and returns
//! `rusqlite::Result<()>` — a query failure propagates up to the CLI
//! as an actionable error rather than a half-rendered report.

use rusqlite::{Connection, Error as SqliteError};

use crate::markdown::MarkdownBuilder;
use crate::query::{
    EndReasonBreakdown, WinnerBreakdown, agent_summaries, avg_numeric_metric, end_reason_breakdown,
    games_count, winner_breakdown,
};

/// Write the top-level **Summary** section: total games, average
/// length, winner distribution, end-reason breakdown, and average
/// wall-clock time when the Unit 12b `wall_clock_ms` built-in is
/// present.
///
/// # Errors
/// Propagates any `rusqlite` error from the underlying canned queries.
pub fn write_summary_section(
    md: &mut MarkdownBuilder,
    conn: &Connection,
) -> Result<(), SqliteError> {
    md.h2("Summary");

    let total = games_count(conn)?;
    if total == 0 {
        md.paragraph("*No games ingested.*");
        return Ok(());
    }

    let avg_length = conn.query_row(
        "SELECT AVG(CAST(event_count AS REAL)) FROM games",
        [],
        |row| row.get::<_, Option<f64>>(0),
    )?;

    md.bullet(&format!("Total games: **{total}**"));
    if let Some(avg) = avg_length {
        md.bullet(&format!("Average event count per game: **{avg:.1}**"));
    }
    if let Some(avg_wall) = avg_numeric_metric(
        conn,
        crate::builtin::BuiltInMetrics::WALL_CLOCK_MS,
        None,
        None,
    )? {
        md.bullet(&format!("Average wall-clock time: **{avg_wall:.1} ms**"));
    }
    md.end_block();

    md.h3("Winner distribution");
    let winners = winner_breakdown(conn)?;
    render_winner_breakdown(md, total, &winners);

    md.h3("End-reason breakdown");
    let reasons = end_reason_breakdown(conn)?;
    render_end_reason_breakdown(md, total, &reasons);

    Ok(())
}

/// Write the **Per-agent** section: win rate, avg final score, games
/// played, average event count per game.
///
/// # Errors
/// Propagates any `rusqlite` error.
pub fn write_per_agent_section(
    md: &mut MarkdownBuilder,
    conn: &Connection,
) -> Result<(), SqliteError> {
    md.h2("Per-agent");

    let agents = agent_summaries(conn)?;
    if agents.is_empty() {
        md.paragraph("*No agent stats recorded.*");
        return Ok(());
    }

    let avg_length_by_agent = avg_event_count_per_agent(conn)?;

    let headers = &[
        "agent",
        "games",
        "wins",
        "win rate",
        "avg score",
        "avg length",
    ];
    let mut rows = Vec::with_capacity(agents.len());
    for a in &agents {
        let avg_len = avg_length_by_agent
            .iter()
            .find(|(name, _)| name == &a.agent_name)
            .map_or(0.0, |(_, v)| *v);
        rows.push(vec![
            a.agent_name.clone(),
            a.games_played.to_string(),
            a.wins.to_string(),
            format!("{:.1}%", a.win_rate() * 100.0),
            format!("{:.1}", a.avg_score),
            format!("{avg_len:.1}"),
        ]);
    }
    md.table(headers, &rows);

    // Ingestion invariant: total wins across all rows equals the
    // number of games (exactly one winner per game). Surfacing this
    // in the report helps spot ingestion bugs (missing Final records,
    // double-counted wins).
    let total_games: u64 = agent_total_games(conn)?;
    let total_wins: u64 = agents.iter().map(|a| a.wins).sum();
    md.line(&format!(
        "_Total wins recorded: {total_wins}; total games: {total_games} (these should be equal in a 2-player victory game)._"
    ));
    md.end_block();

    Ok(())
}

fn agent_total_games(conn: &Connection) -> Result<u64, SqliteError> {
    conn.query_row("SELECT COUNT(*) FROM games", [], |row| {
        let n: i64 = row.get(0)?;
        Ok(u64::try_from(n).unwrap_or(0))
    })
}

fn render_winner_breakdown(md: &mut MarkdownBuilder, total: u64, winners: &[WinnerBreakdown]) {
    if winners.is_empty() {
        md.paragraph("*No winner data.*");
        return;
    }
    let mut rows = Vec::with_capacity(winners.len());
    for w in winners {
        let label = match w.winner {
            Some(n) => format!("player_{n}"),
            None => "draw".into(),
        };
        rows.push(vec![
            label,
            w.games.to_string(),
            format_share(w.games, total),
        ]);
    }
    md.table(&["winner", "games", "share"], &rows);
}

fn render_end_reason_breakdown(
    md: &mut MarkdownBuilder,
    total: u64,
    reasons: &[EndReasonBreakdown],
) {
    if reasons.is_empty() {
        md.paragraph("*No end-reason data.*");
        return;
    }
    let mut rows = Vec::with_capacity(reasons.len());
    for r in reasons {
        rows.push(vec![
            r.end_reason.clone(),
            r.games.to_string(),
            format_share(r.games, total),
        ]);
    }
    md.table(&["end reason", "games", "share"], &rows);
}

/// Mean event count per agent, for the per-agent table's "avg length"
/// column. SQLite's AVG gives us the right answer even when the same
/// agent shows up in many games.
fn avg_event_count_per_agent(conn: &Connection) -> Result<Vec<(String, f64)>, SqliteError> {
    let mut stmt = conn.prepare(
        "SELECT a.agent_name, AVG(CAST(g.event_count AS REAL)) \
         FROM agent_stats a JOIN games g ON g.id = a.game_id \
         GROUP BY a.agent_name \
         ORDER BY a.agent_name",
    )?;
    let rows = stmt.query_map([], |row| {
        let name: String = row.get(0)?;
        let avg: Option<f64> = row.get(1)?;
        Ok((name, avg.unwrap_or(0.0)))
    })?;
    rows.collect()
}

/// Percentage-formatted share of `games` out of `total`. Returns a
/// dash when `total` is zero rather than NaN so the table stays
/// readable.
pub(crate) fn format_share(games: u64, total: u64) -> String {
    if total == 0 {
        "-".to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_share_handles_zero_total_without_nan() {
        assert_eq!(format_share(0, 0), "-");
    }

    #[test]
    fn format_share_renders_percent_one_decimal() {
        assert_eq!(format_share(1, 4), "25.0%");
        assert_eq!(format_share(1, 3), "33.3%");
    }

    #[test]
    fn summary_of_empty_db_reports_no_games() {
        let conn = Connection::open_in_memory().unwrap();
        crate::init_schema(&conn).unwrap();
        let mut md = MarkdownBuilder::new();
        write_summary_section(&mut md, &conn).unwrap();
        let out = md.into_string();
        assert!(out.contains("## Summary"));
        assert!(out.contains("No games ingested"));
    }

    #[test]
    fn per_agent_of_empty_db_reports_no_stats() {
        let conn = Connection::open_in_memory().unwrap();
        crate::init_schema(&conn).unwrap();
        let mut md = MarkdownBuilder::new();
        write_per_agent_section(&mut md, &conn).unwrap();
        let out = md.into_string();
        assert!(out.contains("## Per-agent"));
        assert!(out.contains("No agent stats recorded"));
    }

    /// Shape check: inserting a few rows by hand produces all the
    /// expected section headings and a sensible table.
    #[test]
    fn summary_and_per_agent_render_inserted_rows() {
        let conn = Connection::open_in_memory().unwrap();
        crate::init_schema(&conn).unwrap();

        conn.execute(
            "INSERT INTO games (id, game, version, seed, started_at, finished_at, winner, end_reason, config_hash, event_count, source_path) \
             VALUES ('g1', 'x', '0', 0, 0, 100, 0, 'victory', '0', 10, 'g1.jsonl')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO games (id, game, version, seed, started_at, finished_at, winner, end_reason, config_hash, event_count, source_path) \
             VALUES ('g2', 'x', '0', 1, 0, 200, 1, 'victory', '0', 20, 'g2.jsonl')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO agent_stats (game_id, player, agent_name, won, score) VALUES \
             ('g1', 0, 'random',   1, 121), ('g1', 1, 'scripted', 0, 95), \
             ('g2', 0, 'random',   0, 80),  ('g2', 1, 'scripted', 1, 121)",
            [],
        )
        .unwrap();

        let mut md = MarkdownBuilder::new();
        write_summary_section(&mut md, &conn).unwrap();
        write_per_agent_section(&mut md, &conn).unwrap();
        let out = md.into_string();
        assert!(out.contains("Total games: **2**"));
        assert!(out.contains("Average event count per game: **15.0**"));
        assert!(out.contains("| player_0 |"), "winner p0 row absent: {out}");
        assert!(out.contains("| player_1 |"), "winner p1 row absent: {out}");
        assert!(out.contains("| random   |"));
        assert!(out.contains("| scripted |"));
        assert!(
            out.contains("50.0%"),
            "expected 50/50 agent win rates in: {out}"
        );
        assert!(
            out.contains("Total wins recorded: 2; total games: 2"),
            "expected invariant note in: {out}"
        );
    }
}
