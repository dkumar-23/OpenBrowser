//! G28 — Cross-Process Worker Pool (Priority P0)
//!
//! Observable: a task runs in a child process. A crashing/aborting child is
//! reported `Failed` while the pool remains healthy for the next task; a task
//! exceeding its per-process wall quota is killed and reported
//! `ResourceExceeded`. No execution leaks a worker.

use std::path::PathBuf;
use std::time::Duration;

use runtime_core::worker::{ProcessOutcome, ProcessSpec, ProcessWorkerPool};
use runtime_sandbox::ResourceQuota;
use uuid::Uuid;

fn worker() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_contract_worker"))
}

#[tokio::test]
async fn crashed_child_fails_but_pool_stays_healthy() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let pool = ProcessWorkerPool::new();

        let crash = pool
            .execute(
                Uuid::new_v4(),
                ResourceQuota::default(),
                ProcessSpec::new(worker()).arg("fail").arg("1"),
            )
            .await;
        assert!(
            matches!(crash, ProcessOutcome::Failed { .. }),
            "non-zero child exit must be Failed, got {crash:?}"
        );

        let abort = pool
            .execute(
                Uuid::new_v4(),
                ResourceQuota::default(),
                ProcessSpec::new(worker()).arg("abort"),
            )
            .await;
        assert!(
            matches!(abort, ProcessOutcome::Failed { .. }),
            "aborted child must be Failed, got {abort:?}"
        );

        let ok = pool
            .execute(
                Uuid::new_v4(),
                ResourceQuota::default(),
                ProcessSpec::new(worker()).arg("echo").arg("ok"),
            )
            .await;
        assert_eq!(
            ok,
            ProcessOutcome::Success { stdout: "ok".into() },
            "pool must accept and complete a following task"
        );
        assert_eq!(pool.worker_pool().count().await, 0, "no leaked workers");
    })
    .await
    .expect("crash-isolation test must not hang");
}

#[tokio::test]
async fn wall_quota_kills_child_and_reports_resource_exceeded() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let pool = ProcessWorkerPool::new();
        let quota = ResourceQuota { max_wall_ms: 150, ..Default::default() };

        let outcome = pool
            .execute(
                Uuid::new_v4(),
                quota,
                ProcessSpec::new(worker()).arg("sleep").arg("5000"),
            )
            .await;
        match outcome {
            ProcessOutcome::ResourceExceeded { reason } => assert!(
                reason.contains("wall"),
                "expected a wall-budget reason, got: {reason}"
            ),
            other => panic!("expected ResourceExceeded, got {other:?}"),
        }

        let ok = pool
            .execute(
                Uuid::new_v4(),
                ResourceQuota::default(),
                ProcessSpec::new(worker()).arg("echo").arg("after"),
            )
            .await;
        assert_eq!(
            ok,
            ProcessOutcome::Success { stdout: "after".into() },
            "pool must stay healthy after a quota kill"
        );
        assert_eq!(pool.worker_pool().count().await, 0, "no leaked workers");
    })
    .await
    .expect("wall-quota test must not hang");
}
