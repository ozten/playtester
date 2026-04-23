//! Offline coder pass — extracts structured `CodedTag` values from the
//! free-form open-ended responses in a `<gid>.critique.jsonl` sidecar.
//!
//! Runs via `playtest critique-code <run-dir>`. The coder prompt
//! presents a fixed taxonomy of ~20 tags; the LLM picks zero or more
//! from the list, assigns a 1–5 severity, and optionally blames a
//! specific card by name. Tags the LLM produces that are outside the
//! taxonomy are warned-and-dropped (coder drift is tolerated at the
//! per-record level but never silently widens the schema).
//!
//! This module has no filesystem I/O — it builds prompts, parses
//! replies, and returns validated `CodedTag` values. The subcommand
//! does file I/O.

use std::collections::BTreeMap;

use playtest_ports::{ChatMessage, ChatRole, LlmClient, LlmError, LlmRequest, SystemBlock};
use serde::{Deserialize, Serialize};

use super::sidecar::CodedTag;

/// The fixed tag taxonomy. The coder picks zero or more of these
/// per open-ended response. Adding a tag here is a schema change
/// that should bump `QuestionnaireSpec::version` so downstream
/// reporters can detect the drift.
///
/// Tags cluster across agency, pacing, tension, variety, and balance
/// dimensions — matching the Likert-item dimensions in `spec::
/// default_questionnaire_v1`.
pub const CODER_TAG_TAXONOMY: &[&str] = &[
    // Agency cluster
    "forced_sacrifice",
    "lack_of_agency",
    "random_loss",
    "no_counterplay",
    // Pacing cluster
    "turn_length",
    "slow_start",
    "rushed_endgame",
    "boring_early_game",
    // Tension cluster
    "snowball_win",
    "snowball_loss",
    "anticlimactic",
    "close_finish",
    // Variety cluster
    "repetitive_choices",
    "surprising_moment",
    "stale_opening",
    "novel_interaction",
    // Balance cluster
    "overwhelming_lead",
    "satisfying_comeback",
    "unclear_rules",
    "blowout",
    // Endgame quality
    "stalemate_feeling",
    "tense_endgame",
];

/// Build the LlmRequest for coding one seat's open-ended responses.
///
/// System block 0: taxonomy definition + reply contract.
/// User message: the open-ended responses keyed by prompt id.
#[must_use]
pub fn build_coder_request(
    open_ended: &BTreeMap<String, String>,
    model: String,
    max_tokens: u32,
) -> LlmRequest {
    let system = build_coder_system_block();
    let user = build_coder_user_message(open_ended);
    LlmRequest {
        system_blocks: vec![SystemBlock {
            text: system,
            cache: true,
        }],
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: user,
        }],
        model,
        max_tokens,
        temperature: None,
    }
}

/// Cacheable system block: taxonomy + reply-format contract.
#[must_use]
pub fn build_coder_system_block() -> String {
    use core::fmt::Write as _;
    let mut s = String::new();
    s.push_str(
        "You are a coder extracting structured tags from a game \
playtester's free-form critique. You will receive a set of open-ended \
responses; your job is to classify any pain or delight moments they \
describe into a fixed taxonomy.\n\
\n\
Reply with ONLY a single JSON object with exactly this key:\n\
  \"tags\": array of objects, each with shape \
{\"tag\": string, \"severity\": integer 1-5, \"ref_card\": string|null}\n\
\n\
Rules:\n\
- Use ONLY tags from the taxonomy below. If no tag applies, return an \
empty array (`\"tags\": []`).\n\
- `severity` is 1-5: 1 = mild, 5 = game-breaking. Default to 3 if \
unsure.\n\
- `ref_card` is the name of a specific card mentioned in the response \
(e.g. \"typhoon\", \"seven_of_diamonds\"). Use null when no card is blamed.\n\
- One response may produce zero, one, or many tags. Over-tag when \
multiple dimensions are implicated.\n\
- Be literal. Do not invent content not in the response text.\n\
\n\
Taxonomy (tag — what it means):\n",
    );
    for &tag in CODER_TAG_TAXONOMY {
        let _ = writeln!(s, "  - \"{tag}\"");
    }
    s.push_str(
        "\nDo not emit any text outside the JSON object. Do not wrap \
the JSON in markdown fences.",
    );
    s
}

