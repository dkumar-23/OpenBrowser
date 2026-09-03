use std::sync::Arc;
use runtime_core::{RuntimeKernel, TaskContext};
use runtime_auth::{AgentIdentity, HumanId};
use runtime_policy::{PolicyEngine, Capability, Scope, CapabilitySet};
use runtime_interaction::{AdapterParams, InteractionAdapter, TaskInfo};
use runtime_adapters_http::HttpAdapter;
use runtime_observability::{init_tracing, TraceObservability, Observability, TraceContext};
use runtime_sandbox::ResourceQuota;

#[tokio::main]
async fn main() {
    let _ = init_tracing();

    // Wire kernel
    let observability: Arc<dyn Observability> = Arc::new(TraceObservability::default());
    let mut policy_engine = PolicyEngine::new();
    policy_engine.add_capability("http.get");
    let policy = Arc::new(policy_engine);
    let kernel = RuntimeKernel::new(policy.clone(), observability.clone());

    // Create agent with CapabilitySet
    let human = HumanId::default();
    let agent = AgentIdentity::new(human);
    let mut caps = CapabilitySet::new();
    caps.grant(Capability::new("http.get", Scope::All, Some(3600)));
    let caps = Arc::new(caps);

    // Build TaskContext (CF-7: submit via scheduler)
    let task_ctx = TaskContext::new(
        agent.agent_id.0,
        None,
        ResourceQuota::default(),
        policy.clone(),
    );
    let task_id = task_ctx.task_id;

    // Submit to scheduler
    let adapter = HttpAdapter::new(observability.clone(), policy.clone());
    let agent_clone = agent.clone();
    let caps_clone = (*caps).clone();
    let task_ctx_clone = task_ctx.clone();

    let info = TaskInfo { task_id, agent_id: agent.agent_id.0 };
    let handle = kernel.scheduler.submit(task_ctx).await.expect("submit failed");

    // Run the work async (the scheduler doesn't auto-dispatch; we keep the existing
    // direct execution to remain testable, but use the scheduler as the entry point).
    let _ = tokio::spawn(async move {
        let params = AdapterParams::Http { url: "https://example.com".into(), method: Some("GET".into()) };
        let info = TaskInfo { task_id: task_ctx_clone.task_id, agent_id: task_ctx_clone.agent_id };
        let result = adapter.execute(&agent_clone, &caps_clone, &info, &params).await;
        println!("[OpenBrowser] HTTP result: {:?}", result);
    }).await;

    let _ = handle;
    let _ = task_id;

    // Observable completion
    let trace = TraceContext::new(agent.agent_id.0, None);
    observability.log_structured(
        runtime_observability::LogLevel::Info,
        "runtime_started",
        &trace,
        &[("phase", "1")],
    );

    // Verify scheduler metrics observable
    let metrics = kernel.scheduler.metrics();
    println!("[OpenBrowser] scheduler metrics: {:?}", metrics);
}
