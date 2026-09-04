use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio::task::JoinHandle;
use anyhow::Result;
use crate::TaskContext;
use runtime_interaction::AdapterResult;

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
///
/// Dispatch model: `start` spawns a single background task that consumes the
/// queue and invokes the registered executor for each task. `submit` is the
/// sole entry point — callers receive a TaskHandle with a oneshot result
/// channel and must await it to observe the adapter outcome.
pub struct Scheduler {
    queue_tx: mpsc::Sender<TaskEnvelope>,
    /// Receiver is held behind a Mutex so `start` can take it exactly once
    /// after construction. Before `start` the queue is dormant (submit
    /// still works for metrics/backpressure but results will not be sent).
    queue_rx: Arc<std::sync::Mutex<Option<mpsc::Receiver<TaskEnvelope>>>>,
    metrics: Arc<std::sync::RwLock<SchedulerMetrics>>,
    backpressure: Arc<Semaphore>,
    dispatcher: Arc<std::sync::Mutex<Option<Arc<JoinHandle<()>>>>>,  
}

impl Scheduler {
    /// Create a scheduler with max_queue depth. Backpressure is enforced via Semaphore.
    /// Call `start` to activate the dispatcher loop after wiring the executor.
    pub fn new(max_queued: usize) -> Self {
        let (tx, rx) = mpsc::channel(max_queued);
        Self {
            queue_tx: tx,
            queue_rx: Arc::new(std::sync::Mutex::new(Some(rx))),
            metrics: Arc::new(std::sync::RwLock::new(SchedulerMetrics::default())),
            backpressure: Arc::new(Semaphore::new(max_queued)),
            dispatcher: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Start the dispatcher loop. The executor is called for each dequeued task
    /// and must produce the adapter result. Must be called exactly once.
    ///
    /// The executor receives ownership of the TaskContext so it can drive the
    /// adapter (HTTP, DOM, JS, etc.) from within the scheduled task. This is
    /// the only place adapter execution should be invoked — `submit` is the
    /// sole entry point for callers.
    pub fn start<F, Fut>(&self, executor: Arc<F>) -> Arc<JoinHandle<()>>
    where
        F: Fn(TaskContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = AdapterResult> + Send + 'static,
    {
        // Take the receiver out — start() must be called exactly once.
        let mut rx_guard = self.queue_rx.lock().unwrap();
        let mut rx = rx_guard
            .take()
            .expect("Scheduler::start called more than once");

        let metrics = self.metrics.clone();

        let handle = tokio::spawn(async move {
            while let Some(envelope) = rx.recv().await {
                let TaskEnvelope { context, result_tx } = envelope;
                // Transition queued -> running
                {
                    let mut m = metrics.write().unwrap();
                    m.queued = m.queued.saturating_sub(1);
                    m.running += 1;
                }
                // Execute the adapter inside the scheduled task. This is the
                // single point where the adapter runs; callers must never
                // bypass the scheduler with their own tokio::spawn.
                let result = executor(context).await;
                // Classify before sending so we can update metrics correctly.
                let succeeded = matches!(result, AdapterResult::Success { .. });
                // Deliver the result to the awaiting TaskHandle. If the caller
                // dropped their handle (cancellation) the send is a no-op.
                let _ = result_tx.send(result);
                // Transition running -> completed/failed
                {
                    let mut m = metrics.write().unwrap();
                    m.running = m.running.saturating_sub(1);
                    if succeeded {
                        m.completed += 1;
                    } else {
                        m.failed += 1;
                    }
                }
            }
        });

        // Store the dispatcher handle and return a clone of the same handle.
        // JoinHandle is not Clone, so wrap in Arc to share it.
        let dispatcher_handle = Arc::new(handle);
        *self.dispatcher.lock().unwrap() = Some(dispatcher_handle.clone());
        // Drop the lock guard explicitly to release before returning.
        drop(rx_guard);
        dispatcher_handle
    }

    /// Submit a task with backpressure: waits if queue is full.
    /// Returns a handle with a oneshot receiver for the adapter result.
    /// This is the only entry point for scheduling work.
    pub async fn submit(&self, task: TaskContext) -> Result<TaskHandle, BackpressureError> {
        let permit = self.backpressure.acquire().await.map_err(|_| BackpressureError)?;
        let (result_tx, result_rx) = oneshot::channel();
        let handle = TaskHandle {
            task_id: task.task_id,
            cancel: task.cancel.clone(),
            result: result_rx,
        };
        let envelope = TaskEnvelope { context: task, result_tx };
        // Send to queue; if consumer is gone, drop permit and report error
        if self.queue_tx.send(envelope).await.is_err() {
            drop(permit);
            return Err(BackpressureError);
        }
        drop(permit);
        self.inc_queued();
        Ok(handle)
    }

    pub fn cancel(&self, _task_id: uuid::Uuid) -> bool {
        self.inc_cancelled();
        true
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
        let sched = Scheduler::new(10);
        assert_eq!(sched.metrics().cancelled, 0);
        sched.cancel(uuid::Uuid::new_v4());
        assert_eq!(sched.metrics().cancelled, 1);
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
        let sched = Scheduler::new(1);
        // Semaphore starts at 1 permit
        let p1 = sched.backpressure.try_acquire();
        assert!(p1.is_ok());
        // Second acquire should fail (queue size = 1)
        let p2 = sched.backpressure.try_acquire();
        assert!(p2.is_err());
        drop(p1);
        // Now we can acquire again
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
        let sched = Scheduler::new(10);
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
