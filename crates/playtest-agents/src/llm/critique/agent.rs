//! The core critique call — one LLM completion producing a validated
//! `QuestionnaireResponseRecord`, with one retry on parse failure.
//!
//! This module has no I/O: it never touches a `CritiqueSidecar`. The
//! caller (`LlmAgent::post_game_critique`) owns the sidecar append.
//! Keeping the I/O external makes this function testable against a
//! stub `LlmClient` without any filesystem setup.
//!
//! The retry discipline mirrors `LlmAgent::choose` exactly: on the
//! first parse failure, reassemble the conversation with the bad
//! reply as an Assistant turn and a User-turn reminder of the format,
//! then call the LLM once more. A second parse failure surfaces as
//! `AgentError::Other` and the caller decides whether to log-and-skip
//! or propagate.

use std::collections::BTreeMap;

use playtest_core::{AgentError, GameResult, PlayerId};
use playtest_ports::{ChatMessage, ChatRole, LlmError, LlmRequest};
use serde::{Deserialize, Serialize};

use super::prompt::{build_critique_system_blocks, build_critique_user_message};
use super::sidecar::QuestionnaireResponseRecord;
use super::spec::QuestionnaireSpec;
use crate::llm::agent::LlmAgentConfig;
use crate::llm::scratch::ScratchBuffer;

/// Shape the model must reply with — mirrors `build_critique_instructions`.
#[derive(Debug, Deserialize, Serialize)]
struct CritiqueReply {
    likert: BTreeMap<String, u8>,
    open_ended: BTreeMap<String, String>,
}

/// Issue one critique LLM call for `seat`, with one retry on parse
/// failure. Returns the validated record ready for sidecar append.
///
/// # Errors
/// Returns `AgentError::Other` for LLM transport failures, budget
/// exhaustion, and parse failures that persisted through one retry.
pub async fn critique_once<V: Serialize>(
    cfg: &LlmAgentConfig,
    seat: PlayerId,
    view: &V,
    result: &GameResult,
    scratch: &ScratchBuffer,
    spec: &QuestionnaireSpec,
    persona_addendum: Option<&str>,
) -> Result<QuestionnaireResponseRecord, AgentError> {
    let system_blocks =
        build_critique_system_blocks(&cfg.rules_text, &cfg.card_catalog, spec);
    let user_body = build_critique_user_message(view, result, scratch, persona_addendum)
        .map_err(|e| AgentError::Other(format!("serialize critique user message: {e}")))?;

    let mut messages: Vec<ChatMessage> = vec![ChatMessage {
        role: ChatRole::User,
        content: user_body,
    }];

    // First attempt.
    let resp = match cfg
        .llm
        .complete(LlmRequest {
            system_blocks: system_blocks.clone(),
            messages: messages.clone(),
            model: cfg.model.clone(),
            max_tokens: cfg.max_tokens,
            temperature: cfg.temperature,
        })
        .await
    {
        Ok(r) => r,
        Err(LlmError::BudgetExceeded {
            requested,
            remaining,
        }) => {
            return Err(AgentError::Other(format!(
                "critique budget exceeded: requested {requested}, remaining {remaining}"
            )));
        }
        Err(e) => return Err(AgentError::Other(format!("critique llm call failed: {e}"))),
    };

    // Happy-path parse.
    let first_err = match parse_and_validate(&resp.text, spec) {
        Ok(reply) => return Ok(into_record(seat, spec, reply)),
        Err(e) => e,
    };
    // Retry once with an augmented conversation.
    messages.push(ChatMessage {
        role: ChatRole::Assistant,
        content: resp.text,
    });
    messages.push(ChatMessage {
        role: ChatRole::User,
        content: format!(
            "Your previous reply was not the expected JSON shape ({first_err}). \
Please respond with only a single JSON object with keys `likert` and `open_ended`, \
using only the keys specified in the system instructions. Integer Likert values must be 1-5."
        ),
    });

    let resp2 = match cfg
        .llm
        .complete(LlmRequest {
            system_blocks,
            messages,
            model: cfg.model.clone(),
            max_tokens: cfg.max_tokens,
            temperature: cfg.temperature,
        })
        .await
    {
        Ok(r) => r,
        Err(LlmError::BudgetExceeded {
            requested,
            remaining,
        }) => {
            return Err(AgentError::Other(format!(
                "critique budget exceeded during retry: requested {requested}, remaining {remaining}"
            )));
        }
        Err(e) => {
            return Err(AgentError::Other(format!(
                "critique retry call failed: {e}"
            )));
        }
    };

    match parse_and_validate(&resp2.text, spec) {
        Ok(reply) => Ok(into_record(seat, spec, reply)),
        Err(second_err) => Err(AgentError::Other(format!(
            "critique parse failed after retry: {first_err}; retry error: {second_err}"
        ))),
    }
}

