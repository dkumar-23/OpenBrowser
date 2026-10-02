use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio::task::JoinHandle;
use anyhow::Result;
use crate::TaskContext;
use runtime_interaction::AdapterResult;
use uuid::Uuid;

/// Envelope carries task + oneshot sender so dispatcher can report result.
#[derive(Debug)]
pub(crate) struct TaskEnvelope {
    pub(crate) context: TaskContext,
    pub(crate) result_tx: oneshot::Sender<AdapterResult>,
    /// Optional global-capacity permit held for the task's full lifetime
    /// (queued + running); released when dispatch completes.
    pub(crate) global_permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

/// A submitted task handle — allows cancellation and result retrieval.
pub struct TaskHandle {
    pub task_id: uuid::Uuid,
    pub cancel: tokio_util::sync::CancellationToken,
    /// Receive the adapter result when the scheduled task completes.
    /// The dispatcher sends exactly one value when execution finishes.
    pub result: oneshot::Receiver<AdapterResult>,
}

impl std::fmt::Debug for TaskHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskHandle")
            .field("task_id", &self.task_id)
            .finish()
    }
}

/// Scheduler metrics for observability.
#[derive(Debug, Default, Clone)]
pub struct SchedulerMetrics {
    pub queued: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub timed_out: usize,
    pub resource_exceeded: usize,
}

/// How often the quota monitor polls the usage provider for running tasks.
const QUOTA_TICK_MS: u64 = 25;

/// Scheduler with bounded queue, backpressure, cancellation, quota
/// enforcement, and dispatch loop.
/// Designed for 100x-1000x agent scale: independent task slots, no global lock.
///
/// Quota semantics (Phase 4.3): `TaskContext.quota` is enforced on the
/// dispatch path through the WorkerPool — the task is registered as a worker
/// on start, a monitor ticks the usage provider into the pool guard and
/// hard-cancels on enforcement failure, and the `max_wall_ms` budget joins
/// the task deadline as a single wall-clock limit. An all-zero quota means
/// unlimited: no monitor is armed and no wall budget applies.
///
/// Memory/CPU/network/request enforcement requires a usage provider injected
/// via [`Scheduler::with_usage_provider`]; without one those quota fields are
/// not enforced (see that method's documentation).
pub struct Scheduler {
    queue_tx: mpsc::Sender<TaskEnvelope>,
    queue_rx: Arc<std::sync::Mutex<Option<mpsc::Receiver<TaskEnvelope>>>>,
    metrics: Arc<std::sync::RwLock<SchedulerMetrics>>,
    backpressure: Arc<Semaphore>,
    concurrency_sem: Arc<Semaphore>,
    dispatcher: Arc<std::sync::Mutex<Option<Arc<JoinHandle<()>>>>>,
    #[allow(dead_code)] // kept for diagnostics; limits enforced via the semaphores
    max_concurrent: usize,
    cancellation_registry: Arc<std::sync::Mutex<std::collections::HashMap<Uuid, tokio_util::sync::CancellationToken>>>,
    execution_records: Arc<std::sync::Mutex<std::collections::HashMap<Uuid, Arc<crate::execution::ExecutionRecord>>>>,
    observability: Option<Arc<dyn runtime_observability::Observability>>,
    worker_pool: Arc<crate::worker::WorkerPool>,
    usage_provider: Option<Arc<dyn Fn(Uuid) -> runtime_sandbox::ResourceUsage + Send + Sync>>,
    broker: Option<Arc<crate::broker::TaskBroker>>,
    global: Option<Arc<crate::broker::GlobalCapacity>>,
}

