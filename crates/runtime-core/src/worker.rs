use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio::sync::RwLock;
use uuid::Uuid;
use runtime_sandbox::ResourceQuota;

#[derive(Debug)]
pub struct WorkerState {
    pub quota_remaining: ResourceQuota,
    pub cancel: CancellationToken,
    pub handle: Option<JoinHandle<()>>,
}

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
    pub async fn spawn<F>(&self, task_id: Uuid, f: F) -> JoinHandle<F::Output>
    where F: std::future::Future + Send + 'static, F::Output: Send + 'static,
    {
        let cancel = CancellationToken::new();
        let handle = tokio::spawn(f);
        let state = WorkerState {
            quota_remaining: self.default_quota.clone(),
            cancel: cancel.clone(),
            handle: None,
        };
        self.workers.write().await.insert(task_id, state);
        handle
    }
    pub async fn cancel(&self, task_id: Uuid) -> bool {
        if let Some(w) = self.workers.read().await.get(&task_id) {
            w.cancel.cancel();
            true
        } else { false }
    }
    pub async fn remove(&self, task_id: Uuid) -> Option<WorkerState> {
        self.workers.write().await.remove(&task_id)
    }
    pub async fn count(&self) -> usize {
        self.workers.read().await.len()
    }
}

impl Default for WorkerPool {
    fn default() -> Self { Self::new() }
}