/// Validate the reply matches the spec: every declared Likert key
/// present with a value in 1..=5, every declared open-ended key
/// present (empty strings allowed).
fn parse_and_validate(text: &str, spec: &QuestionnaireSpec) -> Result<CritiqueReply, String> {
    let reply: CritiqueReply =
        serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;

    for (key, value) in &reply.likert {
        if !(1..=5).contains(value) {
            return Err(format!(
                "Likert value for `{key}` is {value}, must be in 1..=5"
            ));
        }
    }
    for item in &spec.items {
        if !reply.likert.contains_key(item.id) {
            return Err(format!("missing required Likert key `{}`", item.id));
        }
    }
    for prompt in &spec.open_ended {
        if !reply.open_ended.contains_key(prompt.id) {
            return Err(format!(
                "missing required open-ended key `{}`",
                prompt.id
            ));
        }
    }
    Ok(reply)
}

fn into_record(
    seat: PlayerId,
    spec: &QuestionnaireSpec,
    reply: CritiqueReply,
) -> QuestionnaireResponseRecord {
    QuestionnaireResponseRecord::new(seat, spec.version, reply.likert, reply.open_ended)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::critique::spec::default_questionnaire_v1;

    fn make_spec_reply_text(extra_likert: Option<(&str, i32)>) -> String {
        let mut likert = serde_json::Map::new();
        for key in [
            "agency",
            "fairness",
            "tension",
            "pacing",
            "variety",
            "frustration",
            "satisfaction",
            "would_play_again",
        ] {
            likert.insert(key.into(), serde_json::json!(4));
        }
        if let Some((k, v)) = extra_likert {
            likert.insert(k.into(), serde_json::json!(v));
        }
        let mut open = serde_json::Map::new();
        open.insert("worst_moment".into(), serde_json::json!("it was fine"));
        open.insert(
            "what_would_you_change".into(),
            serde_json::json!("nothing major"),
        );
        serde_json::Value::Object(
            [
                ("likert".to_owned(), serde_json::Value::Object(likert)),
                ("open_ended".to_owned(), serde_json::Value::Object(open)),
            ]
            .into_iter()
            .collect(),
        )
        .to_string()
    }

    #[test]
    fn parse_and_validate_accepts_well_formed_reply() {
        let spec = default_questionnaire_v1();
        let text = make_spec_reply_text(None);
        let reply = parse_and_validate(&text, &spec).unwrap();
        assert_eq!(reply.likert.len(), 8);
        assert_eq!(reply.open_ended.len(), 2);
    }

    #[test]
    fn parse_and_validate_rejects_out_of_range_likert() {
        let spec = default_questionnaire_v1();
        // agency = 6 overrides the 4 from the helper
        let text = make_spec_reply_text(Some(("agency", 6)));
        let err = parse_and_validate(&text, &spec).unwrap_err();
        assert!(err.contains("agency"));
        assert!(err.contains("1..=5"));
    }

    #[test]
    fn parse_and_validate_rejects_zero_likert() {
        let spec = default_questionnaire_v1();
        let text = make_spec_reply_text(Some(("fairness", 0)));
        let err = parse_and_validate(&text, &spec).unwrap_err();
        assert!(err.contains("fairness"));
    }

    #[test]
    fn parse_and_validate_rejects_missing_likert_key() {
        let spec = default_questionnaire_v1();
        // Build reply that's missing `variety`.
        let text = r#"{
            "likert": {"agency": 3, "fairness": 3, "tension": 3, "pacing": 3,
                       "frustration": 3, "satisfaction": 3, "would_play_again": 3},
            "open_ended": {"worst_moment": "x", "what_would_you_change": "y"}
        }"#;
        let err = parse_and_validate(text, &spec).unwrap_err();
        assert!(err.contains("variety"), "expected variety missing; got: {err}");
    }

    #[test]
    fn parse_and_validate_rejects_missing_open_ended_key() {
        let spec = default_questionnaire_v1();
        let text = r#"{
            "likert": {"agency": 3, "fairness": 3, "tension": 3, "pacing": 3,
                       "variety": 3, "frustration": 3, "satisfaction": 3, "would_play_again": 3},
            "open_ended": {"worst_moment": "x"}
        }"#;
        let err = parse_and_validate(text, &spec).unwrap_err();
        assert!(err.contains("what_would_you_change"));
    }

    #[test]
    fn parse_and_validate_rejects_non_json() {
        let spec = default_questionnaire_v1();
        let err = parse_and_validate("I think the game was fine", &spec).unwrap_err();
        assert!(err.contains("invalid JSON"));
    }
}
