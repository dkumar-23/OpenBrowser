//! G36 — Unified Quota Semantics: Zero Means Unlimited for Every Dimension (M4)
//!
//! Observable: a mixed quota with one zero dimension (`max_memory_bytes: 0`)
//! and one nonzero dimension does not insta-kill a task even while a usage
//! provider reports memory churn; a nonzero quota dimension is still enforced
//! and hard-cancels a task that exceeds it.

use std::sync::Arc;
use std::time::Duration;

use runtime_core::{Scheduler, TaskContext};
use runtime_interaction::AdapterResult;
use runtime_sandbox::{ResourceQuota, ResourceUsage};
use uuid::Uuid;

#[tokio::test]
async fn mixed_quota_zero_dimension_is_unlimited_and_nonzero_enforced() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let policy = Arc::new(runtime_policy::PolicyEngine::new());

        // A provider that reports large memory usage every tick.
        let provider: Arc<dyn Fn(Uuid) -> ResourceUsage + Send + Sync> =
            Arc::new(|_task_id| ResourceUsage {
                memory_bytes: 10_000_000,
                ..Default::default()
            });

        // Mixed quota: memory unlimited (0), wall budget nonzero.
        let mixed_quota = ResourceQuota {
            max_memory_bytes: 0,
            max_cpu_ms: 0,
            max_wall_ms: 5_000,
            max_network_bytes: 0,
            max_requests: 0,
        };
        let sched = Scheduler::new(10, 2).with_usage_provider(provider.clone());
        let exec = Arc::new(move |_ctx: TaskContext| async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            AdapterResult::Success { response: "ok".into(), replay_sequence: 1 }
        });
        let dispatcher = sched.start(exec);

        let ctx = TaskContext::with_action(
            Uuid::new_v4(),
            None,
            mixed_quota,
            policy.clone(),
            "work",
        );
        let handle = sched.submit(ctx).await.expect("submit");
        let result = tokio::time::timeout(Duration::from_secs(3), handle.result)
            .await
            .expect("mixed-quota task must resolve")
            .expect("result channel");
        assert!(
            result.is_success(),
            "zero memory quota must be unlimited even with usage reported, got {result:?}"
        );
        drop(dispatcher);

        // A nonzero memory quota is still enforced.
        let enforced_quota = ResourceQuota {
            max_memory_bytes: 1_000,
            max_cpu_ms: 0,
            max_wall_ms: 0,
            max_network_bytes: 0,
            max_requests: 0,
        };
        let sched2 = Scheduler::new(10, 2).with_usage_provider(provider);
        let exec2 = Arc::new(move |_ctx: TaskContext| async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            AdapterResult::Success { response: "should_not_finish".into(), replay_sequence: 1 }
        });
        let dispatcher2 = sched2.start(exec2);

        let ctx2 = TaskContext::with_action(
            Uuid::new_v4(),
            None,
            enforced_quota,
            policy.clone(),
            "work",
        );
        let handle2 = sched2.submit(ctx2).await.expect("submit");
        let result2 = tokio::time::timeout(Duration::from_secs(3), handle2.result)
            .await
            .expect("enforced-quota task must resolve by cancellation")
            .expect("result channel");
        assert!(
            !result2.is_success(),
            "usage above a nonzero quota must breach, got {result2:?}"
        );
        drop(dispatcher2);
    })
    .await
    .expect("g36 mixed-quota test must not hang");
}
