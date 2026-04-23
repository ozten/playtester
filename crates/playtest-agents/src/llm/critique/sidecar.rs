//! Sidecar logger for post-game critique.
//!
//! Writes a `<run>/games/<gid>.critique.jsonl` file with one header
//! line followed by one `questionnaire_response` record per LlmAgent
//! seat and (after `playtest critique-code` runs) one `coded_tag`
//! record per seat.
//!
//! Structurally identical to [`LlmSidecar`](crate::llm::sidecar::LlmSidecar)
//! — same `Arc<Mutex<dyn FileSystem + Send>>` sharing pattern, same
//! line-atomic mutex discipline, same `FsError` propagation. The split
//! is semantic: cost-observability records (`llm_call`) live in
//! `.llm.jsonl`; subjective-signal records live here.
//!
//! Two invariants shape this module:
//!
//! 1. Every `LlmAgent` seat that answers the questionnaire emits
//!    exactly one `questionnaire_response` line. The `playtest
//!    critique-code` pass later emits at most one `coded_tag` record
//!    per seat.
//! 2. Sidecar failures never kill a game. Callers (`LlmAgent::
//!    post_game_critique`, the critique-code subcommand) log the error
//!    and move on — the main event log is authoritative, and losing a
//!    critique record is a degraded observation, not a correctness bug.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use playtest_ports::{FileSystem, FsError};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::spec::SpecVersion;

/// First line of a critique sidecar file. Pins the game identity, the
/// questionnaire schema hash (so cross-version data never aggregates
/// silently), and the rules-text hash (for cache-stability audit
/// parity with [`SidecarHeader`](crate::llm::sidecar::SidecarHeader)).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CritiqueSidecarHeader {
    /// Always `"critique_sidecar_header"`. Present so line-level
    /// consumers can dispatch on `kind` like every other JSONL record.
    pub kind: String,
    pub game: String,
    pub seed: u64,
    pub questionnaire_spec_sha256: String,
    pub rules_text_sha256: String,
}

impl CritiqueSidecarHeader {
    #[must_use]
    pub fn new(
        game: impl Into<String>,
        seed: u64,
        questionnaire_spec_sha256: impl Into<String>,
        rules_text_sha256: impl Into<String>,
    ) -> Self {
        Self {
            kind: "critique_sidecar_header".to_owned(),
            game: game.into(),
            seed,
            questionnaire_spec_sha256: questionnaire_spec_sha256.into(),
            rules_text_sha256: rules_text_sha256.into(),
        }
    }
}

/// One record per LlmAgent seat. Likert scores are `u8` in the range
/// 1..=5 (not enforced by the type system — the caller validates
/// before constructing); open-ended strings are arbitrary UTF-8.
///
/// `BTreeMap` (not `HashMap`) keeps key ordering deterministic at
/// serialize time — important for byte-level replay tape diffing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionnaireResponseRecord {
    pub kind: String,
    pub seat: u8,
    pub spec_version: SpecVersion,
    pub likert: BTreeMap<String, u8>,
    pub open_ended: BTreeMap<String, String>,
}

impl QuestionnaireResponseRecord {
    #[must_use]
    pub fn new(
        seat: u8,
        spec_version: SpecVersion,
        likert: BTreeMap<String, u8>,
        open_ended: BTreeMap<String, String>,
    ) -> Self {
        Self {
            kind: "questionnaire_response".to_owned(),
            seat,
            spec_version,
            likert,
            open_ended,
        }
    }
}

/// A single coded tag extracted from an open-ended response by the
/// offline `playtest critique-code` pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodedTag {
    pub tag: String,
    pub severity: u8,
    /// `None` when the tag blames no specific card.
    pub ref_card: Option<String>,
}

/// One record per seat per coding run. Appending a second record for
/// the same `(game_id, seat)` is intentional: the latest record wins
/// at ingest time via `INSERT OR REPLACE`, so `--overwrite` of the
/// coder subcommand works by append, not by in-place edit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodedTagRecord {
    pub kind: String,
    pub seat: u8,
    pub tags: Vec<CodedTag>,
}

impl CodedTagRecord {
    #[must_use]
    pub fn new(seat: u8, tags: Vec<CodedTag>) -> Self {
        Self {
            kind: "coded_tag".to_owned(),
            seat,
            tags,
        }
    }
}

/// A critique-sidecar appender — `Arc`-shareable across agents within
/// one run, mutex-serialized so concurrent appends never tear across
/// line boundaries. One instance per `<gid>.critique.jsonl` file.
pub struct CritiqueSidecar {
    fs: Arc<Mutex<dyn FileSystem + Send>>,
    path: PathBuf,
}

