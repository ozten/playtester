//! Concurrent `LlmSidecar::append_call` via `tokio::join!` must produce
//! line-atomic writes. Two `append_call`s with distinct token counts
//! both land on separate lines and parse as valid JSON.

use std::sync::Arc;

use playtest_adapters::ProductionFileSystem;
use playtest_agents::{LlmCallRecord, LlmSidecar, SidecarHeader};
use playtest_ports::FileSystem;
use tempfile::tempdir;
use tokio::sync::Mutex;

#[tokio::test]
async fn concurrent_append_calls_never_tear() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("games/g1.llm.jsonl");

    let fs: Arc<Mutex<dyn FileSystem + Send>> =
        Arc::new(Mutex::new(ProductionFileSystem::new()));
    let sidecar = Arc::new(
        LlmSidecar::new(
            fs.clone(),
            path.clone(),
            SidecarHeader::new("cribbage", 1, "rules", "catalog"),
        )
        .await
        .unwrap(),
    );

    let rec_a = LlmCallRecord {
        tick: 0,
        seat: 0,
        model: "m".into(),
        input_tokens: 111,
        output_tokens: 11,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
        latency_ms: 5,
        chosen_index: Some(0),
        budget_exceeded: false,
    };
    let rec_b = LlmCallRecord {
        tick: 0,
        seat: 1,
        model: "m".into(),
        input_tokens: 222,
        output_tokens: 22,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
        latency_ms: 5,
        chosen_index: Some(3),
        budget_exceeded: false,
    };

    let a = sidecar.clone();
    let b = sidecar.clone();
    let (ra, rb) = tokio::join!(
        async move { a.append_call(&rec_a).await },
        async move { b.append_call(&rec_b).await },
    );
    ra.unwrap();
    rb.unwrap();

    // Read via a fresh read path so the mutex inside the sidecar never
    // hides a partial write. Using a separate FileSystem instance for
    // the assertion side is fine — ProductionFileSystem is effectively
    // stateless.
    let read_fs = ProductionFileSystem::new();
    let bytes = read_fs.read(&path).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.len(),
        3,
        "expected header + two call records, got {lines:?}"
    );

    // Every line parses as a JSON object — no torn writes.
    for (i, line) in lines.iter().enumerate() {
        let v: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("line {i} not JSON: {e}: {line}"));
        assert!(v.is_object(), "line {i} not an object: {line}");
    }

    // Both records landed. Order is non-deterministic under tokio::join!,
    // so check input_tokens as the distinguishing fingerprint.
    let tokens: Vec<u64> = lines[1..]
        .iter()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["input_tokens"].as_u64().unwrap()
        })
        .collect();
    assert!(tokens.contains(&111));
    assert!(tokens.contains(&222));
}
