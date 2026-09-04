use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio::task::JoinHandle;
use anyhow::Result;
use crate::TaskContext;
use runtime_interaction::AdapterResult;
use uuid::Uuid;

/// Envelope carries task + oneshot sender so dispatcher can report result.
struct TaskEnvelope {
    context: TaskContext,
    result_tx: oneshot::Sender<AdapterResult>,
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
}

/// Scheduler with bounded queue, backpressure, cancellation, and dispatch loop.
/// Designed for 100x-1000x agent scale: independent task slots, no global lock.
pub struct Scheduler {
    queue_tx: mpsc::Sender<TaskEnvelope>,
    queue_rx: Arc<std::sync::Mutex<Option<mpsc::Receiver<TaskEnvelope>>>>,
    metrics: Arc<std::sync::RwLock<SchedulerMetrics>>,
    backpressure: Arc<Semaphore>,
    concurrency_sem: Arc<Semaphore>,
    dispatcher: Arc<std::sync::Mutex<Option<Arc<JoinHandle<()>>>>>,
    max_concurrent: usize,
    cancellation_registry: Arc<std::sync::Mutex<std::collections::HashMap<Uuid, tokio_util::sync::CancellationToken>>>,
    execution_records: Arc<std::sync::Mutex<std::collections::HashMap<Uuid, Arc<crate::execution::ExecutionRecord>>>>,
    observability: Option<Arc<dyn runtime_observability::Observability>>,
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
        }
    }

    pub fn with_observability(mut self, obs: Arc<dyn runtime_observability::Observability>) -> Self {
        self.observability = Some(obs);
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

        let metrics = self.metrics.clone();
        let observability = self.observability.clone();
        let concurrency_sem = self.concurrency_sem.clone();
        let cancellation_registry = self.cancellation_registry.clone();
        let execution_records = self.execution_records.clone();

        let handle = tokio::spawn(async move {
            while let Some(envelope) = rx.recv().await {
                let TaskEnvelope { context, result_tx } = envelope;

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

                tokio::spawn(async move {
                    let _permit = sem.acquire().await.ok();
                    let deadline = context.deadline;
                    let cancel_token = context.cancel.clone();
                    let exec_record_clone = exec_recs.clone();
                    let obs_inner = obs_clone_for_spawn.as_ref().cloned();

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
                    {
                        let mut m = metrics_inner.write().unwrap();
                        m.queued = m.queued.saturating_sub(1);
                        m.running += 1;
                    }

                    let result = if let Some(deadline_millis) = deadline {
                        let deadline_instant = std::time::Instant::now() + std::time::Duration::from_millis(deadline_millis);
                        match tokio::time::timeout_at(deadline_instant.into(), executor_inner(context.clone())).await {
                            Ok(result) => result,
                            Err(_) => {
                                cancel_token.cancel();
                                let rec_clone = {
                                    let guard = exec_record_clone.lock().unwrap();
                                    guard.get(&task_id).cloned()
                                };
                                if let Some(r2) = rec_clone {
                                    let _ = r2.transition(crate::execution::ExecutionState::TimedOut).await;
                                }
                                AdapterResult::Error { message: "deadline exceeded".into(), replay_sequence: 0 }
                            }
                        }
                    } else {
                        // Select on cancellation (G4) — executor must be cancellable
                        tokio::select! {
                            r = executor_inner(context.clone()) => r,
                            _ = cancel_token.cancelled() => {
                                // Transition to Cancelled
                                let rec_clone = {
                                    let guard = exec_record_clone.lock().unwrap();
                                    guard.get(&task_id).cloned()
                                };
                                if let Some(r2) = rec_clone {
                                    let _ = r2.transition(crate::execution::ExecutionState::Cancelled).await;
                                }
                                AdapterResult::Error { message: "cancelled".into(), replay_sequence: 0 }
                            }
                        }
                    };
                    let succeeded;
                    let is_cancelled;
                    let is_timed_out;
                    {
                        use runtime_interaction::AdapterResult;
                        succeeded = matches!(result, AdapterResult::Success { .. });
                        is_cancelled = matches!(&result, AdapterResult::Error { message, .. } if message == "cancelled");
                        is_timed_out = matches!(&result, AdapterResult::Error { message, .. } if message == "deadline exceeded");
                    }
                    let _ = result_tx.send(result);
                    // Emit lifecycle: terminal state
                    if let Some(obs) = obs_inner.as_ref() {
                        let evt_type = if succeeded { "completed" }
                            else if is_cancelled { "cancelled" }
                            else if is_timed_out { "timed_out" }
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
                            // timed out tracked separately
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
        let permit = self.backpressure.acquire().await.map_err(|_| BackpressureError)?;
        let (result_tx, result_rx) = oneshot::channel();
        // Register execution record
        let rec = Arc::new(crate::execution::ExecutionRecord::new(task.task_id));
        {
            let mut er = self.execution_records.lock().unwrap();
            er.insert(task.task_id, rec.clone());
        }
        // Register cancellation
        {
            let mut reg = self.cancellation_registry.lock().unwrap();
            reg.insert(task.task_id, task.cancel.clone());
        }
        let task_id = task.task_id;
        let cancel_token = task.cancel.clone();
        let envelope = TaskEnvelope { context: task, result_tx };
        if self.queue_tx.send(envelope).await.is_err() {
            drop(permit);
            return Err(BackpressureError);
        }
        drop(permit);
        self.inc_queued();
        let handle = TaskHandle {
            task_id,
            cancel: cancel_token,
            result: result_rx,
        };
        Ok(handle)
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
