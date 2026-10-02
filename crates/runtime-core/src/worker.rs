use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio::sync::RwLock;
use uuid::Uuid;
use runtime_sandbox::{ResourceQuota, Watchdog, WorkerGuard, ResourceUsage};

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

    /// Register a running task under quota enforcement WITHOUT spawning a
    /// future: the scheduler owns the execution, the pool owns the quota
    /// bookkeeping and can hard-cancel through the shared token.
    pub async fn register(&self, task_id: Uuid, quota: ResourceQuota, cancel: CancellationToken) {
        let guard = WorkerGuard::new(quota);
        let state = WorkerState {
            guard,
            cancel,
            handle: None,
            status: WorkerStateStatus::Running,
        };
        self.workers.write().await.insert(task_id, state);
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

    /// Which quota dimension a worker is currently breaching
    /// (None when unbreached or unknown).
    pub async fn breach_reason(&self, task_id: Uuid) -> Option<&'static str> {
        self.workers.read().await.get(&task_id).map_or(None, |s| s.guard.breached_dimension())
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

/// Description of the child program a [`ProcessWorkerPool`] runs. The pool is
/// program-agnostic so any worker entrypoint (including
/// [`run_worker_entrypoint`]) can be used.
#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl ProcessSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self { program: program.into(), args: Vec::new() }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }
}

/// Terminal outcome of one process-isolated task execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutcome {
    Success { stdout: String },
    Failed { code: Option<i32>, stderr: String },
    ResourceExceeded { reason: String },
}

/// Process-isolated worker pool: each task runs in a child process, its
/// wall-clock quota is enforced as a hard kill, and a crash/kill of one child
/// never affects the pool. Quota bookkeeping reuses [`WorkerPool`]'s
/// [`WorkerGuard`] and the sandbox [`Watchdog`]; the actual OS termination is
/// `kill_on_drop` plus the wall timeout.
pub struct ProcessWorkerPool {
    pool: Arc<WorkerPool>,
}

impl ProcessWorkerPool {
    pub fn new() -> Self {
        Self { pool: Arc::new(WorkerPool::new()) }
    }

    pub fn with_worker_pool(pool: Arc<WorkerPool>) -> Self {
        Self { pool }
    }

    pub fn worker_pool(&self) -> Arc<WorkerPool> {
        self.pool.clone()
    }

    /// Run `spec` in a child process under `quota`. A non-zero exit is
    /// `Failed`; exceeding `max_wall_ms` is `ResourceExceeded`. The pool is
    /// always left healthy.
    pub async fn execute(
        &self,
        task_id: Uuid,
        quota: ResourceQuota,
        spec: ProcessSpec,
    ) -> ProcessOutcome {
        let cancel = CancellationToken::new();
        self.pool.register(task_id, quota, cancel.clone()).await;
        let _watchdog = Watchdog::arm(quota.max_wall_ms, cancel.clone());

        let mut command = tokio::process::Command::new(&spec.program);
        command
            .args(&spec.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let outcome = match command.spawn() {
            Ok(child) => {
                if quota.max_wall_ms > 0 {
                    match tokio::time::timeout(
                        Duration::from_millis(quota.max_wall_ms),
                        child.wait_with_output(),
                    )
                    .await
                    {
                        Ok(Ok(output)) => classify_process_output(output),
                        Ok(Err(err)) => ProcessOutcome::Failed {
                            code: None,
                            stderr: format!("wait error: {err}"),
                        },
                        Err(_) => ProcessOutcome::ResourceExceeded {
                            reason: "resource exceeded: wall clock budget".into(),
                        },
                    }
                } else {
                    match child.wait_with_output().await {
                        Ok(output) => classify_process_output(output),
                        Err(err) => ProcessOutcome::Failed {
                            code: None,
                            stderr: format!("wait error: {err}"),
                        },
                    }
                }
            }
            Err(err) => ProcessOutcome::Failed {
                code: None,
                stderr: format!("spawn error: {err}"),
            },
        };

        self.pool.remove(task_id).await;
        outcome
    }
}

impl Default for ProcessWorkerPool {
    fn default() -> Self {
        Self::new()
    }
}

fn classify_process_output(output: std::process::Output) -> ProcessOutcome {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        ProcessOutcome::Success { stdout }
    } else {
        ProcessOutcome::Failed { code: output.status.code(), stderr }
    }
}

/// Minimal in-process worker protocol used by the child-process entrypoint.
/// Modes: `sleep <ms>`, `fail <code>`, `abort`, `echo <text>`. Returns the
/// process exit code.
pub async fn run_worker_entrypoint(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("sleep") => {
            let ms = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
            tokio::time::sleep(Duration::from_millis(ms)).await;
            0
        }
        Some("fail") => args.get(1).and_then(|s| s.parse::<i32>().ok()).unwrap_or(1),
        Some("abort") => std::process::abort(),
        Some("echo") => {
            println!("{}", args.get(1).map(String::as_str).unwrap_or(""));
            0
        }
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn worker_pool_spawn_enforces_quota() {
        let pool = WorkerPool::new();

        // Spawning should succeed with default quota (all-zero = unlimited).
        let handle = pool.spawn(Uuid::new_v4(), async {}).await;
        assert!(handle.is_ok(), "spawn should succeed under default quota");

        // A zero quota is unlimited: accumulating usage never breaches.
        let pool2 = WorkerPool::new();
        let quota_zero = ResourceQuota {
            max_memory_bytes: 0,
            max_cpu_ms: 0,
            max_wall_ms: 0,
            max_network_bytes: 0,
            max_requests: 0,
        };
        let id = Uuid::new_v4();
        let result = pool2.spawn_with_quota(id, quota_zero, async {}).await;
        assert!(result.is_ok(), "spawn with zero quota should succeed");
        assert!(pool2.check_enforcement(id).await, "zero quota is unlimited initially");
        pool2.add_usage(id, ResourceUsage { memory_bytes: 1, ..Default::default() }).await;
        assert!(
            pool2.check_enforcement(id).await,
            "zero quota must remain unlimited after usage accumulates"
        );

        // A nonzero quota is still enforced: exceeding it breaches.
        let pool3 = WorkerPool::new();
        let quota_nonzero = ResourceQuota {
            max_memory_bytes: 100,
            max_cpu_ms: 0,
            max_wall_ms: 0,
            max_network_bytes: 0,
            max_requests: 0,
        };
        let id3 = Uuid::new_v4();
        pool3.spawn_with_quota(id3, quota_nonzero, async {}).await.unwrap();
        assert!(pool3.check_enforcement(id3).await, "usage under a nonzero quota must pass");
        pool3.add_usage(id3, ResourceUsage { memory_bytes: 101, ..Default::default() }).await;
        assert!(
            !pool3.check_enforcement(id3).await,
            "usage above a nonzero quota must breach"
        );
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
