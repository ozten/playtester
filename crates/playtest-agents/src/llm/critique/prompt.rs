//! Critique-time prompt builders — system blocks + user message.
//!
//! Mirrors `crate::llm::prompt` but for the post-game questionnaire.
//! The first two system blocks (rules, card catalog) are byte-identical
//! to the gameplay pass so Anthropic's prefix cache stays warm for the
//! last sub-second hop; only the third block (instructions) is
//! critique-specific and uncached.

use std::sync::Arc;

use playtest_core::GameResult;
use playtest_ports::SystemBlock;
use serde::Serialize;

use super::spec::QuestionnaireSpec;
use crate::llm::scratch::ScratchBuffer;

/// Build the three-block system prompt used for the post-game
/// questionnaire call.
///
/// Blocks 0 and 1 are byte-identical to `build_system_blocks` from
/// gameplay so the prefix cache hits. Block 2 is critique-specific —
/// it tells the model the reply contract and enumerates the keys the
/// caller expects to see.
#[must_use]
pub fn build_critique_system_blocks(
    rules_text: &Arc<str>,
    card_catalog: &Arc<str>,
    spec: &QuestionnaireSpec,
) -> Vec<SystemBlock> {
    vec![
        SystemBlock {
            text: rules_text.as_ref().to_owned(),
            cache: true,
        },
        SystemBlock {
            text: card_catalog.as_ref().to_owned(),
            cache: true,
        },
        SystemBlock {
            text: build_critique_instructions(spec),
            cache: false,
        },
    ]
}

/// The third system block: reply contract for the questionnaire.
///
/// Lists the expected Likert keys and open-ended keys explicitly so the
/// model cannot silently rename them. Values are bounded to integers
/// 1-5 for Likert, arbitrary strings for open-ended.
#[must_use]
pub fn build_critique_instructions(spec: &QuestionnaireSpec) -> String {
    use core::fmt::Write as _;
    let mut s = String::new();
    s.push_str(
        "You have just finished playing a card game. Answer the \
post-game questionnaire honestly, from the perspective of the player \
you just were — not as a neutral reviewer.\n\
\n\
Reply with ONLY a single JSON object with exactly these keys:\n\
  \"likert\": object — map each of the keys below to an integer 1-5, \
where 1 = strongly disagree and 5 = strongly agree.\n\
  \"open_ended\": object — map each of the prompt keys below to a \
string of at most ~500 characters.\n\
\n\
Likert keys (all required):\n",
    );
    for item in &spec.items {
        let _ = writeln!(s, "  - \"{}\" — {}", item.id, item.text);
    }
    s.push_str("\nOpen-ended keys (all required):\n");
    for prompt in &spec.open_ended {
        let _ = writeln!(s, "  - \"{}\" — {}", prompt.id, prompt.text);
    }
    s.push_str(
        "\nDo not emit any text outside the JSON object. Do not wrap \
the JSON in markdown fences. Do not invent keys; use only the keys \
listed above. Integer Likert values must be in the range 1-5 \
inclusive.",
    );
    s
}

/// Build the per-game critique user message. Serializes to a single
/// JSON object with keys `final_public_view`, `game_result`, `scratch`,
/// `persona_addendum` (if any), and no reply placeholder — the reply
/// contract is in the system instructions.
///
/// # Errors
/// Returns [`serde_json::Error`] if `final_view` fails to serialize.
pub fn build_critique_user_message<V>(
    final_view: &V,
    result: &GameResult,
    scratch: &ScratchBuffer,
    persona_addendum: Option<&str>,
) -> Result<String, serde_json::Error>
where
    V: Serialize,
{
    let mut payload = serde_json::json!({
        "final_public_view": serde_json::to_value(final_view)?,
        "game_result": serde_json::to_value(result)?,
        "scratch": serde_json::to_value(scratch)?,
    });
    if let Some(addendum) = persona_addendum {
        payload
            .as_object_mut()
            .expect("just built a json object")
            .insert(
                "persona_addendum".to_owned(),
                serde_json::Value::String(addendum.to_owned()),
            );
    }
    serde_json::to_string_pretty(&payload)
}

