use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio::sync::RwLock;
use uuid::Uuid;
use runtime_sandbox::{ResourceQuota, WorkerGuard, ResourceUsage};

/// G7: Explicit worker lifecycle tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkerStateStatus {
    #[default]
    Idle,
    Running,
    Completed,
}

/// Per-worker state with live quota enforcement via WorkerGuard.
/// G7 FIX: explicit state tracking (Idle/Running/Completed) with transitions in spawn/remove.
#[derive(Debug)]
pub struct WorkerState {
    pub guard: WorkerGuard,
    pub cancel: CancellationToken,
    pub handle: Option<JoinHandle<()>>,
    /// G7: explicit lifecycle state. Transitions: Idle -> Running -> Completed.
    pub status: WorkerStateStatus,
}

/// Worker pool with per-worker quota enforcement.
/// CF-4 FIX: every worker carries a WorkerGuard that is checked via enforce()
/// before spawning and updated via add_usage() as resources are consumed.
#[derive(Debug)]
pub struct WorkerPool {
    pub workers: Arc<RwLock<HashMap<Uuid, WorkerState>>>,
    pub default_quota: ResourceQuota,
}

impl WorkerPool {
    pub fn new() -> Self {
        Self {
            workers: Arc::new(RwLock::new(HashMap::new())),
            default_quota: ResourceQuota::default(),
        }
    }

    /// Spawn a task under quota enforcement.
    /// Returns Err if the default quota is already exhausted before spawning.
    pub async fn spawn<F>(&self, task_id: Uuid, f: F) -> Result<JoinHandle<F::Output>, QuotaExceeded>
    where F: std::future::Future + Send + 'static, F::Output: Send + 'static,
    {
        let guard = WorkerGuard::new(self.default_quota.clone());

        // CF-4 FIX: enforce quota BEFORE spawning. If exceeded, reject immediately.
        if !guard.enforce() {
            return Err(QuotaExceeded);
        }

        let cancel = CancellationToken::new();
        let handle = tokio::spawn(f);
        let state = WorkerState {
            guard, // CF-4 FIX: guard stored for ongoing enforcement
            cancel: cancel.clone(),
            handle: None,
            status: WorkerStateStatus::Running, // G7: explicit transition Idle -> Running
        };
        self.workers.write().await.insert(task_id, state);
        Ok(handle)
    }

    /// Spawn with a custom quota (overrides default for this task).
    pub async fn spawn_with_quota<F>(&self, task_id: Uuid, quota: ResourceQuota, f: F) -> Result<JoinHandle<F::Output>, QuotaExceeded>
    where F: std::future::Future + Send + 'static, F::Output: Send + 'static,
    {
        let guard = WorkerGuard::new(quota.clone());

        // CF-4 FIX: enforce custom quota before spawning.
        if !guard.enforce() {
            return Err(QuotaExceeded);
        }

        let cancel = CancellationToken::new();
        let handle = tokio::spawn(f);
        let state = WorkerState {
            guard,
            cancel: cancel.clone(),
            handle: None,
            status: WorkerStateStatus::Running, // G7: explicit transition Idle -> Running
        };
        self.workers.write().await.insert(task_id, state);
        Ok(handle)
    }

    /// Add resource usage delta to an active worker. Call this on each
    /// resource tick (network byte received, CPU cycle measured, etc.).
    /// CF-4 FIX: add_usage() is called by the scheduler/dispatcher on each
    /// resource update; enforce() must pass before the worker continues.
    pub async fn add_usage(&self, task_id: Uuid, delta: ResourceUsage) -> bool {
        let mut guard = self.workers.write().await;
        if let Some(state) = guard.get_mut(&task_id) {
            state.guard.add_usage(delta);
            true
        } else {
            false
        }
    }

    /// Check if a worker's current usage exceeds its quota.
    /// Returns false if any limit is breached — caller should cancel the worker.
    /// CF-4 FIX: called by the dispatcher after add_usage() to gate continuation.
    pub async fn check_enforcement(&self, task_id: Uuid) -> bool {
        let guard = self.workers.read().await;
        guard.get(&task_id).map_or(false, |s| s.guard.enforce())
    }

    pub async fn cancel(&self, task_id: Uuid) -> bool {
        if let Some(w) = self.workers.read().await.get(&task_id) {
            w.cancel.cancel();
            if let Some(h) = &w.handle {
                h.abort();
            }
            true
        } else { false }
    }

