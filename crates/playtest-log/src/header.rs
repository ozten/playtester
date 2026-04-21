//! Log header: the first line of every event log file.
//!
//! The header fixes everything needed to verify and replay the log:
//! schema version, game identity and version, RNG seed, agent names,
//! start timestamp, and a hash of the game config. A mismatch on any
//! field (schema, game, config_hash) at replay time is a hard error.

use playtest_ports::UnixMillis;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Current event-log schema version. Bump when `LogRecord` shape
/// changes in a way prior logs cannot be read under.
///
/// - `1` — initial shape: header + events + `Final { winner, reason, scores }`
/// - `2` — adds `finished_at` to the `Final` record so `wall_clock_ms`
///   can be derived. Old v1 logs without `finished_at` deserialize
///   with `finished_at = 0` (serde default), and replay rejects any
///   `schema != 2` log outright.
pub const SCHEMA_VERSION: u32 = 2;

/// Everything needed to verify and replay a log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogHeader {
    /// Schema version of the log file. Must equal [`SCHEMA_VERSION`] at
    /// replay time.
    pub schema: u32,
    /// Short game identifier, e.g. `"cribbage"`. Replay rejects a log
    /// if this does not match the runtime `Game` impl.
    pub game: String,
    /// Game-specific version, e.g. `"0.1.0"`. Not currently enforced at
    /// replay but carried for compatibility diagnostics.
    pub version: String,
    /// RNG seed passed to `Game::initial_state`.
    pub seed: u64,
    /// Agent names in player-index order. Mostly for human diagnostics;
    /// replay does not invoke agents.
    pub agents: Vec<String>,
    /// Wall-clock start time in Unix epoch milliseconds (from the
    /// [`Clock`](playtest_ports::Clock) port).
    pub started_at: UnixMillis,
    /// Hex-encoded SHA-256 of the JSON-serialized `Game::Config`.
    /// Detects replay against a different config shape. See
    /// [`compute_config_hash`].
    pub config_hash: String,
}

/// Compute the config hash used in [`LogHeader::config_hash`].
///
/// JSON-serializes `config` and SHA-256s the resulting bytes. The hash
/// is deterministic within a single build but not *strictly* stable
/// across `serde_json` versions — serde may reorder fields or change
/// whitespace between minor releases. That's acceptable for Phase 0
/// because replay is always done by a build with a compatible stack.
///
/// # Errors
/// Returns a `serde_json::Error` if the config does not serialize.
pub fn compute_config_hash<T: Serialize>(config: &T) -> Result<String, serde_json::Error> {
    let json = serde_json::to_vec(config)?;
    let mut hasher = Sha256::new();
    hasher.update(&json);
    let digest = hasher.finalize();
    Ok(hex_encode(&digest))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_config_hashes_equal() {
        let a = compute_config_hash(&"cribbage-cfg").unwrap();
        let b = compute_config_hash(&"cribbage-cfg").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64, "sha256 hex is 64 chars");
    }

    #[test]
    fn different_configs_produce_different_hashes() {
        let a = compute_config_hash(&"cfg-a").unwrap();
        let b = compute_config_hash(&"cfg-b").unwrap();
        assert_ne!(a, b);
    }
}
