//! Post-game questionnaire schema.
//!
//! Eight Likert items (1–5 scale) plus two open-ended prompts, matching
//! the roadmap's Phase 5 list. The schema is a hardcoded Rust constant
//! whose SHA-256 gets stored in the critique sidecar header, so the
//! markdown reporter can detect cross-version drift at aggregate time.
//!
//! Keep the item count between 8 and 12 and the open-ended count between
//! 2 and 3 (enforced by `debug_assert!` in `default_questionnaire_v1`).
//! Changing any item text bumps the SHA-256 — downstream mixing of old
//! and new versions is the reporter's problem, not this module's.

use serde::Serialize;

use crate::llm::sidecar::sha256_hex;

/// A single Likert-scale question. Every item uses an implicit 1–5
/// scale; the scale is not a field on the item so the JSON shape stays
/// minimal. A future variant family can replace this struct with an
/// enum if additional question kinds ever ship.
///
/// The struct intentionally does not implement `Deserialize` — specs
/// are defined in Rust source, never read back from disk or the wire.
/// Round-tripped data uses `BTreeMap<String, _>` shapes on the sidecar
/// record types (see `critique/sidecar.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuestionItem {
    pub id: &'static str,
    pub text: &'static str,
}

/// An open-ended free-form prompt. See [`QuestionItem`] for the
/// "no-Deserialize" rationale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenEndedPrompt {
    pub id: &'static str,
    pub text: &'static str,
}

/// Version of the questionnaire schema. Bumped on any item change so
/// the reporter can surface cross-version warnings.
pub type SpecVersion = u16;

/// The full post-game questionnaire: Likert items + open-ended prompts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuestionnaireSpec {
    pub version: SpecVersion,
    pub items: Vec<QuestionItem>,
    pub open_ended: Vec<OpenEndedPrompt>,
}

impl QuestionnaireSpec {
    /// SHA-256 of the canonical JSON representation. Stable as long as
    /// every field value is byte-identical — changing a single char in
    /// any item text bumps the hash.
    #[must_use]
    pub fn sha256(&self) -> String {
        let json = serde_json::to_string(self).expect("QuestionnaireSpec Serialize is infallible");
        sha256_hex(json.as_bytes())
    }
}

/// The default 8-item + 2-prompt questionnaire shipped in Phase 5.
///
/// Item IDs match the roadmap's list. Text wording is first-pass; tune
/// during prompt iteration without bumping the constructor signature
/// (but do bump `version` when text drifts to protect aggregate data).
///
/// # Panics (debug only)
///
/// Asserts 8 ≤ items ≤ 12 and 2 ≤ open_ended ≤ 3 — invariants from R5.2.
#[must_use]
pub fn default_questionnaire_v1() -> QuestionnaireSpec {
    let spec = QuestionnaireSpec {
        version: 1,
        items: vec![
            QuestionItem {
                id: "agency",
                text: "I felt that my decisions meaningfully influenced the outcome of the game.",
            },
            QuestionItem {
                id: "fairness",
                text: "The outcomes of random events and opponent plays felt fair.",
            },
            QuestionItem {
                id: "tension",
                text: "I felt genuine tension about whether I would win or lose.",
            },
            QuestionItem {
                id: "pacing",
                text: "The turn-to-turn pacing of the match felt right — not too fast, not too slow.",
            },
            QuestionItem {
                id: "variety",
                text: "I faced an interesting variety of choices across different turns.",
            },
            QuestionItem {
                id: "frustration",
                text: "I felt frustrated by moments where the game took decisions away from me.",
            },
            QuestionItem {
                id: "satisfaction",
                text: "Overall, I enjoyed playing this match.",
            },
            QuestionItem {
                id: "would_play_again",
                text: "I would willingly play another match of this game.",
            },
        ],
        open_ended: vec![
            OpenEndedPrompt {
                id: "worst_moment",
                text: "What moment of the game felt the worst? Describe what happened and why it felt bad.",
            },
            OpenEndedPrompt {
                id: "what_would_you_change",
                text: "If you could change one rule, card, or mechanic to improve the experience, what would it be?",
            },
        ],
    };
    debug_assert!(
        (8..=12).contains(&spec.items.len()),
        "Likert item count must be 8..=12 per R5.2, got {}",
        spec.items.len()
    );
    debug_assert!(
        (2..=3).contains(&spec.open_ended.len()),
        "open-ended prompt count must be 2..=3 per R5.2, got {}",
        spec.open_ended.len()
    );
    spec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_eight_likert_items_and_two_open_ended() {
        let spec = default_questionnaire_v1();
        assert_eq!(spec.items.len(), 8);
        assert_eq!(spec.open_ended.len(), 2);
        assert_eq!(spec.version, 1);
    }

    #[test]
    fn default_item_ids_match_roadmap_list() {
        let spec = default_questionnaire_v1();
        let ids: Vec<&str> = spec.items.iter().map(|i| i.id).collect();
        assert_eq!(
            ids,
            vec![
                "agency",
                "fairness",
                "tension",
                "pacing",
                "variety",
                "frustration",
                "satisfaction",
                "would_play_again",
            ]
        );
    }

    #[test]
    fn default_open_ended_ids_are_stable() {
        let spec = default_questionnaire_v1();
        let ids: Vec<&str> = spec.open_ended.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec!["worst_moment", "what_would_you_change"]);
    }

    #[test]
    fn sha256_is_64_lowercase_hex() {
        let hex = default_questionnaire_v1().sha256();
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
    }

    #[test]
    fn sha256_is_stable_across_reruns() {
        assert_eq!(
            default_questionnaire_v1().sha256(),
            default_questionnaire_v1().sha256()
        );
    }

    #[test]
    fn sha256_changes_when_item_text_changes() {
        let a = default_questionnaire_v1().sha256();
        let mut spec = default_questionnaire_v1();
        spec.items[0].text = "different wording";
        let b = spec.sha256();
        assert_ne!(a, b);
    }

    #[test]
    fn sha256_changes_when_version_changes() {
        let a = default_questionnaire_v1().sha256();
        let mut spec = default_questionnaire_v1();
        spec.version = 2;
        let b = spec.sha256();
        assert_ne!(a, b);
    }

    #[test]
    fn item_ids_are_unique() {
        let spec = default_questionnaire_v1();
        let mut ids: Vec<&str> = spec.items.iter().map(|i| i.id).collect();
        let original_len = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), original_len, "item ids must be unique");
    }

    #[test]
    fn open_ended_ids_are_unique() {
        let spec = default_questionnaire_v1();
        let mut ids: Vec<&str> = spec.open_ended.iter().map(|p| p.id).collect();
        let original_len = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), original_len, "open-ended ids must be unique");
    }
}
