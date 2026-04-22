//! Integration tests for the record/playback discipline.
//!
//! These exercise the full `Record<P> -> tape file -> Playback<P>`
//! pipeline through the public API, the way real engine code will use
//! the adapters in later units.

use std::path::Path;

use playtest_adapters::{
    PlaybackClock, PlaybackFileSystem, PlaybackLlmClient, PlaybackRng, ProductionClock,
    ProductionLlmClient, ProductionRng, RecordClock, RecordFileSystem, RecordLlmClient, RecordRng,
    StubFileSystem, StubLlmClient,
};
use playtest_ports::{
    ChatMessage, ChatRole, Clock, FileSystem, LlmClient, LlmRequest, Rng, SystemBlock,
};
use tempfile::tempdir;

/// Tiny "fake game" that drives a Clock and an Rng in a fixed pattern.
/// Returning the observable trace lets us assert the playback run produces
/// the same sequence of values without any per-call boilerplate.
fn run_fake_game(clock: &mut dyn Clock, rng: &mut dyn Rng) -> Vec<(u64, u64)> {
    let mut trace = Vec::new();
    for _ in 0..5 {
        let t = clock.now();
        let r = rng.gen_range(0..52).unwrap();
        trace.push((t, r));
    }
    let final_u64 = rng.next_u64();
    trace.push((clock.now(), final_u64));
    trace
}

#[test]
fn record_then_playback_reproduces_clock_and_rng_outputs_bit_for_bit() {
    let dir = tempdir().unwrap();
    let clock_tape = dir.path().join("clock.jsonl");
    let rng_tape = dir.path().join("rng.jsonl");

    let recorded_trace = {
        let mut record_clock = RecordClock::create(ProductionClock::new(), &clock_tape).unwrap();
        let mut record_rng = RecordRng::create(ProductionRng::from_seed(12345), &rng_tape).unwrap();
        let trace = run_fake_game(&mut record_clock, &mut record_rng);
        record_clock.flush().unwrap();
        record_rng.flush().unwrap();
        trace
    };

    let mut playback_clock = PlaybackClock::open(&clock_tape).unwrap();
    let mut playback_rng = PlaybackRng::open(&rng_tape).unwrap();
    let replayed_trace = run_fake_game(&mut playback_clock, &mut playback_rng);

    assert_eq!(recorded_trace, replayed_trace);
    assert_eq!(playback_clock.remaining(), 0);
    assert_eq!(playback_rng.remaining(), 0);
}

#[test]
fn filesystem_record_then_playback_replays_reads_and_writes() {
    let dir = tempdir().unwrap();
    let tape = dir.path().join("fs.jsonl");

    let recorded = {
        let inner = StubFileSystem::new();
        let mut record_fs = RecordFileSystem::create(inner, &tape).unwrap();

        record_fs.write(Path::new("/a.txt"), b"alpha").unwrap();
        record_fs.append_line(Path::new("/log"), "first").unwrap();
        record_fs.append_line(Path::new("/log"), "second").unwrap();
        let read_back = record_fs.read(Path::new("/a.txt")).unwrap();
        let log_exists = record_fs.exists(Path::new("/log"));
        let missing_exists = record_fs.exists(Path::new("/missing"));

        record_fs.flush().unwrap();
        (read_back, log_exists, missing_exists)
    };

    let mut playback_fs = PlaybackFileSystem::open(&tape).unwrap();
    playback_fs.write(Path::new("/a.txt"), b"alpha").unwrap();
    playback_fs.append_line(Path::new("/log"), "first").unwrap();
    playback_fs
        .append_line(Path::new("/log"), "second")
        .unwrap();
    let replayed_read = playback_fs.read(Path::new("/a.txt")).unwrap();
    let replayed_log_exists = playback_fs.exists(Path::new("/log"));
    let replayed_missing_exists = playback_fs.exists(Path::new("/missing"));

    assert_eq!(recorded.0, replayed_read);
    assert_eq!(recorded.1, replayed_log_exists);
    assert_eq!(recorded.2, replayed_missing_exists);
    assert_eq!(playback_fs.remaining(), 0);
}

