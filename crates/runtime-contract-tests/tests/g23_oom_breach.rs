//! G23 — Memory/Quota Breach Cancels Only the Offending Task (Priority P0)
//!
//! Observable: a task whose usage provider exceeds its memory cap is
//! cancelled as a resource breach, while a concurrent within-budget task
//! completes and the shared pool stays usable afterwards.
//!
//! Scope note: this exercises **scheduler-level** quota monitoring through
//! the `WorkerPool` (the production mechanism). The sandbox `OOMContainment`
//! primitive itself is exercised directly by `g26_sandbox_mechanisms`.

use std::sync::Arc;
use std::time::Duration;

use runtime_core::{Scheduler, TaskContext, WorkerPool};
use runtime_interaction::AdapterResult;
use runtime_sandbox::{ResourceQuota, ResourceUsage};
use uuid::Uuid;

#[tokio::test]
async fn memory_breach_cancels_task_and_pool_stays_healthy() {
    let policy = Arc::new(runtime_policy::PolicyEngine::new());
    let pool = Arc::new(WorkerPool::new());

    let breached = Arc::new(std::sync::Mutex::new(Uuid::nil()));
    let breached_provider = breached.clone();
    let provider = Arc::new(move |task_id: Uuid| -> ResourceUsage {
        if *breached_provider.lock().unwrap() == task_id {
            ResourceUsage { memory_bytes: 1_000_000, ..Default::default() }
        } else {
            ResourceUsage::default()
        }
    });

    let scheduler = Scheduler::new(10, 4)
        .with_worker_pool(pool.clone())
        .with_usage_provider(provider);

    let exec = Arc::new(move |ctx: TaskContext| {
        let action = ctx.action.as_str().to_string();
        async move {
            if action == "memory_work" {
                std::future::pending::<()>().await;
            } else {
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            AdapterResult::Success { response: "ok".into(), replay_sequence: 1 }
        }
    });
    let dispatcher = scheduler.start(exec);

    let tight = ResourceQuota { max_memory_bytes: 100, ..Default::default() };

    let breach_ctx =
        TaskContext::with_action(Uuid::new_v4(), None, tight, policy.clone(), "memory_work");
    *breached.lock().unwrap() = breach_ctx.task_id;
    let breach_handle = scheduler.submit(breach_ctx).await.expect("submit breached task");

    let ok_ctx = TaskContext::with_action(Uuid::new_v4(), None, tight, policy.clone(), "ok_work");
    let ok_handle = scheduler.submit(ok_ctx).await.expect("submit unaffected task");

    let breach = tokio::time::timeout(Duration::from_secs(2), breach_handle.result)
        .await
        .expect("breached task must resolve promptly")
        .expect("result channel");
    match breach {
        AdapterResult::Error { message, .. } => assert!(
            message.contains("resource exceeded") && message.contains("memory"),
            "expected a memory-breach cancellation, got: {message}"
        ),
        other => panic!("breached task must be cancelled, got {other:?}"),
    }

    let ok = tokio::time::timeout(Duration::from_secs(2), ok_handle.result)
        .await
        .expect("unaffected task must resolve")
        .expect("result channel");
    assert!(ok.is_success(), "unaffected concurrent task must complete, got {ok:?}");

    let fresh_ctx = TaskContext::with_action(
        Uuid::new_v4(),
        None,
        ResourceQuota::default(),
        policy.clone(),
        "ok_work",
    );
    let fresh_handle = scheduler.submit(fresh_ctx).await.expect("submit fresh task");
    let fresh = tokio::time::timeout(Duration::from_secs(2), fresh_handle.result)
        .await
        .expect("fresh task must resolve")
        .expect("result channel");
    assert!(fresh.is_success(), "pool must stay healthy after a breach, got {fresh:?}");

    drop(dispatcher);
}
