//! G29 — Global Quota, Backpressure, and Queue-Depth Metric (Priority P0)
//!
//! Observable: a `GlobalCapacity` shared by two worker sets rejects a third
//! in-flight task with `BackpressureError` within a bounded time, then accepts
//! again once the system drains; the scheduler reports queue depth as a real
//! `metrics` gauge (> 0) while tasks are queued.

use std::sync::Arc;
use std::time::Duration;

use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use runtime_core::{GlobalCapacity, Scheduler, TaskContext};
use runtime_interaction::AdapterResult;
use runtime_observability::{Observability, TraceObservability};
use runtime_sandbox::ResourceQuota;
use uuid::Uuid;

fn ctx(policy: &Arc<runtime_policy::PolicyEngine>) -> TaskContext {
    TaskContext::with_action(
        Uuid::new_v4(),
        None,
        ResourceQuota::default(),
        policy.clone(),
        "work",
    )
}

#[tokio::test]
async fn global_capacity_rejects_then_recovers_and_queue_gauge_reports() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        recorder.install().expect("install debug recorder");

        let policy = Arc::new(runtime_policy::PolicyEngine::new());

        // Phase 1 — queue depth gauge under load (single worker set, slot=1).
        let obs: Arc<dyn Observability> = Arc::new(TraceObservability::default());
        let slow = Arc::new(move |_ctx: TaskContext| async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            AdapterResult::Success { response: "ok".into(), replay_sequence: 1 }
        });
        let sched_c = Scheduler::new(100, 1).with_observability(obs.clone());
        let dispatcher_c = sched_c.start(slow.clone());

        let mut queued_handles = Vec::new();
        for _ in 0..5 {
            queued_handles.push(sched_c.submit(ctx(&policy)).await.expect("submit queued"));
        }

        let snapshot = snapshotter.snapshot().into_vec();
        let gauge = snapshot
            .iter()
            .find_map(|(key, _, _, value)| {
                (key.key().name() == "scheduler_queue_depth").then_some(value)
            })
            .expect("scheduler_queue_depth gauge must be reported");
        match gauge {
            DebugValue::Gauge(v) => assert!(
                v.into_inner() > 0.0,
                "queue depth gauge must be > 0 during load, got {}",
                v.into_inner()
            ),
            other => panic!("scheduler_queue_depth must be a gauge, got {other:?}"),
        }

        for handle in queued_handles {
            tokio::time::timeout(Duration::from_secs(3), handle.result)
                .await
                .expect("queued task must resolve")
                .expect("result channel");
        }
        drop(dispatcher_c);

        // Phase 2 — global capacity shared by two worker sets.
        let capacity = Arc::new(GlobalCapacity::new(2));
        let long = Arc::new(move |_ctx: TaskContext| async move {
            tokio::time::sleep(Duration::from_millis(250)).await;
            AdapterResult::Success { response: "ok".into(), replay_sequence: 1 }
        });
        let sched_a = Scheduler::new(100, 4).with_global_capacity(capacity.clone());
        let sched_b = Scheduler::new(100, 4).with_global_capacity(capacity.clone());
        let dispatcher_a = sched_a.start(long.clone());
        let dispatcher_b = sched_b.start(long.clone());

        let h1 = sched_a.submit(ctx(&policy)).await.expect("submit to set A");
        let h2 = sched_b.submit(ctx(&policy)).await.expect("submit to set B");

        let rejected = tokio::time::timeout(
            Duration::from_millis(500),
            sched_a.try_submit(ctx(&policy)),
        )
        .await
        .expect("backpressure must be reported in bounded time, not hang");
        assert!(
            rejected.is_err(),
            "submitting beyond global capacity must return BackpressureError"
        );

        for handle in [h1, h2] {
            tokio::time::timeout(Duration::from_secs(3), handle.result)
                .await
                .expect("in-flight task must resolve")
                .expect("result channel");
        }

        // The system must drain and accept again (no deadlock).
        let mut accepted = None;
        for _ in 0..100 {
            if let Ok(handle) = sched_a.try_submit(ctx(&policy)).await {
                accepted = Some(handle);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let recovered = accepted.expect("global capacity must recover after drain");
        let result = tokio::time::timeout(Duration::from_secs(3), recovered.result)
            .await
            .expect("recovered task must resolve")
            .expect("result channel");
        assert!(
            result.is_success(),
            "recovered submission must complete successfully"
        );

        drop(dispatcher_a);
        drop(dispatcher_b);
    })
    .await
    .expect("global-quota test must not hang");
}
