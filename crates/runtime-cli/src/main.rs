use std::sync::Arc;
use runtime_core::{RuntimeKernel, TaskContext};
use runtime_auth::{AgentIdentity, HumanId};
use runtime_policy::{PolicyEngine, Capability, Scope, CapabilitySet};
use runtime_interaction::{AdapterParams, InteractionAdapter, TaskInfo, AdapterResult};
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

    // Wire the adapter that the scheduler will invoke. The scheduler is now
    // the sole dispatch point — no independent tokio::spawn is used here.
    let adapter = Arc::new(HttpAdapter::new(observability.clone(), policy.clone()));
    let agent_for_exec = agent.clone();
    let caps_for_exec = (*caps).clone();

    // Executor: receives a TaskContext from the scheduler and runs the adapter
    // inside the scheduled task. This closure is the only place the adapter
    // is called from.
    let executor = Arc::new(move |ctx: TaskContext| {
        let adapter = adapter.clone();
        let agent = agent_for_exec.clone();
        let caps = caps_for_exec.clone();
        async move {
            let info = TaskInfo {
                task_id: ctx.task_id,
                agent_id: ctx.agent_id,
            };
            let params = AdapterParams::Http {
                url: "https://example.com".into(),
                method: Some("GET".into()),
            };
            adapter.execute(&agent, &caps, &info, &params).await
        }
    });

    // Start the dispatcher loop. After this, submit() enqueues tasks and the
    // dispatcher dequeues + runs the adapter.
    let _dispatcher = kernel.scheduler.start(executor);

    // Submit to scheduler — this is the entry point. The adapter is NOT
    // invoked from an independent tokio::spawn.
    let handle = kernel
        .scheduler
        .submit(task_ctx)
        .await
        .expect("submit failed");

    // Wait for the scheduled task to complete and report its result.
    let result: AdapterResult = handle.result.await.expect("scheduler dropped result");
    println!("[OpenBrowser] HTTP result: {:?}", result);
    let _ = task_id;

    // Observable completion
    let trace = TraceContext::new(agent.agent_id.0, None);
    observability.log_structured(
        runtime_observability::LogLevel::Info,
        "runtime_started",
        &trace,
        &[("phase", "1")],
    );

    // Verify scheduler metrics observable — completed should be 1, failed 0.
    let metrics = kernel.scheduler.metrics();
    println!("[OpenBrowser] scheduler metrics: {:?}", metrics);
}