impl core::fmt::Debug for CritiqueSidecar {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CritiqueSidecar")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl CritiqueSidecar {
    /// Construct a sidecar whose first line is `header`.
    ///
    /// # Errors
    /// Propagates [`FsError`] from the underlying `FileSystem` port
    /// if the header write fails.
    pub async fn new(
        fs: Arc<Mutex<dyn FileSystem + Send>>,
        path: PathBuf,
        header: CritiqueSidecarHeader,
    ) -> Result<Self, FsError> {
        let line = serde_json::to_string(&header)
            .expect("CritiqueSidecarHeader Serialize is infallible");
        {
            let mut guard = fs.lock().await;
            guard.append_line(&path, &line)?;
        }
        Ok(Self { fs, path })
    }

    /// Append a single `questionnaire_response` record.
    ///
    /// # Errors
    /// Propagates [`FsError`] from the underlying `FileSystem` port.
    pub async fn append_questionnaire(
        &self,
        record: &QuestionnaireResponseRecord,
    ) -> Result<(), FsError> {
        let line = serde_json::to_string(record)
            .expect("QuestionnaireResponseRecord Serialize is infallible");
        let mut guard = self.fs.lock().await;
        guard.append_line(&self.path, &line)
    }

    /// Append a single `coded_tag` record. Subsequent calls for the
    /// same `(game_id, seat)` append additional lines; the ingest
    /// layer's `INSERT OR REPLACE` gives last-writer-wins semantics
    /// (see Phase 5 Unit 6).
    ///
    /// # Errors
    /// Propagates [`FsError`] from the underlying `FileSystem` port.
    pub async fn append_coded_tags(&self, record: &CodedTagRecord) -> Result<(), FsError> {
        let line = serde_json::to_string(record)
            .expect("CodedTagRecord Serialize is infallible");
        let mut guard = self.fs.lock().await;
        guard.append_line(&self.path, &line)
    }

    /// File path this sidecar writes to. Exposed so callers can log or
    /// assert on the target.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use playtest_adapters::StubFileSystem;

    fn stub_fs() -> Arc<Mutex<dyn FileSystem + Send>> {
        Arc::new(Mutex::new(StubFileSystem::new()))
    }

    fn sample_header() -> CritiqueSidecarHeader {
        CritiqueSidecarHeader::new("shipwreck", 42, "spec-sha", "rules-sha")
    }

    fn sample_questionnaire(seat: u8) -> QuestionnaireResponseRecord {
        let mut likert = BTreeMap::new();
        likert.insert("agency".to_owned(), 4);
        likert.insert("fairness".to_owned(), 5);
        let mut open_ended = BTreeMap::new();
        open_ended.insert("worst_moment".to_owned(), "typhoon took my cordage".into());
        QuestionnaireResponseRecord::new(seat, 1, likert, open_ended)
    }

    fn sample_coded_tags(seat: u8) -> CodedTagRecord {
        CodedTagRecord::new(
            seat,
            vec![
                CodedTag {
                    tag: "forced_sacrifice".into(),
                    severity: 3,
                    ref_card: Some("typhoon".into()),
                },
                CodedTag {
                    tag: "lack_of_agency".into(),
                    severity: 2,
                    ref_card: None,
                },
            ],
        )
    }

