//! G24 — Zero Quota Means Unlimited (Priority P0)
//!
//! Observable: a task carrying an all-zero `ResourceQuota` is not killed by
//! quota enforcement and instead runs to completion.

use std::sync::Arc;
use std::time::Duration;

use runtime_core::{Scheduler, TaskContext};
use runtime_interaction::AdapterResult;
use runtime_sandbox::ResourceQuota;
use uuid::Uuid;

#[tokio::test]
async fn zero_quota_runs_to_completion() {
    let policy = Arc::new(runtime_policy::PolicyEngine::new());
    let scheduler = Scheduler::new(10, 2);

    let exec = Arc::new(move |_ctx: TaskContext| async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        AdapterResult::Success { response: "completed".into(), replay_sequence: 1 }
    });
    let dispatcher = scheduler.start(exec);

    let ctx = TaskContext::with_action(
        Uuid::new_v4(),
        None,
        ResourceQuota::default(),
        policy.clone(),
        "work",
    );
    let handle = scheduler.submit(ctx).await.expect("submit");

    let result = tokio::time::timeout(Duration::from_secs(2), handle.result)
        .await
        .expect("zero-quota task must resolve")
        .expect("result channel");
    assert!(
        result.is_success(),
        "all-zero quota must mean unlimited, got {result:?}"
    );

    drop(dispatcher);
}
