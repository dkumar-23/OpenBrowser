//! G1 — Concurrent Execution (P0)
//!
//! Observable contract: two concurrent tasks submitted to the scheduler
//! should overlap in time (elapsed < 700ms for two 500ms sleeps),
//! and both should return Ok results.

use runtime_core::{Scheduler, TaskContext};
// PolicyEngine used directly
use std::sync::Arc;

#[tokio::test]
async fn concurrent_execution_2_tasks_overlap() {
    let mut policy = runtime_policy::PolicyEngine::new();
    policy.add_capability("sleep_500");
    let policy = Arc::new(policy);
    // Note: using a mock executor that simulates concurrent work.
    // This measures actual overlap, not method existence.
    let scheduler = Scheduler::new(10, 2);

    // Mock executor that sleeps 500ms then returns Success.
    let exec = Arc::new(move |_ctx: TaskContext| {
        async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            runtime_interaction::AdapterResult::Success {
                response: "sleep_done".into(),
                replay_sequence: 1,
            }
        }
    });

    let dispatcher = scheduler.start(exec);

    let start = std::time::Instant::now();

    let agent_id = uuid::Uuid::new_v4();
    let ctx1 = TaskContext::with_action(
        agent_id,
        None,
        runtime_sandbox::ResourceQuota::default(),
        policy.clone(),
        "sleep_500",
    );
    let ctx2 = TaskContext::with_action(
        agent_id,
        None,
        runtime_sandbox::ResourceQuota::default(),
        policy.clone(),
        "sleep_500",
    );

    let h1 = scheduler.submit(ctx1).await.expect("submit 1");
    let h2 = scheduler.submit(ctx2).await.expect("submit 2");

    let (r1, r2) = tokio::join!(
        async { h1.result.await.ok() },
        async { h2.result.await.ok() }
    );

    let elapsed = start.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(700),
        "tasks should overlap; elapsed={}ms",
        elapsed.as_millis()
    );
    assert!(r1.is_some() && r2.is_some(), "both tasks should complete");

    // Keep dispatcher alive until we are done inspecting results.
    drop(dispatcher);
}