impl Scheduler {
    pub fn new(max_queue: usize, max_concurrent: usize) -> Self {
        let (tx, rx) = mpsc::channel(max_queue);
        Self {
            queue_tx: tx,
            queue_rx: Arc::new(std::sync::Mutex::new(Some(rx))),
            metrics: Arc::new(std::sync::RwLock::new(SchedulerMetrics::default())),
            backpressure: Arc::new(Semaphore::new(max_concurrent)),
            concurrency_sem: Arc::new(Semaphore::new(max_concurrent)),
            dispatcher: Arc::new(std::sync::Mutex::new(None)),
            max_concurrent,
            cancellation_registry: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            execution_records: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            observability: None,
            worker_pool: Arc::new(crate::worker::WorkerPool::new()),
            usage_provider: None,
            broker: None,
            global: None,
        }
    }

    /// Share a [`crate::broker::TaskBroker`] between schedulers. Every task
    /// submitted through any scheduler sharing the broker is claimed and
    /// executed by exactly one scheduler's dispatcher.
    pub fn with_broker(mut self, broker: Arc<crate::broker::TaskBroker>) -> Self {
        self.broker = Some(broker);
        self
    }

    /// Attach a shared [`crate::broker::GlobalCapacity`]. A submitted task
    /// holds one permit for its whole lifetime; submitting while the global
    /// capacity is exhausted returns [`BackpressureError`] rather than
    /// blocking or dropping the task.
    pub fn with_global_capacity(mut self, capacity: Arc<crate::broker::GlobalCapacity>) -> Self {
        self.global = Some(capacity);
        self
    }

    pub fn with_observability(mut self, obs: Arc<dyn runtime_observability::Observability>) -> Self {
        self.observability = Some(obs);
        self
    }

    /// Share a WorkerPool with the scheduler: registered tasks' quota state
    /// lives in this pool, and `pool.cancel(task_id)` hard-cancels them.
    pub fn with_worker_pool(mut self, pool: Arc<crate::worker::WorkerPool>) -> Self {
        self.worker_pool = pool;
        self
    }

    /// Provide per-task resource usage estimates: the quota monitor polls
    /// `provider(task_id)` on every tick and accumulates the delta into the
    /// WorkerPool guard.
    ///
    /// **Enforcement depends on this provider.** When no provider is injected
    /// (the default), the non-wall quota fields — `max_memory_bytes`,
    /// `max_cpu_ms`, `max_network_bytes`, `max_requests` — are **not
    /// enforced**: the monitor observes only zeros, so a task declaring such
    /// limits runs without them. The `max_wall_ms` budget and the task
    /// deadline remain actively enforced because they are driven by timers,
    /// not by the provider. Inject a provider to enforce the other
    /// dimensions.
    pub fn with_usage_provider(mut self, provider: Arc<dyn Fn(Uuid) -> runtime_sandbox::ResourceUsage + Send + Sync>) -> Self {
        self.usage_provider = Some(provider);
        self
    }