#[test]
#[should_panic(expected = "tape exhausted")]
fn playback_beyond_tape_end_panics_with_call_index() {
    let dir = tempdir().unwrap();
    let tape = dir.path().join("rng.jsonl");

    {
        let mut record_rng = RecordRng::create(ProductionRng::from_seed(7), &tape).unwrap();
        let _ = record_rng.next_u64();
        record_rng.flush().unwrap();
    }

    let mut playback_rng = PlaybackRng::open(&tape).unwrap();
    let _ = playback_rng.next_u64();
    let _ = playback_rng.next_u64(); // one past end
}

#[test]
fn playback_against_wrong_port_tape_is_rejected_at_open() {
    let dir = tempdir().unwrap();
    let rng_tape = dir.path().join("rng.jsonl");
    {
        let mut record_rng = RecordRng::create(ProductionRng::from_seed(1), &rng_tape).unwrap();
        let _ = record_rng.next_u64();
        record_rng.flush().unwrap();
    }

    let err = PlaybackClock::open(&rng_tape).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("port"),
        "expected port-mismatch message, got: {msg}"
    );
}

#[tokio::test]
async fn llm_client_record_then_playback_replays_responses() {
    let dir = tempdir().unwrap();
    let tape = dir.path().join("llm.jsonl");

    let req = LlmRequest {
        system_blocks: vec![SystemBlock {
            text: "you are a test".into(),
            cache: true,
        }],
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "say hi".into(),
        }],
        model: "claude-test".into(),
        max_tokens: 16,
        temperature: Some(0.7),
    };

    let recorded = {
        let inner = StubLlmClient::new("hi back")
            .with_token_counts(4, 2)
            .with_cache_tokens(5, 1);
        let mut record_llm = RecordLlmClient::create(inner, &tape).unwrap();
        let resp = record_llm.complete(req.clone()).await.unwrap();
        let prod_inner = ProductionLlmClient::not_configured();
        let mut record_prod =
            RecordLlmClient::create(prod_inner, dir.path().join("llm2.jsonl")).unwrap();
        let prod_err = record_prod.complete(req.clone()).await.unwrap_err();
        record_llm.flush().unwrap();
        record_prod.flush().unwrap();
        (resp, prod_err)
    };

    let playback = PlaybackLlmClient::open(&tape).unwrap();
    let replayed = playback.complete(req.clone()).await.unwrap();

    assert_eq!(recorded.0.text, replayed.text);
    assert_eq!(recorded.0.input_tokens, replayed.input_tokens);
    assert_eq!(recorded.0.output_tokens, replayed.output_tokens);
    assert_eq!(
        recorded.0.cache_read_input_tokens,
        replayed.cache_read_input_tokens
    );
    assert_eq!(
        recorded.0.cache_creation_input_tokens,
        replayed.cache_creation_input_tokens
    );
    assert_eq!(replayed.cache_read_input_tokens, 5);
    assert_eq!(replayed.cache_creation_input_tokens, 1);
    assert!(matches!(
        recorded.1,
        playtest_ports::LlmError::NotConfigured
    ));
}

#[tokio::test]
async fn llm_client_playback_with_diverging_request_returns_tape_divergence() {
    let dir = tempdir().unwrap();
    let tape = dir.path().join("llm.jsonl");

    {
        let inner = StubLlmClient::new("ok");
        let mut record_llm = RecordLlmClient::create(inner, &tape).unwrap();
        let _ = record_llm
            .complete(LlmRequest {
                system_blocks: vec![],
                messages: vec![ChatMessage {
                    role: ChatRole::User,
                    content: "first".into(),
                }],
                model: "claude-test".into(),
                max_tokens: 8,
                temperature: None,
            })
            .await
            .unwrap();
        record_llm.flush().unwrap();
    }

    let playback = PlaybackLlmClient::open(&tape).unwrap();
    let err = playback
        .complete(LlmRequest {
            system_blocks: vec![],
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "different".into(),
            }],
            model: "claude-test".into(),
            max_tokens: 8,
            temperature: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, playtest_ports::LlmError::TapeDivergence));
}
