//! G34 — Exactly One Decision Logged per Evaluated Action (M2)
//!
//! Observable: `check_with_caps` writes exactly one `policy_decision` replay
//! event per evaluated action, and that event carries the FINAL decision
//! (base evaluation combined with the per-agent scope check), not an
//! intermediate allow.

use std::sync::Arc;

use metrics_util::debugging::DebuggingRecorder;
use runtime_auth::{AgentIdentity, HumanId};
use runtime_observability::{ReplayEvent, TraceObservability};
use runtime_policy::{Capability, CapabilitySet, Decision, PolicyEngine, Scope};
use uuid::Uuid;

#[test]
fn check_with_caps_logs_exactly_one_final_decision_per_action() {
    let recorder = DebuggingRecorder::new();
    recorder.install().expect("install debug recorder");

    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("replay.jsonl");
    let obs = Arc::new(TraceObservability::with_replay(path.clone()));

    let mut policy = PolicyEngine::new().with_observability(obs.clone());
    policy.add_capability("http.get");

    let agent = AgentIdentity::new(HumanId(Uuid::new_v4()));
    let empty_caps = CapabilitySet::new();

    // Base allows but the agent scope check escalates to deny: one event.
    let denied = policy.check_with_caps(&agent, &empty_caps, "http.get");
    assert!(matches!(denied, Decision::Deny { .. }));

    // Granted capability: one allow event.
    let mut caps = CapabilitySet::new();
    caps.grant(Capability::new("http.get", Scope::All, None));
    let allowed = policy.check_with_caps(&agent, &caps, "http.get");
    assert!(matches!(allowed, Decision::Allow));

    // Base deny (unknown action): one deny event, no capability escalation.
    let unknown = policy.check_with_caps(&agent, &empty_caps, "secret_op");
    assert!(matches!(unknown, Decision::Deny { .. }));

    let contents = std::fs::read_to_string(&path).expect("replay.jsonl must exist");
    let events: Vec<ReplayEvent> = contents
        .lines()
        .map(|line| serde_json::from_str(line).expect("ReplayEvent json"))
        .collect();

    assert_eq!(
        events.len(),
        3,
        "exactly one policy_decision event per evaluated action, got {}",
        events.len()
    );
    assert!(events.iter().all(|e| e.event_type == "policy_decision"));
    assert!(
        events[0].result_summary.starts_with("deny"),
        "scope-escalated deny must be the logged final decision, got {}",
        events[0].result_summary
    );
    assert!(
        events[1].result_summary.starts_with("allow"),
        "granted action must log allow, got {}",
        events[1].result_summary
    );
    assert!(
        events[2].result_summary.starts_with("deny"),
        "base deny must log deny, got {}",
        events[2].result_summary
    );
}
