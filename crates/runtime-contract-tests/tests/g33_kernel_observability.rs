//! G33 — Composed Runtime Observability + Caps-Aware Kernel (M1/M5)
//!
//! Observable: a `RuntimeKernel` composed over a policy engine that shares the
//! kernel's observability sink emits a `policy_decision` replay event plus a
//! `policy_decision` metric for a deny; and the kernel's scheduler (wired with
//! observability) emits a `scheduler_queue_depth` gauge under load. The kernel
//! check is capability-aware: an action present in the global allow-list but
//! absent from the agent's `CapabilitySet` is denied, and exactly one decision
//! is logged per check.

use std::sync::Arc;
use std::time::Duration;

use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use runtime_auth::{AgentIdentity, HumanId};
use runtime_core::{RuntimeKernel, TaskContext};
use runtime_interaction::AdapterResult;
use runtime_observability::{ReplayEvent, TraceObservability};
use runtime_policy::{Capability, CapabilitySet, Decision, PolicyEngine, Scope};
use runtime_sandbox::ResourceQuota;
use uuid::Uuid;

#[tokio::test]
async fn composed_runtime_logs_deny_and_emits_queue_gauge() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        recorder.install().expect("install debug recorder");

        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("replay.jsonl");
        let obs = Arc::new(TraceObservability::with_replay(path.clone()));

        // The global allow-list contains the action; the caller injects the
        // observability sink into the policy engine before composing.
        let mut policy = PolicyEngine::new().with_observability(obs.clone());
        policy.add_capability("http.get");
        let policy = Arc::new(policy);

        let kernel = RuntimeKernel::new(policy.clone(), obs.clone());
        let agent = AgentIdentity::new(HumanId(Uuid::new_v4()));

        // M5: allow-listed but not granted in the agent's CapabilitySet -> deny.
        let empty_caps = CapabilitySet::new();
        let denied = kernel.check_capability(&agent, &empty_caps, "http.get");
        assert!(
            matches!(denied, Decision::Deny { .. }),
            "kernel must be caps-aware: allow-listed action absent from CapabilitySet must deny"
        );

        // A granted capability is authorized through the same path.
        let mut caps = CapabilitySet::new();
        caps.grant(Capability::new("http.get", Scope::All, None));
        assert!(
            matches!(kernel.check_capability(&agent, &caps, "http.get"), Decision::Allow),
            "granted capability must be authorized"
        );

        // Exactly one policy_decision replay event per evaluated action.
        let contents = std::fs::read_to_string(&path).expect("replay.jsonl must exist");
        let events: Vec<ReplayEvent> = contents
            .lines()
            .map(|line| serde_json::from_str(line).expect("ReplayEvent json"))
            .collect();
        assert_eq!(events.len(), 2, "one decision logged per check, got {}", events.len());
        assert!(events.iter().all(|e| e.event_type == "policy_decision"));
        assert!(
            events[0].result_summary.starts_with("deny"),
            "first decision must carry the final deny, got {}",
            events[0].result_summary
        );
        assert!(
            events[1].result_summary.starts_with("allow"),
            "second decision must carry the final allow, got {}",
            events[1].result_summary
        );

        // The deny and allow must be reflected in the policy_decision metric
        // (each decision carries its own label, so sum all series).
        let snapshot = snapshotter.snapshot().into_vec();
        let total: u64 = snapshot
            .iter()
            .filter_map(|(key, _, _, value)| {
                if key.key().name() == "policy_decision" {
                    match value {
                        DebugValue::Counter(c) => Some(*c),
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .sum();
        assert!(total >= 2, "policy_decision metric count >= 2, got {total}");

        // The composed scheduler emits scheduler_queue_depth under load.
        let exec = Arc::new(move |_ctx: TaskContext| async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            AdapterResult::Success { response: "ok".into(), replay_sequence: 1 }
        });
        let dispatcher = kernel.scheduler.start(exec);

        let ctx = TaskContext::with_action(
            agent.agent_id.0,
            None,
            ResourceQuota::default(),
            policy.clone(),
            "http.get",
        );
        let handle = kernel.scheduler.submit(ctx).await.expect("submit");

        let snapshot = snapshotter.snapshot().into_vec();
        let gauge_value = snapshot
            .iter()
            .find_map(|(key, _, _, value)| {
                (key.key().name() == "scheduler_queue_depth").then_some(value)
            })
            .expect("scheduler_queue_depth gauge must be emitted under load");
        match gauge_value {
            DebugValue::Gauge(v) => assert!(
                v.into_inner() > 0.0,
                "queue depth gauge must be > 0 during load, got {}",
                v.into_inner()
            ),
            other => panic!("scheduler_queue_depth must be a gauge, got {other:?}"),
        }

        tokio::time::timeout(Duration::from_secs(3), handle.result)
            .await
            .expect("task must resolve")
            .expect("result channel");
        drop(dispatcher);
    })
    .await
    .expect("g33 must not hang");
}
