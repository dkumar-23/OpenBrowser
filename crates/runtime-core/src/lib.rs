use std::sync::{Arc, RwLock};
use uuid::Uuid;
use tokio_util::sync::CancellationToken;
use runtime_observability::{TraceContext, Observability, ReplayEvent};
use runtime_sandbox::ResourceQuota;
use runtime_policy::{PolicyEngine, Decision};
use runtime_auth::AgentIdentity;

pub mod scheduler;
pub mod worker;

pub use scheduler::{Scheduler, SchedulerMetrics};
pub use worker::WorkerPool;

/// Every task carries full context through the runtime — trace/observability requirement.
#[derive(Clone, Debug)]
pub struct TaskContext {
    pub task_id: Uuid,
    pub agent_id: Uuid,
    pub delegation_id: Option<Uuid>,
    pub trace: Arc<TraceContext>,
    pub quota: ResourceQuota,
    pub cancel: CancellationToken,
    pub policy: Arc<PolicyEngine>,
}

impl TaskContext {
    pub fn new(
        agent_id: Uuid,
        delegation_id: Option<Uuid>,
        quota: ResourceQuota,
        policy: Arc<PolicyEngine>,
    ) -> Self {
        let trace = Arc::new(TraceContext::new(
            agent_id,
            delegation_id,
        ));
        Self {
            task_id: trace.task_id,
            agent_id,
            delegation_id,
            trace,
            quota,
            cancel: CancellationToken::new(),
            policy,
        }
    }
}

/// Runtime kernel — wires all Phase 1 crates together.
pub struct RuntimeKernel {
    pub scheduler: Scheduler,
    pub workers: Arc<RwLock<WorkerPool>>,
    pub observability: Arc<dyn Observability>,
    pub policy: Arc<PolicyEngine>,
}

impl RuntimeKernel {
    pub fn new(policy: Arc<PolicyEngine>, observability: Arc<dyn Observability>) -> Self {
        Self {
            scheduler: Scheduler::new(1000), // max 1000 queued tasks
            workers: Arc::new(RwLock::new(WorkerPool::new())),
            observability,
            policy,
        }
    }

    /// Enforce capability check before any task submission.
    pub fn check_capability(&self, agent: &AgentIdentity, action: &str) -> Decision {
        let d = self.policy.check(agent, action);
        self.observability.log_structured(
            runtime_observability::LogLevel::Info,
            &format!("policy_check: {}", action),
            &self.make_trace_context(agent.agent_id.0),
            &[
                ("action", action),
                match &d {
                    runtime_policy::Decision::Allow => ("decision", "allow"),
                    runtime_policy::Decision::Deny { reason: _ } => ("decision", "deny"),
                },
            ],
        );
        d
    }

    fn make_trace_context(&self, agent_id: Uuid) -> TraceContext {
        TraceContext::new(agent_id, None)
    }

    /// Record a replay event for determinism.
    pub fn record_replay(&self, event: ReplayEvent) -> u64 {
        self.observability.record_replay(event)
    }
}
