//! G35 — Local-First Backpressure Under Global Saturation (M3)
//!
//! Observable: several schedulers sharing a small `GlobalCapacity` with small
//! local queues admit and complete work boundedly under a saturated burst and
//! recover afterward with no permanent starvation. Because submission acquires
//! the local slot before global capacity, no scheduler holds a shared global
//! permit while blocked on its own local queue.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use runtime_core::{GlobalCapacity, Scheduler, TaskContext};
use runtime_interaction::AdapterResult;
use runtime_sandbox::ResourceQuota;
use uuid::Uuid;

const SCHEDULERS: usize = 4;
const ATTEMPTS_PER_SCHED: usize = 12;

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
async fn saturated_schedulers_complete_without_starvation() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let policy = Arc::new(runtime_policy::PolicyEngine::new());
        let completed = Arc::new(AtomicUsize::new(0));

        let exec = {
            let completed = completed.clone();
            Arc::new(move |_ctx: TaskContext| {
                let c = completed.clone();
                async move {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    c.fetch_add(1, Ordering::SeqCst);
                    AdapterResult::Success { response: "ok".into(), replay_sequence: 1 }
                }
            })
        };

        let capacity = Arc::new(GlobalCapacity::new(3));
        let schedulers: Vec<Arc<Scheduler>> = (0..SCHEDULERS)
            .map(|_| Arc::new(Scheduler::new(8, 1).with_global_capacity(capacity.clone())))
            .collect();
        let dispatchers: Vec<_> = schedulers.iter().map(|s| s.start(exec.clone())).collect();

        // Phase 1 — saturated concurrent submissions across schedulers.
        let mut attempts = Vec::new();
        for sched in &schedulers {
            for _ in 0..ATTEMPTS_PER_SCHED {
                let sched = sched.clone();
                let policy = policy.clone();
                attempts.push(tokio::spawn(async move {
                    sched.try_submit(ctx(&policy)).await.ok()
                }));
            }
        }
        let mut handles = Vec::new();
        for attempt in attempts {
            if let Some(handle) = attempt.await.expect("join attempt") {
                handles.push(handle);
            }
        }

        assert!(
            !handles.is_empty(),
            "at least one saturated submission must be admitted"
        );

        for handle in handles {
            let result = tokio::time::timeout(Duration::from_secs(4), handle.result)
                .await
                .expect("admitted task must resolve boundedly")
                .expect("result channel");
            assert!(result.is_success(), "admitted task must succeed, got {result:?}");
        }

        // Phase 2 — after the burst drains, every scheduler makes progress
        // (no scheduler permanently starved by another).
        for sched in &schedulers {
            let handle = tokio::time::timeout(Duration::from_secs(4), sched.submit(ctx(&policy)))
                .await
                .expect("post-drain submit must not hang")
                .expect("every scheduler must submit after drain");
            let result = tokio::time::timeout(Duration::from_secs(4), handle.result)
                .await
                .expect("post-drain task must resolve")
                .expect("result channel");
            assert!(result.is_success(), "post-drain task must succeed, got {result:?}");
        }

        assert!(
            completed.load(Ordering::SeqCst) >= SCHEDULERS,
            "all post-drain tasks must have completed"
        );

        for dispatcher in dispatchers {
            drop(dispatcher);
        }
    })
    .await
    .expect("g35 saturation test must not hang");
}
