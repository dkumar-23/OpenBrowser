//! G2 — Real Cancellation (P0)
//!
//! Observable: cancel(task_id) stops a long-running task promptly,
//! execution state reaches Cancelled, resources released.

use runtime_core::{Scheduler, TaskContext};
use std::sync::Arc;

#[tokio::test]
async fn cancel_stops_execution() {
    let mut policy = runtime_policy::PolicyEngine::new();
    policy.add_capability("long_sleep");
    let policy = Arc::new(policy);
    let scheduler = Scheduler::new(10, 2);

    let exec = Arc::new(move |_ctx: TaskContext| {
        async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            runtime_interaction::AdapterResult::Success {
                response: "done".into(),
                replay_sequence: 1,
            }
        }
    });

    let dispatcher = scheduler.start(exec);
    let agent = uuid::Uuid::new_v4();
    let ctx = TaskContext::with_action(
        agent,
        None,
        runtime_sandbox::ResourceQuota::default(),
        policy.clone(),
        "long_sleep",
    );
    let handle = scheduler.submit(ctx).await.expect("submit");
    let task_id = handle.task_id;

    // Allow it to reach Running.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Cancel it.
    assert!(scheduler.cancel(task_id), "cancel should return true");

    // The result should resolve promptly, not after 30s.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        handle.result,
    )
    .await;
    assert!(
        result.is_ok(),
        "cancelled task should complete promptly (not 30s)"
    );

    // Observability: execution record should show cancelled state.
    // We verify via metrics that cancelled count incremented.
    let m = scheduler.metrics();
    assert!(m.cancelled >= 1, "cancelled metric should be >= 1");

    drop(dispatcher);
}
