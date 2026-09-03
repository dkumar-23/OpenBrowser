# OpenBrowser — Architect Output

**Phase A: Ground** — DONE
**Phase B: Sketch** — DONE (2 candidates + 1 synthesis)
**Phase C: Agree** — opt-in; no checkpoint requested; proceed to Phase D

---

## Ground: Vision / Goal / Requirements (from context.md)

### Vision
Build a secure, lightweight, massively concurrent machine-interaction runtime for the web. The browser is a compatibility mechanism, not the center. The runtime moves between native agent interface, HTTP/API, MCP, structured web semantics, DOM, JS, and visual fallback — without forcing every task through a graphical browser.

### 2040 Design Goal
Future where autonomous agents dominate Internet traffic. System must handle: autonomous agents, agent-to-agent communication, delegated authority, workload identity, capability-based access, policy enforcement, auditability, sandboxing, high concurrency, fault isolation, deterministic execution, massive short-lived tasks, long-running tasks, distributed execution.

### Key Requirements
1. Layered architecture with explicit interfaces (no hard coupling)
2. JavaScript engine abstracted (JsEngine trait; V8 initial impl)
3. Concurrency: workers + isolates, not single global context
4. Identity is first-class: human → agent → sub-agent → task
5. Capability/permission model enforced by runtime (not LLM self-assertion)
6. Credential isolation (token exchange, scoped/ephemeral handles)
7. Tiered browser compatibility (HTTP/fetch/DOM/Tier1 first)
8. Rendering optional — only when task requires it
9. Unified interaction API (agent doesn't know mechanism chosen)
10. MCP/CDP as adapters, not core
11. Scheduler with quotas, backpressure, cancellation
12. Sandboxing (process/worker/isolate/page layers)
13. Full observability (logs, traces, audit, replay)

---

## Phase B: Sketch

### Candidate A — Layered Compatibility-First
Vertical build matching Phases 1-6. Start with runtime-core + HTTP + scheduler; add DOM/JS; add agent interface last (Phase 3); security last (Phase 4).

### Candidate B — Agent-Native Capability-First
Invert. Capability/auth is Phase 1 (not 4). Browser/web as replaceable adapters. Semantic capabilities as first-class entry.

### Candidate C — Hybrid (SYNTHESIZED)
Capability-first at entry (B); layered replaceable internals (A). Phase 1 = kernel + scheduler + identity stub + HTTP adapter + observability. No premature complexity.

---

## Final Synthesized Design (Candidate C)

### Usage (caller-first)
```
agent registers identity + capabilities
agent invokes semantic capability (search_web, extract_page, purchase...)
runtime selects adapter (HTTP/DOM/JS/MCP/visual)
execution runs in sandboxed worker with identity/delegation chain
all delegation/policy decisions logged for replay/debug
```

### Module Map
```
runtime-auth           (identity, credential broker, token exchange, revocation)
runtime-policy         (capability model, policy engine, decision log)
runtime-agent          (agent identity, sub-agent, delegation chain, semantic registry)
runtime-core           (scheduler, task model, worker pool, cancellation, backpressure)
runtime-sandbox        (process/worker/isolate isolation, quotas, watchdogs)
runtime-interaction    (unified interaction API, adapter selection)
runtime-adapters       (HTTP, DOM, JS, MCP, CDP, visual)
runtime-observability  (logs, traces, metrics, replay, audit)
runtime-network        (TLS, cookies, redirects — owned by HTTP adapter)
runtime-js             (JsEngine trait + V8 + lightweight; isolated from DOM)
runtime-dom            (parser, mutation, events, selectors)
runtime-browser        (navigation, forms, cookies, basic Web APIs)
runtime-cli            (entry point)
```

### Type Sketch
```
struct AgentIdentity   { agent_id, sub_agent, human, chain }
struct Capability      { name, scope, expiration, policy_ref, chain }
struct AuthHandle      { opaque_token, broker, revoker }

trait SemanticCapability {
    fn authorize(&self, agent, ctx) -> Decision;
    fn execute(&self, agent, task) -> CapabilityResult;
}
trait InteractionAdapter {
    fn select(&self, action, ctx) -> bool;
    fn execute(&self, task, action) -> InteractionResult;
}
trait JsEngine { compile, execute, isolate }

struct RuntimeKernel {
    scheduler, workers, observability, policy
}
```

### Rationale
- Capability/auth are entry point, not Phase 4. Avoids retrofitting into layered web stack.
- Browser/web platform built vertically (A's approach) but behind replaceable adapters (B's approach).
- Interface depth: small high-level agent-facing API; deep but encapsulated internal subsystems.
- All 10 decision rules satisfied: replaceability, security, observability, testability, agent-native, future-compatibility.
- Phase 1 scope is achievable and testable: kernel + scheduler + identity stub + HTTP adapter + observability.

### Design Red Flags Check
- [x] No pass-through methods
- [x] No shallow modules (each crate owns a subsystem)
- [x] No temporal decomposition
- [x] No information leakage (credentials stay in auth layer)
- [x] No locked dependency (V8 behind JsEngine trait; MCP/CDP are adapters)

---

## Phase D: Next Steps (implementation order)

**Phase 1** — Runtime kernel (no full browser yet)
- runtime-core (scheduler, task model, worker pool, cancellation)
- runtime-sandbox (isolation, quotas, watchdogs)
- runtime-observability (structured logs, traces, audit)
- runtime-auth (identity stub, credential broker stub)
- runtime-policy (capability model, policy engine stub)
- runtime-adapters (HTTP adapter only)
- runtime-cli (entry point)

**Phase 2** — Web platform
- runtime-network
- runtime-dom
- runtime-js
- runtime-browser

**Phase 3** — Agent interface
- runtime-agent (semantic capabilities)
- runtime-interaction (unified API)
- runtime-mcp adapter

**Phase 4+** — Security hardening, scale, advanced browser features per context.md Phases 4-6.