    pub fn start<F, Fut>(&self, executor: Arc<F>) -> Arc<JoinHandle<()>>
    where
        F: Fn(TaskContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = AdapterResult> + Send + 'static,
    {
        let mut rx_guard = self.queue_rx.lock().unwrap();
        let mut rx = rx_guard
            .take()
            .expect("Scheduler::start called more than once");
        drop(rx_guard);

        // Distributed scheduling: forward envelopes claimed from the shared
        // broker into this scheduler's local queue. Exactly one scheduler
        // wins each claim (atomic pop under the broker mutex).
        if let Some(broker) = self.broker.clone() {
            let tx = self.queue_tx.clone();
            tokio::spawn(async move {
                while let Some(envelope) = broker.claim().await {
                    if tx.send(envelope).await.is_err() {
                        break;
                    }
                }
            });
        }

        let metrics = self.metrics.clone();
        let observability = self.observability.clone();
        let concurrency_sem = self.concurrency_sem.clone();
        let cancellation_registry = self.cancellation_registry.clone();
        let execution_records = self.execution_records.clone();
        let worker_pool = self.worker_pool.clone();
        let usage_provider = self.usage_provider.clone();

        let handle = tokio::spawn(async move {
            while let Some(envelope) = rx.recv().await {
                let TaskEnvelope { context, result_tx, global_permit } = envelope;

                // Register cancellation token
                {
                    let mut reg = cancellation_registry.lock().unwrap();
                    reg.insert(context.task_id, context.cancel.clone());
                }

                // Transition queued -> running (counter) — ONLY after actual concurrency acquired
                let sem = concurrency_sem.clone();
                let reg = cancellation_registry.clone();
                let exec_recs = execution_records.clone();
                let metrics_inner = metrics.clone();
                let obs_clone_for_spawn = observability.clone();
                let task_id = context.task_id;
                let executor_inner = executor.clone();
                let pool = worker_pool.clone();
                let usage_provider = usage_provider.clone();

                tokio::spawn(async move {
                    let _global_permit = global_permit;
                    let _permit = sem.acquire().await.ok();
                    let deadline = context.deadline;
                    // Phase 4.3: the quota wall budget joins the task deadline
                    // into ONE wall-clock limit (existing timeout pattern, no
                    // parallel timers). quota.max_wall_ms == 0 = unlimited.
                    let quota_wall = if context.quota.max_wall_ms > 0 { Some(context.quota.max_wall_ms) } else { None };
                    let effective_deadline = match (deadline, quota_wall) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                    // When both limits are set, the smaller one fired; equal
                    // limits are classified as the task deadline (TimedOut).
                    let quota_wall_breach = quota_wall.map_or(false, |q| deadline.map_or(true, |d| q < d));
                    let enforce_quota = context.quota != runtime_sandbox::ResourceQuota::default();
                    let cancel_token = context.cancel.clone();
                    let exec_record_clone = exec_recs.clone();
                    let obs_inner = obs_clone_for_spawn.as_ref().cloned();
                    let breach: Arc<std::sync::Mutex<Option<String>>> =
                        Arc::new(std::sync::Mutex::new(None));

                    // Emit lifecycle: started
                    if let Some(obs) = obs_inner.as_ref() {
                        let evt = runtime_observability::LifecycleEvent {
                            task_id: context.task_id,
                            agent_id: context.agent_id,
                            delegation_id: context.delegation_id,
                            event_type: "started".into(),
                            timestamp: chrono::Utc::now(),
                            details: None,
                        };
                        obs.record_lifecycle(evt);
                    }

                    // Get execution record and transition to Running
                    let rec = {
                        let guard = exec_recs.lock().unwrap();
                        guard.get(&context.task_id).cloned()
                    };
                    if let Some(r) = rec {
                        let _ = r.transition(crate::execution::ExecutionState::Running { worker_id: Uuid::new_v4() }).await;
                    }
                    // Count as running only after concurrency slot acquired
                    let queue_depth = {
                        let mut m = metrics_inner.write().unwrap();
                        m.queued = m.queued.saturating_sub(1);
                        m.running += 1;
                        m.queued as f64
                    };
                    if let Some(obs) = obs_inner.as_ref() {
                        obs.gauge("scheduler_queue_depth", queue_depth, &[]);
                    }

                    // Phase 4.3: register the task's worker/guard in the
                    // WorkerPool — quota state lives in the pool, no
                    // duplicate bookkeeping. Pool cancel() hard-cancels via
                    // the shared token.
                    pool.register(task_id, context.quota, cancel_token.clone()).await;

                    // Phase 4.3: quota usage monitor. Ticks the usage provider
                    // into the pool guard and hard-cancels on enforcement
                    // failure. Only armed when the task has a nonzero quota:
                    // an all-zero quota means unlimited.
                    let monitor = if enforce_quota {
                        let pool_m = pool.clone();
                        let token_m = cancel_token.clone();
                        let provider_m = usage_provider.clone();
                        let breach_m = breach.clone();
                        Some(tokio::spawn(async move {
                            let mut ticker = tokio::time::interval(std::time::Duration::from_millis(QUOTA_TICK_MS));
                            ticker.tick().await; // first tick fires immediately — skip it
                            loop {
                                ticker.tick().await;
                                if token_m.is_cancelled() { break; }
                                let delta = provider_m.as_ref().map(|p| p(task_id)).unwrap_or_default();
                                if delta != runtime_sandbox::ResourceUsage::default() {
                                    pool_m.add_usage(task_id, delta).await;
                                }
                                if !pool_m.check_enforcement(task_id).await {
                                    let dim = pool_m.breach_reason(task_id).await.unwrap_or("quota");
                                    *breach_m.lock().unwrap() = Some(format!("resource exceeded: {}", dim));
                                    token_m.cancel();
                                    break;
                                }
                            }
                        }))
                    } else { None };

                    let result = if let Some(deadline_millis) = effective_deadline {
                        let deadline_instant = std::time::Instant::now() + std::time::Duration::from_millis(deadline_millis);
                        tokio::select! {
                            r = executor_inner(context.clone()) => r,
                            _ = cancel_token.cancelled() => {
                                let reason = breach.lock().unwrap().clone();
                                match reason {
                                    Some(msg) => {
                                        transition_to(&exec_record_clone, task_id, crate::execution::ExecutionState::ResourceExceeded).await;
                                        AdapterResult::Error { message: msg, replay_sequence: 0 }
                                    }
                                    None => {
                                        transition_to(&exec_record_clone, task_id, crate::execution::ExecutionState::Cancelled).await;
                                        AdapterResult::Error { message: "cancelled".into(), replay_sequence: 0 }
                                    }
                                }
                            }
                            _ = tokio::time::sleep_until(deadline_instant.into()) => {
                                cancel_token.cancel();
                                if quota_wall_breach {
                                    transition_to(&exec_record_clone, task_id, crate::execution::ExecutionState::ResourceExceeded).await;
                                    AdapterResult::Error { message: "resource exceeded: wall clock budget".into(), replay_sequence: 0 }
                                } else {
                                    transition_to(&exec_record_clone, task_id, crate::execution::ExecutionState::TimedOut).await;
                                    AdapterResult::Error { message: "deadline exceeded".into(), replay_sequence: 0 }
                                }
                            }
                        }
                    } else {
                        // Select on cancellation (G4) — executor must be cancellable
                        tokio::select! {
                            r = executor_inner(context.clone()) => r,
                            _ = cancel_token.cancelled() => {
                                let reason = breach.lock().unwrap().clone();
                                match reason {
                                    Some(msg) => {
                                        transition_to(&exec_record_clone, task_id, crate::execution::ExecutionState::ResourceExceeded).await;
                                        AdapterResult::Error { message: msg, replay_sequence: 0 }
                                    }
                                    None => {
                                        transition_to(&exec_record_clone, task_id, crate::execution::ExecutionState::Cancelled).await;
                                        AdapterResult::Error { message: "cancelled".into(), replay_sequence: 0 }
                                    }
                                }
                            }
                        }
                    };
                    let succeeded;
                    let is_cancelled;
                    let is_timed_out;
                    let is_resource_exceeded;
                    {
                        use runtime_interaction::AdapterResult;
                        succeeded = matches!(result, AdapterResult::Success { .. });
                        is_cancelled = matches!(&result, AdapterResult::Error { message, .. } if message == "cancelled");
                        is_timed_out = matches!(&result, AdapterResult::Error { message, .. } if message == "deadline exceeded");
                        is_resource_exceeded = matches!(&result, AdapterResult::Error { message, .. } if message.starts_with("resource exceeded"));
                    }
                    let _ = result_tx.send(result);
                    // Emit lifecycle: terminal state
                    if let Some(obs) = obs_inner.as_ref() {
                        let evt_type = if succeeded { "completed" }
                            else if is_cancelled { "cancelled" }
                            else if is_timed_out { "timed_out" }
                            else if is_resource_exceeded { "resource_exceeded" }
                            else { "failed" };
                        let evt = runtime_observability::LifecycleEvent {
                            task_id,
                            agent_id: context.agent_id,
                            delegation_id: context.delegation_id,
                            event_type: evt_type.into(),
                            timestamp: chrono::Utc::now(),
                            details: None,
                        };
                        obs.record_lifecycle(evt);
                    }
                    {
                        let mut m = metrics_inner.write().unwrap();
                        m.running = m.running.saturating_sub(1);
                        if succeeded {
                            m.completed += 1;
                        } else if is_cancelled {
                            m.cancelled += 1;
                        } else if is_timed_out {
                            m.timed_out += 1;
                        } else if is_resource_exceeded {
                            m.resource_exceeded += 1;
                        } else {
                            m.failed += 1;
                        }
                    }
                    // Transition to terminal state
                    let rec2 = {
                        let guard = exec_recs.lock().unwrap();
                        guard.get(&task_id).cloned()
                    };
                    if let Some(r) = rec2 {
                        let state = if succeeded {
                            crate::execution::ExecutionState::Completed
                        } else {
                            crate::execution::ExecutionState::Failed { error: "adapter error".into() }
                        };
                        let _ = r.transition(state).await;
                    }
                    // Phase 4.3: release quota bookkeeping — pool guard outlives
                    // nothing, monitor is stopped, the pool survives for the next task.
                    pool.remove(task_id).await;
                    if let Some(mon) = monitor.as_ref() {
                        mon.abort();
                    }
                    // Clean up registry
                    reg.lock().unwrap().remove(&task_id);
                });
            }
        });

        let dispatcher_handle = Arc::new(handle);
        *self.dispatcher.lock().unwrap() = Some(dispatcher_handle.clone());
        dispatcher_handle
    }

    pub async fn submit(&self, task: TaskContext) -> Result<TaskHandle, BackpressureError> {
        let global_permit = self.acquire_global()?;
        let permit = self.backpressure.acquire().await.map_err(|_| BackpressureError)?;
        let (handle, envelope) = self.prepare(task, global_permit);
        if let Some(broker) = &self.broker {
            broker.push(envelope);
        } else if self.queue_tx.send(envelope).await.is_err() {
            drop(permit);
            return Err(BackpressureError);
        }
        drop(permit);
        self.inc_queued();
        self.emit_queue_depth();
        Ok(handle)
    }

    /// Non-blocking variant of [`Scheduler::submit`]: if the local backpressure
    /// slot or the shared [`crate::broker::GlobalCapacity`] is exhausted it
    /// returns [`BackpressureError`] immediately instead of blocking.
    pub async fn try_submit(&self, task: TaskContext) -> Result<TaskHandle, BackpressureError> {
        let global_permit = self.acquire_global()?;
        let permit = self
            .backpressure
            .clone()
            .try_acquire_owned()
            .map_err(|_| BackpressureError)?;
        let (handle, envelope) = self.prepare(task, global_permit);
        if let Some(broker) = &self.broker {
            broker.push(envelope);
        } else if self.queue_tx.try_send(envelope).is_err() {
            drop(permit);
            return Err(BackpressureError);
        }
        drop(permit);
        self.inc_queued();
        self.emit_queue_depth();
        Ok(handle)
    }

    /// Build the handle + envelope and register execution/cancellation state.
    fn prepare(
        &self,
        task: TaskContext,
        global_permit: Option<tokio::sync::OwnedSemaphorePermit>,
    ) -> (TaskHandle, TaskEnvelope) {
        let (result_tx, result_rx) = oneshot::channel();
        let rec = Arc::new(crate::execution::ExecutionRecord::new(task.task_id));
        {
            let mut er = self.execution_records.lock().unwrap();
            er.insert(task.task_id, rec);
        }
        {
            let mut reg = self.cancellation_registry.lock().unwrap();
            reg.insert(task.task_id, task.cancel.clone());
        }
        let task_id = task.task_id;
        let cancel_token = task.cancel.clone();
        let envelope = TaskEnvelope { context: task, result_tx, global_permit };
        let handle = TaskHandle { task_id, cancel: cancel_token, result: result_rx };
        (handle, envelope)
    }

    fn acquire_global(&self) -> Result<Option<tokio::sync::OwnedSemaphorePermit>, BackpressureError> {
        match &self.global {
            Some(capacity) => capacity.try_acquire().map(Some),
            None => Ok(None),
        }
    }

    pub fn cancel(&self, task_id: Uuid) -> bool {
        let registry = self.cancellation_registry.lock().unwrap();
        if let Some(token) = registry.get(&task_id) {
            token.cancel();
            drop(registry);
            self.inc_cancelled();
            true
        } else {
            false
        }
    }

    pub fn metrics(&self) -> SchedulerMetrics {
        (*self.metrics.read().unwrap()).clone()
    }

    fn inc_queued(&self) {
        let mut m = self.metrics.write().unwrap();
        m.queued += 1;
    }

    fn inc_cancelled(&self) {
        let mut m = self.metrics.write().unwrap();
        m.cancelled += 1;
    }

    /// R6: report the current queue depth as a real gauge on the shared
    /// Observability/metric channel (no separate channel).
    fn emit_queue_depth(&self) {
        if let Some(obs) = &self.observability {
            let depth = self.metrics.read().unwrap().queued as f64;
            obs.gauge("scheduler_queue_depth", depth, &[]);
        }
    }
}

/// Look up a task's execution record and transition it to `state`
/// (invalid transitions are ignored — the record keeps its prior state).
async fn transition_to(
    records: &Arc<std::sync::Mutex<std::collections::HashMap<Uuid, Arc<crate::execution::ExecutionRecord>>>>,
    task_id: Uuid,
    state: crate::execution::ExecutionState,
) {
    let rec = records.lock().unwrap().get(&task_id).cloned();
    if let Some(r) = rec {
        let _ = r.transition(state).await;
    }
}

#[derive(Debug)]
pub struct BackpressureError;

impl std::fmt::Display for BackpressureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "scheduler queue at capacity")
    }
}

