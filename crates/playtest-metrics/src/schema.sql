-- SQLite schema for Unit 14 ingestion.
--
-- Three tables back the Unit 15 reporter:
--
--   games          — one row per game file ingested; the "what happened"
--                    header/Final facts in a single record.
--   agent_stats    — one row per (game, player) pair; agent identity plus
--                    the per-player outcomes from the game's Final record.
--   game_metrics   — tall-and-thin store for every MetricValue emitted by
--                    the registries (BuiltInMetrics + any game registry).
--
-- Design choices:
-- * `tag` in game_metrics is TEXT NOT NULL (defaulting to '') so the
--   composite PRIMARY KEY works on SQLite — where NULL != NULL would
--   otherwise defeat uniqueness checks on untagged values.
-- * `player` in game_metrics is INTEGER NOT NULL with -1 as the
--   sentinel for game-scoped metrics. Same reason as `tag`: STRICT
--   tables forbid NULL in a PRIMARY KEY column, and even on a non-
--   STRICT table NULL would defeat INSERT OR REPLACE deduplication.
--   Queries for game-scoped metrics match `player = -1`; the query
--   module hides the sentinel from callers.
-- * Value is polymorphic via `value_kind` + one of `value_numeric` /
--   `value_text`; the reporter picks the right column based on kind.
-- * Boolean metrics land in value_numeric as 0/1 — SQLite has no
--   native bool, and storing 0/1 keeps aggregate SQL trivial.
-- * `INSERT OR REPLACE` on every row keeps ingestion idempotent: a
--   second run over the same directory replaces rather than duplicates.

CREATE TABLE IF NOT EXISTS games (
    id           TEXT PRIMARY KEY,   -- derived from the log header (see ingest.rs)
    game         TEXT NOT NULL,
    version      TEXT NOT NULL,
    seed         INTEGER NOT NULL,
    started_at   INTEGER NOT NULL,   -- Unix ms from LogHeader
    finished_at  INTEGER,            -- Unix ms from the v2 Final record; NULL for v1 / crashed games
    winner       INTEGER,            -- NULL on draw / unfinished
    end_reason   TEXT NOT NULL,      -- "victory" / "draw" / "stalemate" / "other:<...>" / "unfinished"
    config_hash  TEXT NOT NULL,
    event_count  INTEGER NOT NULL,
    source_path  TEXT NOT NULL       -- the JSONL file this row came from
) STRICT;

CREATE INDEX IF NOT EXISTS idx_games_game ON games(game);
CREATE INDEX IF NOT EXISTS idx_games_end_reason ON games(end_reason);

CREATE TABLE IF NOT EXISTS agent_stats (
    game_id      TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    player       INTEGER NOT NULL,
    agent_name   TEXT NOT NULL,
    won          INTEGER NOT NULL,    -- 0 or 1
    score        INTEGER NOT NULL,
    PRIMARY KEY (game_id, player)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_agent_stats_agent_name ON agent_stats(agent_name);

CREATE TABLE IF NOT EXISTS game_metrics (
    game_id       TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    metric_name   TEXT NOT NULL,
    player        INTEGER NOT NULL DEFAULT -1,  -- -1 sentinel for game-scoped
    tag           TEXT NOT NULL DEFAULT '',     -- '' for untagged
    value_kind    TEXT NOT NULL,                -- "scalar" | "count" | "tag" | "bool"
    value_numeric REAL,                         -- populated for Scalar/Count/Bool
    value_text    TEXT,                         -- populated for Tag (and categorical)
    PRIMARY KEY (game_id, metric_name, player, tag)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_game_metrics_name ON game_metrics(metric_name);
CREATE INDEX IF NOT EXISTS idx_game_metrics_name_tag ON game_metrics(metric_name, tag);
