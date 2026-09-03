use std::sync::Arc;
use tokio::task::JoinHandle;

/// Worker pool — independent state per worker; crash isolation.
#[derive(Debug, Default, Clone)]
pub struct WorkerPool;

impl WorkerPool {
    pub fn new() -> Self { Self }
    pub fn spawn<F>(&self, f: F) -> JoinHandle<F::Output>
    where F: std::future::Future + Send + 'static, F::Output: Send + 'static,
    {
        tokio::spawn(f)
    }
}
