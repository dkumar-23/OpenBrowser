//! G5 — Deadline Terminates (P0)
//!
//! Observable: task with 100ms deadline and 500ms work should return
//! timeout/error and reach TimedOut state.

use runtime_core::{Scheduler, TaskContext};

use std::sync::Arc;

#[tokio::test]
async fn deadline_terminates_execution() {
    let mut policy = runtime_policy::PolicyEngine::new();
    policy.add_capability("slow_work");
    let policy = Arc::new(policy);
    let scheduler = Scheduler::new(10, 2);

    let exec = Arc::new(move |_ctx: TaskContext| {
        async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            runtime_interaction::AdapterResult::Success {
                response: "done".into(),
                replay_sequence: 1,
            }
        }
    });

    let dispatcher = scheduler.start(exec);
    let agent = uuid::Uuid::new_v4();

    let ctx = TaskContext::with_deadline(
        agent,
        None,
        runtime_sandbox::ResourceQuota::default(),
        policy.clone(),
        "slow_work",
        Some(100), // 100ms deadline
    );
    let handle = scheduler.submit(ctx).await.expect("submit");

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        handle.result,
    )
    .await;
    assert!(result.is_ok(), "deadline should abort promptly");

    let res = result.unwrap().expect("result channel");
    assert!(
        matches!(res, runtime_interaction::AdapterResult::Error { .. }),
        "deadline task should produce Error / timeout result"
    );

    drop(dispatcher);
}
