//! Sidecar logger for LLM calls.
//!
//! Writes a `<run>/games/<gid>.llm.jsonl` file with one header line
//! followed by one record per LLM call. Writes go through the existing
//! `FileSystem` port — there is no new `SidecarWriter` trait.
//!
//! Two invariants shape this module:
//!
//! 1. Multiple `LlmAgent`s in the same run may share a single
//!    `Arc<LlmSidecar>`. The mutex guarantees line-atomic appends.
//! 2. Sidecar failures are logged to stderr and ignored by the agent —
//!    the main event log is authoritative; losing a cost-observability
//!    record should never kill a game.

use std::path::PathBuf;
use std::sync::Arc;

use playtest_ports::{FileSystem, FsError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

/// Convenience: SHA-256 of `bytes` as lowercase hex. Used by callers
/// building a [`SidecarHeader`] so the cache-stability hash recorded in
/// the header always matches the bytes actually sent to the LLM.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

/// First line of a sidecar file. Records the game identifier and hashes
/// of the hand-written rules text + card catalog so a drifted rules file
/// is visible to cache-stability auditing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarHeader {
    /// Always `"sidecar_header"`. Present so line-level consumers can
    /// dispatch on `kind` like every other JSONL record in the repo.
    pub kind: String,
    pub game: String,
    pub seed: u64,
    pub rules_text_sha256: String,
    pub card_catalog_sha256: String,
}

impl SidecarHeader {
    #[must_use]
    pub fn new(
        game: impl Into<String>,
        seed: u64,
        rules_text_sha256: impl Into<String>,
        card_catalog_sha256: impl Into<String>,
    ) -> Self {
        Self {
            kind: "sidecar_header".to_owned(),
            game: game.into(),
            seed,
            rules_text_sha256: rules_text_sha256.into(),
            card_catalog_sha256: card_catalog_sha256.into(),
        }
    }
}

/// One record per LLM call. Serialized with a `kind: "llm_call"` tag so
/// the header and the body share the same on-disk namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCallRecord {
    pub tick: u64,
    pub seat: u8,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_input_tokens: u32,
    pub cache_creation_input_tokens: u32,
    pub latency_ms: u64,
    /// `None` if the call failed before producing a choice.
    pub chosen_index: Option<usize>,
    pub budget_exceeded: bool,
}

/// A sidecar appender: `Arc`-shareable across agents, mutex-serialized so
/// concurrent appends never tear across line boundaries.
pub struct LlmSidecar {
    fs: Arc<Mutex<dyn FileSystem + Send>>,
    path: PathBuf,
}

impl core::fmt::Debug for LlmSidecar {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LlmSidecar")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl LlmSidecar {
    /// Construct a sidecar whose first line is `header`.
    ///
    /// # Errors
    /// Propagates [`FsError`] from the underlying `FileSystem` port if
    /// the header write fails.
    pub async fn new(
        fs: Arc<Mutex<dyn FileSystem + Send>>,
        path: PathBuf,
        header: SidecarHeader,
    ) -> Result<Self, FsError> {
        let line = serde_json::to_string(&header).expect("SidecarHeader Serialize is infallible");
        {
            let mut guard = fs.lock().await;
            guard.append_line(&path, &line)?;
        }
        Ok(Self { fs, path })
    }

    /// Append a single `llm_call` record. Serializes the record with its
    /// `kind` tag prepended, then writes it as one line through the
    /// `FileSystem` port.
    ///
    /// # Errors
    /// Propagates [`FsError`] from the underlying `FileSystem` port.
    pub async fn append_call(&self, record: &LlmCallRecord) -> Result<(), FsError> {
        let mut value = serde_json::to_value(record)
            .expect("LlmCallRecord Serialize produces plain JSON object");
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "kind".to_owned(),
                serde_json::Value::String("llm_call".to_owned()),
            );
        }
        let line = serde_json::to_string(&value).expect("serde_json::Value serialization");
        let mut guard = self.fs.lock().await;
        guard.append_line(&self.path, &line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use playtest_adapters::StubFileSystem;

    #[test]
    fn sha256_hex_is_64_chars_lowercase() {
        let hex = sha256_hex(b"abc");
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
        // Known digest for "abc".
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn stub() -> Arc<Mutex<dyn FileSystem + Send>> {
        Arc::new(Mutex::new(StubFileSystem::new()))
    }

    #[tokio::test]
    async fn header_is_written_as_first_line() {
        let fs = stub();
        let header = SidecarHeader::new("cribbage", 7, "aaa", "bbb");
        let path = PathBuf::from("/run/games/abc.llm.jsonl");
        let _sidecar = LlmSidecar::new(fs.clone(), path.clone(), header)
            .await
            .unwrap();

        let guard = fs.lock().await;
        // Reflect back through the port to keep the test boundary honest.
        let bytes = guard.read(&path).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"kind\":\"sidecar_header\""));
        assert!(text.contains("\"game\":\"cribbage\""));
        assert!(text.contains("\"rules_text_sha256\":\"aaa\""));
    }

    #[tokio::test]
    async fn append_call_tags_kind_and_appends_line() {
        let fs = stub();
        let path = PathBuf::from("/run/games/abc.llm.jsonl");
        let sidecar = LlmSidecar::new(
            fs.clone(),
            path.clone(),
            SidecarHeader::new("cribbage", 7, "a", "b"),
        )
        .await
        .unwrap();
        sidecar
            .append_call(&LlmCallRecord {
                tick: 0,
                seat: 0,
                model: "claude-test".into(),
                input_tokens: 100,
                output_tokens: 20,
                cache_read_input_tokens: 80,
                cache_creation_input_tokens: 0,
                latency_ms: 25,
                chosen_index: Some(1),
                budget_exceeded: false,
            })
            .await
            .unwrap();

        let guard = fs.lock().await;
        let bytes = guard.read(&path).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("\"kind\":\"llm_call\""));
        assert!(lines[1].contains("\"chosen_index\":1"));
    }
}