#[cfg(test)]
mod tests {
    use super::super::spec::default_questionnaire_v1;
    use super::*;
    use playtest_core::EndReason;

    #[test]
    fn system_blocks_preserve_cache_flags_and_rules_bytes() {
        let rules: Arc<str> = Arc::from("RULES".to_owned().into_boxed_str());
        let catalog: Arc<str> = Arc::from("CATALOG".to_owned().into_boxed_str());
        let spec = default_questionnaire_v1();
        let blocks = build_critique_system_blocks(&rules, &catalog, &spec);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].text, "RULES");
        assert!(blocks[0].cache, "rules block must be cacheable");
        assert_eq!(blocks[1].text, "CATALOG");
        assert!(blocks[1].cache, "card catalog block must be cacheable");
        assert!(!blocks[2].cache, "instructions block must not be cached");
    }

    #[test]
    fn instructions_list_every_likert_and_open_ended_id() {
        let spec = default_questionnaire_v1();
        let instr = build_critique_instructions(&spec);
        for item in &spec.items {
            assert!(
                instr.contains(&format!("\"{}\"", item.id)),
                "instructions missing Likert id `{}`",
                item.id
            );
        }
        for prompt in &spec.open_ended {
            assert!(
                instr.contains(&format!("\"{}\"", prompt.id)),
                "instructions missing open-ended id `{}`",
                prompt.id
            );
        }
        assert!(
            instr.contains("1-5"),
            "instructions must communicate the 1-5 Likert scale"
        );
    }

    fn sample_result() -> GameResult {
        GameResult {
            winner: Some(0_u8),
            reason: EndReason::Victory,
            scores: vec![121, 97],
        }
    }

    #[test]
    fn user_message_is_valid_json_with_expected_keys() {
        let scratch = ScratchBuffer::default();
        let final_view = serde_json::json!({ "turn": 42 });
        let result = sample_result();
        let msg = build_critique_user_message(&final_view, &result, &scratch, None).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert!(parsed.get("final_public_view").is_some());
        assert!(parsed.get("game_result").is_some());
        assert!(parsed.get("scratch").is_some());
        assert!(
            parsed.get("persona_addendum").is_none(),
            "persona_addendum must be absent when None is passed"
        );
    }

    #[test]
    fn user_message_includes_persona_addendum_when_provided() {
        let scratch = ScratchBuffer::default();
        let final_view = serde_json::json!({});
        let result = sample_result();
        let msg = build_critique_user_message(
            &final_view,
            &result,
            &scratch,
            Some("You are an aggressive player."),
        )
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(
            parsed["persona_addendum"],
            "You are an aggressive player."
        );
    }

    #[test]
    fn user_message_embeds_scratch_contents() {
        let mut scratch = ScratchBuffer {
            plan: "keep resources".into(),
            notes: "opponent hoards food".into(),
            ..ScratchBuffer::default()
        };
        scratch.push_turn_log("tick=0 played card".into());
        let msg = build_critique_user_message(
            &serde_json::json!({}),
            &sample_result(),
            &scratch,
            None,
        )
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["scratch"]["plan"], "keep resources");
        assert_eq!(parsed["scratch"]["notes"], "opponent hoards food");
        assert_eq!(
            parsed["scratch"]["turn_log"].as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn user_message_embeds_game_result() {
        let msg = build_critique_user_message(
            &serde_json::json!({}),
            &sample_result(),
            &ScratchBuffer::default(),
            None,
        )
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["game_result"]["winner"], 0);
        assert_eq!(parsed["game_result"]["reason"], "Victory");
        assert_eq!(parsed["game_result"]["scores"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn instructions_bytes_change_when_spec_changes() {
        let spec_a = default_questionnaire_v1();
        let mut spec_b = default_questionnaire_v1();
        spec_b.items[0].text = "different wording";
        assert_ne!(
            build_critique_instructions(&spec_a),
            build_critique_instructions(&spec_b)
        );
    }
}
