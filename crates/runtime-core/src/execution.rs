use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Phase 1 gap G4: explicit execution state machine.
///
/// State diagram:
///   Created ──> Queued ──> Running ──> Completed
///                           ├──> Failed
///                           ├──> Cancelled
///                           ├──> TimedOut
///                           └──> ResourceExceeded
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExecutionState {
    Created,
    Queued,
    Running { worker_id: Uuid },
    Completed,
    Failed { error: String },
    Cancelled,
    TimedOut,
    ResourceExceeded,
}

impl ExecutionState {
    pub fn can_transition_to(&self, next: &ExecutionState) -> bool {
        match (self, next) {
            (ExecutionState::Created, ExecutionState::Queued) => true,
            (ExecutionState::Queued, ExecutionState::Running { .. }) => true,
            (ExecutionState::Queued, ExecutionState::Cancelled) => true,
            (ExecutionState::Queued, ExecutionState::TimedOut) => true,
            (ExecutionState::Running { .. }, ExecutionState::Completed) => true,
            (ExecutionState::Running { .. }, ExecutionState::Failed { .. }) => true,
            (ExecutionState::Running { .. }, ExecutionState::Cancelled) => true,
            (ExecutionState::Running { .. }, ExecutionState::TimedOut) => true,
            (ExecutionState::Running { .. }, ExecutionState::ResourceExceeded) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
struct StateWithTime {
    state: ExecutionState,
    updated_at: DateTime<Utc>,
}

impl Default for StateWithTime {
    fn default() -> Self {
        Self { state: ExecutionState::Created, updated_at: Utc::now() }
    }
}

/// Per-task execution record — tracks state machine for G4.
#[derive(Clone, Debug)]
pub struct ExecutionRecord {
    pub task_id: Uuid,
    pub state_with_time: Arc<RwLock<StateWithTime>>,
    pub worker_id: Option<Uuid>,
}

impl ExecutionRecord {
    pub fn new(task_id: Uuid) -> Self {
        Self {
            task_id,
            state_with_time: Arc::new(RwLock::new(StateWithTime::default())),
            worker_id: None,
        }
    }

    pub async fn transition(&self, next: ExecutionState) -> Result<(), &'static str> {
        let current = self.state_with_time.read().await;
        if current.state.can_transition_to(&next) {
            drop(current);
            let mut guard = self.state_with_time.write().await;
            guard.state = next;
            guard.updated_at = Utc::now();
            Ok(())
        } else {
            Err("invalid state transition")
        }
    }

    pub async fn state(&self) -> ExecutionState {
        self.state_with_time.read().await.state.clone()
    }
}
