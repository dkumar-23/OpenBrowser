use std::sync::Arc;
use runtime_core::RuntimeKernel;
use runtime_auth::{AgentIdentity, HumanId};
use runtime_policy::{PolicyEngine, Capability, Scope};
use runtime_adapters_http::HttpAdapter;
use runtime_observability::{init_tracing, TraceObservability, Observability, TraceContext};

#[tokio::main]
async fn main() {
    let _ = init_tracing();

    // Wire kernel
    let observability: Arc<dyn Observability> = Arc::new(TraceObservability::default());
    let mut policy_engine = PolicyEngine::new();
    policy_engine.add_capability("http.get");
    let policy = Arc::new(policy_engine);
    let kernel = RuntimeKernel::new(policy.clone(), observability.clone());

    // Create agent with capability
    let human = HumanId::default();
    let agent = AgentIdentity::new(human);

    // Demonstrate capability check + adapter
    let adapter = HttpAdapter::new(observability.clone(), policy.clone());
    let result = adapter.execute(&agent, "http.get", "https://example.com").await;
    println!("[OpenBrowser] HTTP result: {}", result);

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