#[must_use]
fn build_coder_user_message(open_ended: &BTreeMap<String, String>) -> String {
    // BTreeMap round-trips through JSON with keys sorted, making the
    // byte output stable for replay-tape diffing.
    let payload = serde_json::json!({ "open_ended": open_ended });
    serde_json::to_string_pretty(&payload).expect("BTreeMap<String, String> serializes")
}

/// Shape the model must reply with.
#[derive(Debug, Deserialize, Serialize)]
struct CoderReply {
    tags: Vec<RawCodedTag>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RawCodedTag {
    tag: String,
    severity: u8,
    #[serde(default)]
    ref_card: Option<String>,
}

/// Outcome of parsing a single coder reply. `dropped_tags` are tags
/// the LLM produced that aren't in the taxonomy — surfaced so the
/// subcommand can log-and-continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoderOutcome {
    pub accepted: Vec<CodedTag>,
    pub dropped_tags: Vec<String>,
}

/// Parse + validate one coder reply. Drops out-of-taxonomy tags
/// silently into `dropped_tags`; rejects severity outside 1..=5 as
/// a hard error.
///
/// # Errors
/// Returns `String` with a human-readable parse error when the reply
/// is not well-formed JSON or when severity is out of range.
pub fn parse_coder_reply(text: &str) -> Result<CoderOutcome, String> {
    let reply: CoderReply =
        serde_json::from_str(text).map_err(|e| format!("coder reply is not valid JSON: {e}"))?;

    let mut accepted = Vec::new();
    let mut dropped_tags = Vec::new();
    for raw in reply.tags {
        if !(1..=5).contains(&raw.severity) {
            return Err(format!(
                "coder returned severity {} for tag `{}`; must be 1..=5",
                raw.severity, raw.tag
            ));
        }
        if CODER_TAG_TAXONOMY.iter().any(|t| *t == raw.tag) {
            accepted.push(CodedTag {
                tag: raw.tag,
                severity: raw.severity,
                ref_card: raw.ref_card,
            });
        } else {
            dropped_tags.push(raw.tag);
        }
    }
    Ok(CoderOutcome {
        accepted,
        dropped_tags,
    })
}