    #[tokio::test]
    async fn header_written_as_first_line_with_correct_kind() {
        let fs = stub_fs();
        let path = PathBuf::from("/run/games/abc.critique.jsonl");
        let _sidecar = CritiqueSidecar::new(fs.clone(), path.clone(), sample_header())
            .await
            .unwrap();
        let guard = fs.lock().await;
        let bytes = guard.read(&path).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\"kind\":\"critique_sidecar_header\""));
        assert!(lines[0].contains("\"game\":\"shipwreck\""));
        assert!(lines[0].contains("\"questionnaire_spec_sha256\":\"spec-sha\""));
    }

    #[tokio::test]
    async fn append_questionnaire_tags_kind_and_preserves_key_order() {
        let fs = stub_fs();
        let path = PathBuf::from("/run/games/abc.critique.jsonl");
        let sidecar = CritiqueSidecar::new(fs.clone(), path.clone(), sample_header())
            .await
            .unwrap();
        sidecar
            .append_questionnaire(&sample_questionnaire(0))
            .await
            .unwrap();
        let guard = fs.lock().await;
        let bytes = guard.read(&path).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let rec = lines[1];
        assert!(rec.contains("\"kind\":\"questionnaire_response\""));
        assert!(rec.contains("\"seat\":0"));
        assert!(rec.contains("\"spec_version\":1"));
        // BTreeMap serializes keys in sorted order — `agency` comes
        // before `fairness`.
        let agency_pos = rec.find("agency").expect("agency key present");
        let fairness_pos = rec.find("fairness").expect("fairness key present");
        assert!(agency_pos < fairness_pos, "BTreeMap must yield sorted keys");
    }

    #[tokio::test]
    async fn append_coded_tags_writes_one_line_with_kind_tag() {
        let fs = stub_fs();
        let path = PathBuf::from("/run/games/abc.critique.jsonl");
        let sidecar = CritiqueSidecar::new(fs.clone(), path.clone(), sample_header())
            .await
            .unwrap();
        sidecar
            .append_coded_tags(&sample_coded_tags(0))
            .await
            .unwrap();
        let guard = fs.lock().await;
        let bytes = guard.read(&path).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let rec = lines[1];
        assert!(rec.contains("\"kind\":\"coded_tag\""));
        assert!(rec.contains("forced_sacrifice"));
        assert!(rec.contains("\"severity\":3"));
        // `ref_card` is serialized as JSON null when None.
        assert!(rec.contains("\"ref_card\":null"));
    }

    #[tokio::test]
    async fn two_seat_run_produces_five_lines_in_expected_order() {
        let fs = stub_fs();
        let path = PathBuf::from("/run/games/abc.critique.jsonl");
        let sidecar = CritiqueSidecar::new(fs.clone(), path.clone(), sample_header())
            .await
            .unwrap();
        sidecar
            .append_questionnaire(&sample_questionnaire(0))
            .await
            .unwrap();
        sidecar
            .append_questionnaire(&sample_questionnaire(1))
            .await
            .unwrap();
        sidecar.append_coded_tags(&sample_coded_tags(0)).await.unwrap();
        sidecar.append_coded_tags(&sample_coded_tags(1)).await.unwrap();

        let guard = fs.lock().await;
        let bytes = guard.read(&path).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 5, "header + 2 questionnaire + 2 coded_tag");
        assert!(lines[0].contains("critique_sidecar_header"));
        assert!(lines[1].contains("questionnaire_response") && lines[1].contains("\"seat\":0"));
        assert!(lines[2].contains("questionnaire_response") && lines[2].contains("\"seat\":1"));
        assert!(lines[3].contains("coded_tag") && lines[3].contains("\"seat\":0"));
        assert!(lines[4].contains("coded_tag") && lines[4].contains("\"seat\":1"));
    }

    #[tokio::test]
    async fn concurrent_appends_never_tear_lines() {
        let fs = stub_fs();
        let path = PathBuf::from("/run/games/abc.critique.jsonl");
        let sidecar = Arc::new(
            CritiqueSidecar::new(fs.clone(), path.clone(), sample_header())
                .await
                .unwrap(),
        );

        let mut handles = Vec::new();
        for seat in 0..8_u8 {
            let s = sidecar.clone();
            handles.push(tokio::spawn(async move {
                s.append_questionnaire(&sample_questionnaire(seat))
                    .await
                    .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let guard = fs.lock().await;
        let bytes = guard.read(&path).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 9, "header + 8 questionnaire records");
        for line in &lines[1..] {
            // Every non-header line parses as a full JSON object —
            // proves the mutex prevented interleaved writes.
            let _parsed: serde_json::Value =
                serde_json::from_str(line).expect("line is well-formed JSON");
        }
    }

    #[tokio::test]
    async fn kinds_are_distinct_from_llm_sidecar_kinds() {
        // `llm_call` and `sidecar_header` belong to `.llm.jsonl`.
        // Critique kinds must not collide — a defensive grep-safety
        // guarantee for operators scanning combined log output.
        let header = CritiqueSidecarHeader::new("g", 0, "a", "b");
        assert_ne!(header.kind, "llm_call");
        assert_ne!(header.kind, "sidecar_header");
        let q = QuestionnaireResponseRecord::new(0, 1, BTreeMap::new(), BTreeMap::new());
        assert_ne!(q.kind, "llm_call");
        assert_ne!(q.kind, "sidecar_header");
        let c = CodedTagRecord::new(0, vec![]);
        assert_ne!(c.kind, "llm_call");
        assert_ne!(c.kind, "sidecar_header");
    }

    #[test]
    fn coded_tag_round_trips_through_json() {
        let tag = CodedTag {
            tag: "forced_sacrifice".into(),
            severity: 3,
            ref_card: Some("typhoon".into()),
        };
        let json = serde_json::to_string(&tag).unwrap();
        let parsed: CodedTag = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, tag);
    }
}
