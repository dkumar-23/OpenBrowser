//! G21 — Policy Decision Logged (Priority P1)
//!
//! Observable: a deny+allow pair through `PolicyEngine::with_observability`
//! writes `policy_decision` replay events to the single JSONL sequence and
//! increments the `policy_decision` metric. The replay sequence has exactly
//! one source (the observability's `ReplayWriter`).

use std::sync::Arc;

use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use runtime_auth::{AgentIdentity, HumanId};
use runtime_observability::{ReplayEvent, TraceObservability};
use runtime_policy::{Decision, PolicyEngine};
use uuid::Uuid;

#[test]
fn deny_and_allow_pair_is_logged_and_counted() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    recorder.install().expect("install debug recorder");

    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("replay.jsonl");
    let obs = Arc::new(TraceObservability::with_replay(path.clone()));

    let mut policy = PolicyEngine::new().with_observability(obs.clone());
    policy.add_capability("http.get");

    let agent = AgentIdentity::new(HumanId(Uuid::new_v4()));

    assert!(
        matches!(policy.check(&agent, "secret_op"), Decision::Deny { .. }),
        "unknown action must be denied"
    );
    assert!(
        matches!(policy.check(&agent, "http.get"), Decision::Allow),
        "known action must be allowed"
    );

    // The policy_decision metric must have been recorded by the injected sink.
    let snapshot = snapshotter.snapshot().into_vec();
    let counter = snapshot
        .iter()
        .find_map(|(key, _, _, value)| {
            (key.key().name() == "policy_decision").then_some(value)
        })
        .expect("policy_decision metric must be registered");
    match counter {
        DebugValue::Counter(c) => assert!(*c > 0, "policy_decision counter must be > 0, got {c}"),
        other => panic!("policy_decision must be a counter, got {other:?}"),
    }

    // The replay JSONL must contain exactly the two decisions with a
    // monotonically increasing sequence from a single source.
    let contents = std::fs::read_to_string(&path).expect("replay.jsonl must exist");
    let lines: Vec<&str> = contents.lines().collect();
    assert_eq!(lines.len(), 2, "expected 2 policy_decision events, got {}", lines.len());

    let events: Vec<ReplayEvent> = lines
        .iter()
        .map(|line| serde_json::from_str(line).expect("line must be a ReplayEvent"))
        .collect();
    assert_eq!(events[0].event_type, "policy_decision");
    assert_eq!(events[1].event_type, "policy_decision");
    assert!(
        events[1].sequence > events[0].sequence,
        "replay sequence must be monotonic: {} then {}",
        events[0].sequence,
        events[1].sequence
    );
}