/// Issue one coder call for `open_ended`. Returns the validated
/// outcome. The caller appends a `CodedTagRecord` to the sidecar.
///
/// # Errors
/// Propagates `LlmError` as a string (the subcommand aggregates and
/// logs per-game rather than failing the whole run).
pub async fn code_once(
    llm: &dyn LlmClient,
    open_ended: &BTreeMap<String, String>,
    model: &str,
    max_tokens: u32,
) -> Result<CoderOutcome, String> {
    let req = build_coder_request(open_ended, model.to_owned(), max_tokens);
    let resp = llm
        .complete(req)
        .await
        .map_err(|e: LlmError| format!("coder llm call failed: {e}"))?;
    parse_coder_reply(&resp.text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_open_ended() -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert(
            "worst_moment".into(),
            "Typhoon blew away my steel cordage on turn 6 with no way to defend".into(),
        );
        m.insert(
            "what_would_you_change".into(),
            "let players spend food to block typhoons".into(),
        );
        m
    }

    #[test]
    fn system_block_lists_every_taxonomy_tag() {
        let block = build_coder_system_block();
        for &tag in CODER_TAG_TAXONOMY {
            assert!(
                block.contains(&format!("\"{tag}\"")),
                "system block missing tag `{tag}`"
            );
        }
        assert!(block.contains("severity"));
        assert!(block.contains("ref_card"));
    }

    #[test]
    fn system_block_forbids_markdown_fences_and_extra_text() {
        let block = build_coder_system_block();
        assert!(block.contains("Do not wrap"));
        assert!(block.contains("Do not emit any text outside"));
    }

    #[test]
    fn user_message_embeds_open_ended_keys_sorted() {
        let msg = build_coder_user_message(&sample_open_ended());
        // BTreeMap serializes keys sorted. Second-char 'h' < 'o', so
        // `what_would_you_change` precedes `worst_moment`.
        let what_pos = msg.find("what_would_you_change").unwrap();
        let worst_pos = msg.find("worst_moment").unwrap();
        assert!(what_pos < worst_pos, "expected `what_*` before `worst_*`");
    }

    #[test]
    fn parse_accepts_well_formed_reply() {
        let text = r#"{"tags": [
            {"tag": "forced_sacrifice", "severity": 3, "ref_card": "typhoon"},
            {"tag": "lack_of_agency",   "severity": 2, "ref_card": null}
        ]}"#;
        let outcome = parse_coder_reply(text).unwrap();
        assert_eq!(outcome.accepted.len(), 2);
        assert_eq!(outcome.accepted[0].tag, "forced_sacrifice");
        assert_eq!(outcome.accepted[0].severity, 3);
        assert_eq!(outcome.accepted[0].ref_card.as_deref(), Some("typhoon"));
        assert_eq!(outcome.accepted[1].ref_card, None);
        assert!(outcome.dropped_tags.is_empty());
    }

    #[test]
    fn parse_accepts_empty_tags_array() {
        let outcome = parse_coder_reply(r#"{"tags": []}"#).unwrap();
        assert!(outcome.accepted.is_empty());
        assert!(outcome.dropped_tags.is_empty());
    }

    #[test]
    fn parse_drops_out_of_taxonomy_tags() {
        let text = r#"{"tags": [
            {"tag": "forced_sacrifice", "severity": 3, "ref_card": null},
            {"tag": "invented_tag",     "severity": 2, "ref_card": null}
        ]}"#;
        let outcome = parse_coder_reply(text).unwrap();
        assert_eq!(outcome.accepted.len(), 1);
        assert_eq!(outcome.accepted[0].tag, "forced_sacrifice");
        assert_eq!(outcome.dropped_tags, vec!["invented_tag"]);
    }

    #[test]
    fn parse_rejects_severity_out_of_range() {
        let text = r#"{"tags": [{"tag": "forced_sacrifice", "severity": 9, "ref_card": null}]}"#;
        let err = parse_coder_reply(text).unwrap_err();
        assert!(err.contains("severity 9"));
    }

    #[test]
    fn parse_rejects_severity_zero() {
        let text = r#"{"tags": [{"tag": "lack_of_agency", "severity": 0, "ref_card": null}]}"#;
        let err = parse_coder_reply(text).unwrap_err();
        assert!(err.contains("severity 0"));
    }

    #[test]
    fn parse_rejects_non_json() {
        let err = parse_coder_reply("nope").unwrap_err();
        assert!(err.contains("not valid JSON"));
    }

    #[test]
    fn parse_accepts_missing_ref_card_key() {
        // `ref_card` has #[serde(default)] so an absent key defaults
        // to None.
        let text = r#"{"tags": [{"tag": "forced_sacrifice", "severity": 3}]}"#;
        let outcome = parse_coder_reply(text).unwrap();
        assert_eq!(outcome.accepted.len(), 1);
        assert_eq!(outcome.accepted[0].ref_card, None);
    }

    #[test]
    fn taxonomy_has_expected_size_and_no_duplicates() {
        assert_eq!(CODER_TAG_TAXONOMY.len(), 22);
        let mut sorted: Vec<&str> = CODER_TAG_TAXONOMY.to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(sorted.len(), before, "taxonomy must have no duplicates");
    }
}
