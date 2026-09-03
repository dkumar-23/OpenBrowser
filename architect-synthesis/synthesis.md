# Candidate C: Hybrid — Capability-First Entry, Compatibility-First Internals

Usage sketch (caller first):
```
agent registers identity + capabilities (runtime-auth + runtime-policy)
agent invokes semantic capability (runtime-agent: search_web, extract_page, authenticate, purchase)
runtime-core schedules task in sandboxed worker (runtime-sandbox)
runtime-interaction picks adapter (HTTP, DOM, JS, MCP, visual)
runtime-observability logs every delegation/policy decision
replay/debug: deterministic record of request metadata + tool calls + policy decisions
```

Module map (numbered, with explicit boundary ownership):
1. runtime-auth (identity, credential broker, token exchange, secret handles, revocation)
2. runtime-policy (capability model, policy engine, decision log, scope/expiration)
3. runtime-agent (agent identity, sub-agent identity, delegation chain, semantic capability registry)
4. runtime-core (scheduler, task model, worker pool, cancellation, backpressure, deadlines)
5. runtime-sandbox (process/worker/isolate isolation, quotas, watchdogs, crash isolation)
6. runtime-interaction (unified interaction API, adapter selection)
7. runtime-adapters (HTTP, DOM, JS, MCP, CDP, visual)
8. runtime-observability (logs, traces, metrics, replay, audit trail, security events)
9. runtime-network (TLS, cookies, redirects, compression — owned by HTTP adapter)
10. runtime-js (JsEngine trait + V8 impl + lightweight impls; isolated from DOM)
11. runtime-dom (parser, mutation, events, selectors)
12. runtime-browser (navigation, forms, cookies integration, basic Web APIs — composes dom+js+network)
13. runtime-cli (entry point)
14. runtime-mcp, runtime-cdp (adapters, never core)

Type sketch:
```
struct AgentIdentity { agent_id, sub_agent: Option<AgentId>, human: HumanId, chain: Vec<Delegation> }
struct Capability { name, scope, expiration, policy_ref, chain }
struct AuthHandle { opaque_token, broker: CredentialBroker, revoker }

trait SemanticCapability {
    fn authorize(&self, agent: &AgentIdentity, ctx: &TaskContext) -> Decision;
    fn execute(&self, agent: &AgentIdentity, task: AgentTask) -> CapabilityResult;
}

trait InteractionAdapter {
    fn select(&self, action: &SemanticAction, ctx: &TaskContext) -> bool;
    fn execute(&self, task: AgentTask, action: SemanticAction) -> InteractionResult;
}

trait JsEngine { compile, execute, isolate }
struct V8Engine;
struct LightweightEngine;

struct RuntimeKernel {
    scheduler: Scheduler,
    workers: WorkerPool,
    observability: Observability,
    policy: PolicyEngine,
}
```

Rationale (synthesized from A + B):
- B's capability/auth-first entry without abandoning A's compatibility path. Capability and policy are not Phase-4 add-ons; they are the entry point. But the actual HTTP/DOM/JS web platform is built the same way A proposes (vertical, layered, replaceable).
- Interface depth: agent-facing API is small and high-level (search_web, extract_page, purchase). Internal capability/identity/auth/policy subsystems are deep but encapsulated. The public surface is smaller than either A or B alone.
- Adapters (HTTP, DOM, JS, MCP, CDP, visual) are interchangeable; the core is usable without any of them.
- Meets all 10 decision rules in context.md: replaceability, security, observability, testability, agent-native operation, future-compatibility.
- Avoids A's risk of delaying auth/capability work to Phase 4 (which would be hard to retrofit into a layered web stack), and avoids B's risk of treating the web platform as an afterthought.
- Phase 1 now becomes: kernel + scheduler + task model + identity stub + HTTP adapter + observability. No JS yet. No DOM. No rendering. Browser is a growing capability, not the center.

Design red flags screened:
- Pass-through: none. Each layer has real responsibility.
- Shallow modules: each crate is justified.
- Temporal decomposition: split by boundary, not by build order.
- Info leakage: agent identity never flows into adapters as raw credentials; only capability-checked handles.
- Locked dependency: V8 is behind JsEngine trait; MCP/CDP are adapters, not core.

## Synthesis decision

Pick **C: Hybrid**. B's inversion gives us agent-native semantics at the entry, A's layering gives us replaceable internals. Best of both. Implementation proceeds against this sketch; design-red-flags pass.

## Notes for Phase D (implementation)
- Phase 1 crates: runtime-core, runtime-auth, runtime-policy, runtime-agent, runtime-observability, runtime-sandbox, runtime-adapters (HTTP only), runtime-cli.
- Phase 2 adds runtime-js, runtime-dom, runtime-browser.
- Phase 3 adds semantic capabilities and MCP adapter.
- Phases 4-6 from context.md align with the original sequence but auth/policy come first.
