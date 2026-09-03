# Wave 1 — Architect Refinement (high thinking)

## Decision
Keep Candidate C (Hybrid) as the binding contract. Refine into Rust crate map + Phase 1 contracts.

## Crate map (final)

```
runtime-cli          -> entry point, args, config, boot
runtime-core         -> RuntimeKernel, Scheduler, WorkerPool, TaskModel, cancellation, backpressure
runtime-sandbox      -> isolation primitives (process/worker/isolate); quota enforcement; watchdog
runtime-observability-> structured logs, traces, metrics, replay record, audit trail
runtime-auth         -> AgentIdentity, HumanId, DelegationChain, CredentialBroker stub, AuthHandle
runtime-policy       -> Capability, CapabilitySet, PolicyEngine, Decision, decision log
runtime-adapters     -> container; contains runtime-adapters-http (Phase 1)
runtime-network      -> TLS, cookies, redirects (used by HTTP adapter, behind trait)
runtime-js           -> JsEngine trait + V8/lightweight impls (Phase 2)
runtime-dom          -> parser/mutation/events/selectors (Phase 2)
runtime-browser      -> navigation/forms/cookies/Web APIs (Phase 2-3)
runtime-interaction  -> unified interaction API + adapter selection (Phase 3)
runtime-mcp          -> MCP adapter (Phase 3)
runtime-cdp          -> CDP adapter (Phase 4)
```

## Dependency directions (must hold)

```
runtime-cli -> runtime-core, runtime-observability
runtime-core -> runtime-sandbox, runtime-observability
runtime-adapters-http -> runtime-core, runtime-network, runtime-observability
runtime-network -> runtime-observability
runtime-auth -> runtime-observability
runtime-policy -> runtime-observability, runtime-auth (read-only identity)
runtime-adapters-http -> runtime-auth (capability check on entry)
runtime-adapters-http -> runtime-policy (decision log)
runtime-cli -> runtime-auth, runtime-policy (init identity + policy)
```

**Forbidden edges (rule):**
- auth/policy MUST NOT depend on dom, js, browser, network, adapters.
- adapters MUST NOT bypass runtime-core to call auth/policy directly except via the registered capability gate.
- JsEngine MUST NOT be a hard dependency anywhere; only via the `JsEngine` trait.

## Phase 1 scope (concrete)

| Crate                 | Includes in Phase 1                                             |
|-----------------------|-----------------------------------------------------------------|
| runtime-observability | Logger, TraceContext (task/agent/delegation IDs), MetricRecorder, ReplayWriter stub |
| runtime-sandbox       | ResourceQuota, Watchdog, WorkerGuard (RAII), crash signal hook  |
| runtime-core          | Scheduler (queue+backpressure), WorkerPool, TaskContext, cancellation token, RuntimeKernel |
| runtime-auth          | AgentId, HumanId, DelegationChain, AuthHandle, CredentialBroker stub (in-memory) |
| runtime-policy        | Capability, CapabilitySet, PolicyEngine (allow-list + scope), Decision, decision_log |
| runtime-adapters-http | HttpAdapter implementing InteractionAdapter-trait-stub for HTTP GET/POST, capability-gated, returns InteractionResult |
| runtime-cli           | main(), wires kernel, args, REPL or one-shot mode, trace init   |

Phase 1 explicitly excludes: V8, DOM, browser, rendering, MCP, CDP, JS execution, visual fallback. Only the HTTP adapter runs requests.

## Trait contracts (Phase 1)

```rust
// runtime-core/src/kernel.rs
pub struct RuntimeKernel {
    pub scheduler: Scheduler,
    pub workers: Arc<WorkerPool>,
    pub observability: Arc<Observability>,
    pub policy: Arc<PolicyEngine>,
}

pub struct TaskContext {
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub delegation_chain: DelegationChain,
    pub trace: TraceContext,
    pub quota: ResourceQuota,
    pub cancel: CancellationToken,
}

// runtime-core/src/scheduler.rs
pub struct Scheduler { /* queue with backpressure, deadlines, priorities */ }
impl Scheduler {
    pub async fn submit(&self, task: Task) -> Result<TaskHandle, BackpressureError>;
    pub fn cancel(&self, task_id: TaskId) -> bool;
    pub fn metrics(&self) -> SchedulerMetrics;
}

// runtime-sandbox/src/quota.rs
pub struct ResourceQuota {
    pub max_memory_bytes: u64,
    pub max_cpu_ms: u64,
    pub max_wall_ms: u64,
    pub max_network_bytes: u64,
    pub max_requests: u32,
}

// runtime-auth/src/identity.rs
pub struct AgentId(pub Uuid);
pub struct HumanId(pub Uuid);
pub struct DelegationChain { pub links: Vec<DelegationLink> }
pub struct AuthHandle { pub opaque: [u8; 32], pub broker: Arc<dyn CredentialBroker> }
pub trait CredentialBroker: Send + Sync { /* issue/revoke scoped creds */ }

// runtime-policy/src/capability.rs
pub struct Capability { pub name: String, pub scope: Scope, pub expiration: Option<Instant> }
pub struct CapabilitySet { pub caps: Vec<Capability> }
pub enum Decision { Allow, Deny { reason: String } }
pub struct PolicyEngine { /* allow-list + scope + expiration */ }
impl PolicyEngine {
    pub fn check(&self, agent: &AgentIdentity, action: &str) -> Decision;
}

// runtime-adapters-http/src/lib.rs
pub struct HttpAdapter { /* client, observability */ }
impl HttpAdapter {
    pub async fn execute(&self, ctx: TaskContext, req: HttpRequest)
        -> Result<HttpResponse, AdapterError>; // policy-checked internally
}

// runtime-observability/src/lib.rs
pub struct TraceContext { pub task_id, agent_id, delegation_id, request_id }
pub trait Observability: Send + Sync {
    fn log(&self, level: LogLevel, event: &str, ctx: &TraceContext, kv: &[(...)]);
    fn trace(&self, span: SpanName, ctx: &TraceContext);
    fn metric(&self, name: &str, value: f64, kv: &[(...)]);
    fn record_replay(&self, event: ReplayEvent, ctx: &TraceContext);
}
```

## Observability hook points

Every task, every policy decision, every adapter call, every cancellation, every quota violation MUST produce:
- structured log line (with task_id, agent_id, delegation_id)
- trace span (parent for nested work)
- metrics increment (counter/histogram)
- replay record (deterministic sequence id)

## Design-red-flags recheck
- [x] No pass-through methods
- [x] No shallow modules
- [x] No temporal decomposition
- [x] No info leakage (auth/policy not coupled to web layers)
- [x] No locked dependency (JsEngine behind trait, adapters in their own crate)
- [x] Replaceable internals (HTTP, JS, DOM all behind traits)
- [x] Lightweight (Phase 1 avoids V8/DOM entirely)

## Phase 1 exit criteria
1. `cargo build -p runtime-cli` succeeds on stable Rust.
2. `cargo test -p runtime-core -p runtime-auth -p runtime-policy -p runtime-sandbox -p runtime-observability -p runtime-adapters-http` all green.
3. A runnable scenario: agent "test-agent" with capability `http.get` submits a GET request, observability log shows task_id/agent_id/delegation_id, replay file written.
4. A second scenario: agent without capability submits same request, policy denies, denial logged, no network call made.
5. No `unsafe` in Phase 1 code except inside `runtime-sandbox` boundary primitives (FFI-free is fine; if any FFI later, must be `unsafe` isolated and wrapped).
