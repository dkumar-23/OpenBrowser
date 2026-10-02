use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::scheduler::{BackpressureError, TaskEnvelope};

/// Shared work broker for distributed scheduling: multiple [`crate::Scheduler`]
/// instances pull task envelopes from one broker and each envelope is claimed
/// by exactly one consumer. The atomic claim is the mutex-guarded
/// `pop_front`; the `items` semaphore is the wakeup signal so claiming never
/// spins.
#[derive(Debug)]
pub struct TaskBroker {
    inner: Arc<BrokerInner>,
}

#[derive(Debug)]
struct BrokerInner {
    queue: Mutex<VecDeque<TaskEnvelope>>,
    items: Semaphore,
    closed: AtomicBool,
}

impl TaskBroker {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(BrokerInner {
                queue: Mutex::new(VecDeque::new()),
                items: Semaphore::new(0),
                closed: AtomicBool::new(false),
            }),
        }
    }

    pub(crate) fn push(&self, envelope: TaskEnvelope) {
        {
            let mut q = self.inner.queue.lock().unwrap();
            q.push_back(envelope);
        }
        self.inner.items.add_permits(1);
    }

    /// Atomically claim the next envelope. Returns `None` once the broker is
    /// closed and drained.
    pub(crate) async fn claim(&self) -> Option<TaskEnvelope> {
        loop {
            if let Some(envelope) = self.inner.queue.lock().unwrap().pop_front() {
                return Some(envelope);
            }
            if self.inner.closed.load(Ordering::SeqCst) {
                return None;
            }
            match self.inner.items.acquire().await {
                Ok(permit) => permit.forget(),
                Err(_) => {
                    return self.inner.queue.lock().unwrap().pop_front();
                }
            }
        }
    }

    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        self.inner.items.close();
    }

    pub fn len(&self) -> usize {
        self.inner.queue.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for TaskBroker {
    fn default() -> Self {
        Self::new()
    }
}

/// Global capacity shared by one or more schedulers/worker sets. A permit is
/// held for the whole lifetime of a submitted task (queued + running), so
/// aggregate outstanding work can never exceed `max`. Acquiring when full
/// returns [`BackpressureError`] instead of blocking.
#[derive(Debug)]
pub struct GlobalCapacity {
    permits: Arc<Semaphore>,
    max: usize,
}

impl GlobalCapacity {
    pub fn new(max: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max)),
            max,
        }
    }

    pub fn max(&self) -> usize {
        self.max
    }

    pub fn available(&self) -> usize {
        self.permits.available_permits()
    }

    pub fn try_acquire(&self) -> Result<OwnedSemaphorePermit, BackpressureError> {
        self.permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| BackpressureError)
    }
}

impl Default for GlobalCapacity {
    fn default() -> Self {
        Self::new(tokio::sync::Semaphore::MAX_PERMITS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_capacity_rejects_when_full_and_recovers() {
        let cap = GlobalCapacity::new(1);
        let permit = cap.try_acquire().expect("first permit");
        assert!(cap.try_acquire().is_err(), "full capacity must backpressure");
        drop(permit);
        assert!(cap.try_acquire().is_ok(), "capacity must recover after release");
    }

    #[tokio::test]
    async fn broker_close_unblocks_claim() {
        let broker = Arc::new(TaskBroker::new());
        let b = broker.clone();
        let claimer = tokio::spawn(async move { b.claim().await.is_none() });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        broker.close();
        let drained = tokio::time::timeout(std::time::Duration::from_secs(1), claimer)
            .await
            .expect("claim must unblock on close")
            .expect("join");
        assert!(drained);
    }
}
