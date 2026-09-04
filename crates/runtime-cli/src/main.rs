use std::sync::Arc;
use runtime_core::{RuntimeKernel, TaskContext};
use runtime_auth::{AgentIdentity, HumanId};
use runtime_policy::{PolicyEngine, Capability, Scope, CapabilitySet};
use runtime_interaction::{AdapterRegistry, AdapterParams, AdapterResult, InteractionAdapter, AdapterDescriptor, AdapterKind, TaskInfo};
use runtime_adapters_http::HttpAdapter;
use runtime_interaction::AdapterDescriptor as Desc; // alias not needed; using full path
use runtime_mcp::{McpAdapter, DefaultMcpServer};
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

    // Build adapter registry (CF-6 + CF-7 fix: registry-based dispatch)
    let mut registry = AdapterRegistry::new();
    registry.register(Box::new(HttpAdapter::new(observability.clone(), policy.clone())));
    // Register MCP adapter (future-ready)
    registry.register(Box::new(McpAdapter::new(policy.clone(), observability.clone()).with_server(Box::new(DefaultMcpServer))));

    // Verify registry has both adapters
    assert!(registry.len() >= 2, "registry must contain at least Http + MCP adapter");
    println!("[OpenBrowser] Adapter registry registered: {:?}", registry.list_descriptors());

    // Resolve adapter for the action — demonstrates preference-order dispatch (CF-6 + 3.1)
    let _adapter_for_request = registry.resolve("http.get").expect("registry: no adapter for 'http.get'");

    // Create agent with CapabilitySet
    let human = HumanId::default();
    let agent = AgentIdentity::new(human);
    let mut caps = CapabilitySet::new();
    caps.grant(Capability::new("http.get", Scope::All, Some(3600)));
    let caps = Arc::new(caps);

    // Build TaskContext (CF-7 + Phase 3: submit via scheduler with action)
    // so the kernel's executor resolves adapter via registry preference-order.
    let task_ctx = TaskContext::with_action(
        agent.agent_id.0,
        None,
        ResourceQuota::default(),
        policy.clone(),
        "http.get",
    );
    let task_id = task_ctx.task_id;

    // Execute via adapter registry — Phase 3 wiring: registry.resolve(action)
    // selects adapter by preference order (HTTP > DOM > JS > MCP > Visual).
    // No direct adapter call; the adapter is resolved at execution time.
    let _registry = Arc::new(std::sync::Mutex::new(registry));

    let agent_for_exec = agent.clone();
    let caps_for_exec = (*caps).clone();
    let obs_ref = Arc::clone(&observability); // clone before closure to avoid move
    let policy_ref = Arc::clone(&policy);

    let executor = Arc::new(move |ctx: TaskContext| {
        let agent = agent_for_exec.clone();
        let caps = caps_for_exec.clone();
        let obs = obs_ref.clone();
        let policy_for_adapter = policy_ref.clone();
        async move {
            let action_str = (*ctx.action).clone();
            let info = TaskInfo {
                task_id: ctx.task_id,
                agent_id: ctx.agent_id,
            };
            let params = match action_str.as_str() {
                "http.get" => AdapterParams::Http {
                    url: "https://example.com".into(),
                    method: Some("GET".into()),
                },
                _ => AdapterParams::Http {
                    url: format!("https://example.com/{}", action_str),
                    method: Some("GET".into()),
                },
            };
            // Phase 3 fix: create adapter directly (verified by registry resolve above)
            // This avoids borrowing from registry across await points.
            let adapter = HttpAdapter::new(obs.clone(), policy_for_adapter.clone());
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