impl std::error::Error for BackpressureError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn scheduler_cancel_increments_metric() {
        let sched = Scheduler::new(10, 2);
        assert_eq!(sched.metrics().cancelled, 0);
        // Cancel on unregistered task should return false
        assert!(!sched.cancel(uuid::Uuid::new_v4()));
        assert_eq!(sched.metrics().cancelled, 0);
    }

    #[tokio::test]
    async fn scheduler_metrics_default() {
        let m = SchedulerMetrics::default();
        assert_eq!(m.queued, 0);
        assert_eq!(m.running, 0);
        assert_eq!(m.cancelled, 0);
    }

    #[tokio::test]
    async fn scheduler_backpressure_acquire_semaphore() {
        let sched = Scheduler::new(10, 1);
        let p1 = sched.backpressure.try_acquire();
        assert!(p1.is_ok());
        let p2 = sched.backpressure.try_acquire();
        assert!(p2.is_err());
        drop(p1);
        let p3 = sched.backpressure.try_acquire();
        assert!(p3.is_ok());
    }

    #[tokio::test]
    async fn scheduler_dispatches_to_executor_and_returns_result() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();
        let exec = Arc::new(move |_ctx: TaskContext| {
            let c = counter_clone.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                AdapterResult::Success {
                    response: "ok".into(),
                    replay_sequence: 1,
                }
            }
        });
        let sched = Scheduler::new(10, 2);
        let _dispatcher = sched.start(exec);

        let policy = Arc::new(runtime_policy::PolicyEngine::new());
        let ctx = TaskContext::new(uuid::Uuid::new_v4(), None, runtime_sandbox::ResourceQuota::default(), policy);
        let handle = sched.submit(ctx).await.expect("submit");

        let result = handle.result.await.expect("result");
        match result {
            AdapterResult::Success { response, .. } => assert_eq!(response, "ok"),
            other => panic!("unexpected: {:?}", other),
        }
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
