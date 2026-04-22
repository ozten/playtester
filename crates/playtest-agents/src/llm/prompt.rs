//! System and user prompt builders for [`LlmAgent`](super::agent::LlmAgent).
//!
//! The system prompt is structured as three blocks in this exact order so
//! Anthropic's prefix-based prompt cache can tag the first two as
//! ephemeral and reuse them across every turn of the same game:
//!
//! 1. Rules text (cacheable).
//! 2. Card catalog (cacheable).
//! 3. Turn instructions telling the model how to reply (not cacheable —
//!    cheap to re-emit, and this is the block most likely to change
//!    during prompt iteration).
//!
//! The per-turn user message ships the public view, the legal actions
//! slice, and the scratch buffer as a single JSON object.

use std::sync::Arc;

use playtest_ports::SystemBlock;
use serde::Serialize;

use super::scratch::ScratchBuffer;

/// Build the three-block system prompt shared across every turn of a
/// given game.
#[must_use]
pub fn build_system_blocks(rules_text: &Arc<str>, card_catalog: &Arc<str>) -> Vec<SystemBlock> {
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
            text: build_turn_instructions(),
            cache: false,
        },
    ]
}

/// The third system block: the machine-readable reply contract.
///
/// Kept short and specific so the first draft plays legally without
/// prompt-engineering iteration being part of Phase 3's exit criteria.
#[must_use]
pub fn build_turn_instructions() -> String {
    "You are playing a card game. On each turn you will receive the \
public game state, your legal actions (indexed 0..N-1), and your scratch \
buffer.\n\
\n\
Reply with ONLY a single JSON object with exactly these keys:\n\
  \"action_index\": integer, 0-based index into the legal_actions array\n\
  \"plan\": string, your short-term strategic intent\n\
  \"notes\": string, tactical observations to carry forward\n\
\n\
Do not emit any text outside the JSON object. Do not wrap the JSON in \
markdown fences. Choose action_index strictly from the legal_actions \
array you received — do not invent actions."
        .to_owned()
}

/// Build the per-turn user message. Serializes to a single JSON object
/// with keys `public_view`, `legal_actions`, `scratch`.
///
/// # Errors
/// Returns [`serde_json::Error`] if either `view` or `legal` fails to
/// serialize. Both types are expected to be `Serialize` at the call
/// site, so this is only surfaced as an agent error, not a panic.
pub fn build_user_message<V, A>(
    view: &V,
    legal: &[A],
    scratch: &ScratchBuffer,
) -> Result<String, serde_json::Error>
where
    V: Serialize,
    A: Serialize,
{
    let legal_values = legal
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    let payload = serde_json::json!({
        "public_view": serde_json::to_value(view)?,
        "legal_actions": legal_values,
        "scratch": serde_json::to_value(scratch)?,
    });
    serde_json::to_string_pretty(&payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_blocks_pin_cache_flags() {
        let rules: Arc<str> = Arc::from("RULES".to_owned().into_boxed_str());
        let catalog: Arc<str> = Arc::from("CATALOG".to_owned().into_boxed_str());
        let blocks = build_system_blocks(&rules, &catalog);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].text, "RULES");
        assert!(blocks[0].cache);
        assert_eq!(blocks[1].text, "CATALOG");
        assert!(blocks[1].cache);
        assert!(!blocks[2].cache);
        assert!(blocks[2].text.contains("action_index"));
    }

    #[test]
    fn user_message_is_valid_json_with_expected_keys() {
        let scratch = ScratchBuffer::default();
        let legal = vec!["a", "b"];
        let view = serde_json::json!({ "score": [0, 0] });
        let msg = build_user_message(&view, &legal, &scratch).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert!(parsed.get("public_view").is_some());
        assert!(parsed.get("legal_actions").is_some());
        assert!(parsed.get("scratch").is_some());
        assert_eq!(parsed["legal_actions"].as_array().unwrap().len(), 2);
    }
}
