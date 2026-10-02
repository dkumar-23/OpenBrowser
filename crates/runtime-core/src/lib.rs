use std::sync::Arc;
use uuid::Uuid;
use tokio_util::sync::CancellationToken;
use runtime_observability::{TraceContext, Observability, ReplayEvent};
use runtime_sandbox::ResourceQuota;
use runtime_policy::{PolicyEngine, Decision};
use runtime_auth::AgentIdentity;

pub mod scheduler;
pub mod worker;
pub mod execution;
pub mod broker;

pub use scheduler::{Scheduler, SchedulerMetrics, BackpressureError};
pub use worker::WorkerPool;
pub use broker::{TaskBroker, GlobalCapacity};
pub use execution::{ExecutionState, ExecutionRecord};

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
    /// Action name used by the kernel's executor to resolve an adapter via
    /// the AdapterRegistry (preference order: HTTP > DOM > JS > MCP > Visual).
    /// Replaces the previous direct-adapter-call pattern that bypassed
    /// adapter selection.
    pub action: Arc<String>,
    /// Optional deadline in milliseconds from submission. G5.
    pub deadline: Option<u64>,
}

impl TaskContext {
    pub fn new(
        agent_id: Uuid,
        delegation_id: Option<Uuid>,
        quota: ResourceQuota,
        policy: Arc<PolicyEngine>,
    ) -> Self {
        Self::with_action(agent_id, delegation_id, quota, policy, "http.get")
    }

    /// Builder with explicit action — the action string is the contract between
    /// the caller and the kernel's executor. The executor resolves it via the
    /// adapter registry's preference-order selection.
    pub fn with_action(
        agent_id: Uuid,
        delegation_id: Option<Uuid>,
        quota: ResourceQuota,
        policy: Arc<PolicyEngine>,
        action: impl Into<String>,
    ) -> Self {
        Self::with_deadline(agent_id, delegation_id, quota, policy, action, None)
    }

    pub fn with_deadline(
        agent_id: Uuid,
        delegation_id: Option<Uuid>,
        quota: ResourceQuota,
        policy: Arc<PolicyEngine>,
        action: impl Into<String>,
        deadline: Option<u64>,
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
            action: Arc::new(action.into()),
            deadline,
        }
    }
}

/// Runtime kernel — wires all Phase 1 crates together.
pub struct RuntimeKernel {
    pub scheduler: Scheduler,
    pub workers: Arc<WorkerPool>,
    pub observability: Arc<dyn Observability>,
    pub policy: Arc<PolicyEngine>,
}

impl RuntimeKernel {
    /// Compose the kernel around a policy engine and an observability sink.
    ///
    /// The caller must inject the same `observability` sink into the
    /// `PolicyEngine` (via [`PolicyEngine::with_observability`]) when it
    /// constructs it; only then do `policy_decision` lifecycle/replay events
    /// and metrics reach the sink. The kernel wires the scheduler's
    /// observability so `scheduler_queue_depth` gauges are emitted.
    pub fn new(policy: Arc<PolicyEngine>, observability: Arc<dyn Observability>) -> Self {
        let workers = Arc::new(WorkerPool::new());
        Self {
            scheduler: Scheduler::new(1000, 100) // max 1000 queued, 100 concurrent
                .with_worker_pool(workers.clone())
                .with_observability(observability.clone()),
            workers,
            observability,
            policy,
        }
    }

    /// Enforce a capability check before any task submission. The decision is
    /// routed through the policy engine's capability-aware path
    /// (`check_with_caps`), which logs exactly one `policy_decision` event to
    /// the injected Observability sink. This method only adds a structured log
    /// and never emits a second `policy_decision` event.
    pub fn check_capability(
        &self,
        agent: &AgentIdentity,
        caps: &runtime_policy::CapabilitySet,
        action: &str,
    ) -> Decision {
        let d = self.policy.check_with_caps(agent, caps, action);
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