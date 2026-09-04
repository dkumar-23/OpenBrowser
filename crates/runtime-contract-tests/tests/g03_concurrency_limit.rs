//! G6 — Concurrency Limit (P0)
//!
//! Observable: during concurrent load, running count never exceeds
//! max_concurrent.

use runtime_core::{Scheduler, TaskContext};
use runtime_policy::PolicyEngine;
use std::sync::Arc;

#[tokio::test]
async fn running_never_exceeds_max() {
    let mut policy = runtime_policy::PolicyEngine::new();
    policy.add_capability("noop");
    let policy = Arc::new(policy);
    let max_c = 3;
    let scheduler = Scheduler::new(10, max_c);

    let exec = Arc::new(move |_ctx: TaskContext| {
        async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            runtime_interaction::AdapterResult::Success {
                response: "ok".into(),
                replay_sequence: 1,
            }
        }
    });

    let dispatcher = scheduler.start(exec);
    let agent = uuid::Uuid::new_v4();

    let mut handles = Vec::new();
    for i in 0..max_c + 2 {
        let ctx = TaskContext::with_action(
            agent,
            None,
            runtime_sandbox::ResourceQuota::default(),
            policy.clone(),
            "noop",
        );
        let h = scheduler.submit(ctx).await.expect("submit");
        handles.push(h);
        // Small delay to allow dispatcher to pick up.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let m = scheduler.metrics();
        assert!(
            m.running <= max_c,
            "running ({}) exceeded max_concurrent ({}) at submission {}",
            m.running,
            max_c,
            i
        );
    }

    for h in handles {
        let _ = h.result.await;
    }
    drop(dispatcher);
}
