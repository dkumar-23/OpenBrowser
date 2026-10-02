//! G27 — Distributed Scheduling Across Worker Sets (Priority P0)
//!
//! Observable: two schedulers sharing one `TaskBroker` execute every
//! submitted task exactly once — total executions equal the submission count
//! and every task id is observed exactly once.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use runtime_core::{Scheduler, TaskBroker, TaskContext, WorkerPool};
use runtime_interaction::AdapterResult;
use runtime_sandbox::ResourceQuota;
use uuid::Uuid;

#[tokio::test]
async fn shared_broker_executes_each_task_exactly_once() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let broker = Arc::new(TaskBroker::new());
        let pool = Arc::new(WorkerPool::new());

        let executions = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(HashSet::<Uuid>::new()));

        let executions_clone = executions.clone();
        let seen_clone = seen.clone();
        let exec = Arc::new(move |ctx: TaskContext| {
            let executions = executions_clone.clone();
            let seen = seen_clone.clone();
            async move {
                executions.fetch_add(1, Ordering::SeqCst);
                seen.lock().unwrap().insert(ctx.task_id);
                tokio::time::sleep(Duration::from_millis(5)).await;
                AdapterResult::Success { response: "ok".into(), replay_sequence: 1 }
            }
        });

        let policy = Arc::new(runtime_policy::PolicyEngine::new());
        let sched_a = Scheduler::new(100, 4)
            .with_broker(broker.clone())
            .with_worker_pool(pool.clone());
        let sched_b = Scheduler::new(100, 4)
            .with_broker(broker.clone())
            .with_worker_pool(pool.clone());
        let dispatcher_a = sched_a.start(exec.clone());
        let dispatcher_b = sched_b.start(exec.clone());

        const N: usize = 24;
        let mut handles = Vec::new();
        for i in 0..N {
            let ctx = TaskContext::with_action(
                Uuid::new_v4(),
                None,
                ResourceQuota::default(),
                policy.clone(),
                "work",
            );
            let sched = if i % 2 == 0 { &sched_a } else { &sched_b };
            handles.push(sched.submit(ctx).await.expect("submit"));
        }

        for handle in handles {
            tokio::time::timeout(Duration::from_secs(5), handle.result)
                .await
                .expect("each task must resolve (no hang)")
                .expect("result channel");
        }

        assert_eq!(
            executions.load(Ordering::SeqCst),
            N,
            "every submitted task must be executed exactly once"
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            N,
            "every task id must be observed exactly once"
        );

        drop(dispatcher_a);
        drop(dispatcher_b);
    })
    .await
    .expect("shared-broker test must not hang");
}
