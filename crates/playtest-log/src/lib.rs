//! Event log: JSONL writer, reader, and replay.
//!
//! One file per game. Header + events + final record. State snapshots are
//! derived by replaying events from the seed, not serialized separately.
