//! G22 — Wall-Budget Kill (Priority P0)
//!
//! Observable: a never-finishing task with `max_wall_ms` set is cancelled
//! within its budget (resource-exceeded/timed-out), and the shared
//! `WorkerPool` remains usable for a subsequent task.
//!
//! Scope note: this exercises the **scheduler-level** wall-budget enforcement
//! (the production mechanism). The sandbox `Watchdog` primitive itself is
//! exercised directly by `g26_sandbox_mechanisms`.

use std::sync::Arc;
use std::time::Duration;

use runtime_core::{Scheduler, TaskContext, WorkerPool};
use runtime_interaction::AdapterResult;
use runtime_sandbox::ResourceQuota;
use uuid::Uuid;

#[tokio::test]
async fn wall_budget_kills_long_task_and_pool_survives() {
    let policy = Arc::new(runtime_policy::PolicyEngine::new());
    let pool = Arc::new(WorkerPool::new());
    let scheduler = Scheduler::new(10, 2).with_worker_pool(pool.clone());

    let exec = Arc::new(move |ctx: TaskContext| {
        let action = ctx.action.as_str().to_string();
        async move {
            if action == "slow_work" {
                std::future::pending::<()>().await;
            }
            AdapterResult::Success { response: "ok".into(), replay_sequence: 1 }
        }
    });
    let dispatcher = scheduler.start(exec);

    let quota = ResourceQuota { max_wall_ms: 100, ..Default::default() };
    let ctx = TaskContext::with_action(Uuid::new_v4(), None, quota, policy.clone(), "slow_work");
    let handle = scheduler.submit(ctx).await.expect("submit slow task");

    let result = tokio::time::timeout(Duration::from_secs(2), handle.result)
        .await
        .expect("watchdog budget must fire well within the outer timeout")
        .expect("result channel");
    match &result {
        AdapterResult::Error { message, .. } => assert!(
            message.starts_with("resource exceeded") || message == "deadline exceeded",
            "long task must be killed as resource-exceeded/timed-out, got: {message}"
        ),
        other => panic!("never-finishing task must be cancelled, got {other:?}"),
    }

    let fast = TaskContext::with_action(
        Uuid::new_v4(),
        None,
        ResourceQuota::default(),
        policy.clone(),
        "fast_work",
    );
    let fast_handle = scheduler.submit(fast).await.expect("submit fast task");
    let fast_result = tokio::time::timeout(Duration::from_secs(2), fast_handle.result)
        .await
        .expect("fast task must resolve")
        .expect("result channel");
    assert!(
        fast_result.is_success(),
        "pool must remain usable after a watchdog kill, got {fast_result:?}"
    );

    drop(dispatcher);
}