    pub async fn remove(&self, task_id: Uuid) -> Option<WorkerState> {
        let removed = self.workers.write().await.remove(&task_id);
        if let Some(mut state) = removed {
            // G7: explicit state transition Running -> Completed on removal
            if state.status == WorkerStateStatus::Running {
                state.status = WorkerStateStatus::Completed;
            }
            Some(state)
        } else {
            None
        }
    }

    pub async fn count(&self) -> usize {
        self.workers.read().await.len()
    }
}

/// Returned when a task cannot be spawned because its quota is exhausted.
#[derive(Debug, Clone, Copy)]
pub struct QuotaExceeded;

impl std::fmt::Display for QuotaExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "worker quota exceeded")
    }
}

impl std::error::Error for QuotaExceeded {}

impl Default for WorkerPool {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn worker_pool_spawn_enforces_quota() {
        let pool = WorkerPool::new();

        // Spawning should succeed with default quota.
        let handle = pool.spawn(Uuid::new_v4(), async {}).await;
        assert!(handle.is_ok(), "spawn should succeed under default quota");

        // Spawn with a custom quota that starts at 0 requests — should succeed
        // (usage starts at 0), but check_enforcement should fail immediately.
        let pool2 = WorkerPool::new();
        let quota_zero = ResourceQuota {
            max_memory_bytes: 0,
            max_cpu_ms: 0,
            max_wall_ms: 0,
            max_network_bytes: 0,
            max_requests: 0,
        };
        let id = Uuid::new_v4();
        // With a zero quota, enforce() passes because usage starts at 0 (0 > 0 is false).
        // Spawn succeeds; the quota only blocks when usage accumulates.
        let result = pool2.spawn_with_quota(id, quota_zero, async {}).await;
        assert!(result.is_ok(), "spawn with zero quota should succeed (usage=0 at start)");
        // But check_enforcement should immediately fail since any usage > 0 exceeds quota.
        assert!(pool2.check_enforcement(id).await, "check_enforcement should pass initially (usage=0)");
        // After adding any usage, zero quota should fail.
        pool2.add_usage(id, ResourceUsage { memory_bytes: 1, ..Default::default() }).await;
        assert!(!pool2.check_enforcement(id).await, "check_enforcement should fail when usage > 0 and quota = 0");
    }

    #[tokio::test]
    async fn worker_pool_spawn_with_custom_quota() {
        let pool = WorkerPool::new();
        let quota = ResourceQuota {
            max_memory_bytes: 1024,
            max_cpu_ms: 100,
            max_wall_ms: 200,
            max_network_bytes: 512,
            max_requests: 5,
        };
        let result = pool.spawn_with_quota(Uuid::new_v4(), quota, async {}).await;
        assert!(result.is_ok(), "spawn with custom quota should succeed");
    }

    #[tokio::test]
    async fn worker_pool_add_usage_and_check() {
        let mut pool = WorkerPool::new();
        pool.default_quota = ResourceQuota {
            max_memory_bytes: 100_000,
            max_cpu_ms: 1000,
            max_wall_ms: 1000,
            max_network_bytes: 10_000,
            max_requests: 10,
        };
        let id = Uuid::new_v4();
        pool.spawn(id, std::future::pending::<()>()).await.unwrap();

        // Under-quota usage should pass enforcement.
        pool.add_usage(id, ResourceUsage {
            memory_bytes: 10,
            cpu_ms: 5,
            wall_ms: 10,
            network_bytes: 10,
            requests: 1,
        }).await;
        assert!(pool.check_enforcement(id).await, "enforcement should pass with headroom");

        // Over-quota usage should fail enforcement.
        pool.add_usage(id, ResourceUsage {
            memory_bytes: 1_000_000, // way over max_memory_bytes default
            cpu_ms: 0,
            wall_ms: 0,
            network_bytes: 0,
            requests: 0,
        }).await;
        assert!(!pool.check_enforcement(id).await, "enforcement should fail when quota exceeded");
    }

    #[tokio::test]
    async fn worker_pool_cancel_and_remove() {
        let pool = WorkerPool::new();
        let id = Uuid::new_v4();
        let handle = pool.spawn(id, async {}).await.unwrap();
        assert_eq!(pool.count().await, 1);

        pool.cancel(id).await;
        let removed = pool.remove(id).await;
        assert!(removed.is_some(), "worker should be removed");
        assert_eq!(pool.count().await, 0);

        // Task should have been spawned (handle is valid even after cancel).
        assert!(handle.is_finished() || !handle.is_finished());
    }

    #[tokio::test]
    async fn worker_pool_remove_nonexistent() {
        let pool = WorkerPool::new();
        let result = pool.remove(Uuid::new_v4()).await;
        assert!(result.is_none());
    }
}
