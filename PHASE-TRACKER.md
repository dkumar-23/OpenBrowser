# OpenBrowser — Phase Tracker
# Designed for MULTIPLE SESSIONS. Each phase = achievable task, completion state, pass conditions.
# Next session: read SESSION-LOG.md first, then PHASE-TRACKER.md

WORKSPACE: /home/linux-user/Documents/Projects/OpenBrowser
SESSION: 2026-09-04 (Session 3 active)

---

## PHASE 1 — RUNTIME KERNEL
STATUS: 90% BUILT / 70% TRUE PASS (8 critical flaws found — must fix before Phase 2)

### 1.1 Workspace init [PASS]
- [x] Cargo workspace at /crates/
- [x] 8 Phase 1 crates
- [x] Workspace deps pinned

### 1.2 runtime-observability [PASS-but-stub]
- [x] TraceContext with all 4 IDs
- [x] Observability trait
- [x] TraceObservability + init_tracing()
- [FLAW CF-3] ReplayWriter is no-op (only debug!); needs JSONL file
- [FLAW CF-5] metric() is no-op; needs counter/gauge

### 1.3 runtime-sandbox [PASS]
- [x] ResourceQuota, Watchdog, WorkerGuard

### 1.4 runtime-auth [PASS]
- [x] AgentId, HumanId, DelegationChain, AuthHandle, CredentialBroker

### 1.5 runtime-policy [PASS-but-incomplete]
- [x] Capability, CapabilitySet, PolicyEngine, Decision
- [FLAW CF-2] PolicyEngine.check ignores agent CapabilitySet and delegation chain

### 1.6 runtime-core [PARTIAL]
- [x] TaskContext (trace, quota, cancel, policy)
- [x] Scheduler (bounded mpsc queue, backpressure semaphore, cancellation) — 3 tests PASS
- [FLAW CF-4] WorkerPool is empty stub; needs per-worker state and quota enforcement
- [FLAW CF-7] CLI doesn't submit TaskContext to scheduler

### 1.7 runtime-network [PASS]
- [x] HttpClient stub (reqwest-wrapped)

### 1.8 runtime-adapters-http [CRITICAL FLAW]
- [x] HttpAdapter struct
- [FLAW CF-1] policy Arc stored but never called before network request
- [FLAW CF-6] Not plugged into InteractionAdapter trait (trait doesn't exist)

### 1.9 runtime-cli [INCOMPLETE]
- [x] main() wiring
- [FLAW CF-7] Direct adapter call instead of scheduler.submit(TaskContext)

### PHASE 1 TRUE PASS CONDITIONS (must complete before Phase 2)
- [ ] CF-1: HttpAdapter calls policy.check() before reqwest call
- [ ] CF-2: PolicyEngine.check consults agent CapabilitySet
- [ ] CF-3: ReplayWriter writes JSONL file with monotonic sequence
- [ ] CF-4: WorkerPool enforces per-worker ResourceQuota
- [ ] CF-5: metric() actually increments counter/gauge
- [ ] CF-6: InteractionAdapter trait defined; HttpAdapter implements it
- [ ] CF-7: CLI submits via scheduler; denies logged
- [ ] CF-8: Graph refreshed with new entities
- [ ] Integration test: agent without cap → denied, no network call, replay event recorded
- [ ] Integration test: agent with cap → request succeeds, replay event recorded

DESIGN FIX REFERENCE: `swarm-output/wave3-policy-fix-design.md`

---

## PHASE 2 — WEB COMPATIBILITY
STATUS: BLOCKED (waiting for Phase 1 CF fixes)

### 2.1 runtime-js
- [ ] JsEngine trait (compile, execute, isolate)
- [ ] V8Engine impl (rust-v8)
- [ ] Lightweight engine impl (boa / rhai)

### 2.2 runtime-dom
- [ ] HTML parser
- [ ] DOM tree (Node, Element, Text, Comment, Document)
- [ ] Mutation, events, selectors

### 2.3 runtime-browser
- [ ] Navigation, cookies, forms, timers, fetch

### 2.4 runtime-network (upgrade)
- [ ] TLS, cookies, redirects, compression

---

## PHASE 3 — AGENT INTERFACE
### 3.1 runtime-interaction [NEW — created during CF-6 fix]
- [ ] InteractionAdapter trait
- [ ] Adapter selection logic (HTTP > DOM > JS > visual)

### 3.2 Semantic capabilities
- [ ] search_web(), extract_page(), authenticate(), submit_form(), purchase(), schedule()

### 3.3 runtime-mcp
- [ ] MCP adapter (server + client)

---

## PHASE 4 — SECURITY HARDENING
### 4.1 Credential broker (real) [Real impl upgrades Phase 1 stub]
### 4.2 Policy engine (full) [Completes CF-2]
### 4.3 Sandbox hardening (OS-level)

---

## PHASE 5 — SCALE
### 5.1 Distributed scheduler
### 5.2 Worker pool across processes
### 5.3 Global quotas + backpressure [Completes CF-4]

---

## PHASE 6 — ADVANCED BROWSER
### 6.1 Tier 2: Workers, Service Workers, IndexedDB, WebSockets, WASM
### 6.2 Tier 3: CSS, Layout, Rendering, Canvas, WebGL
### 6.3 Visual fallback

---

## OBSERVABILITY / TRACE / GRAPH STATE
- Graph: .graphify/graph.json (refreshed with critical flaws)
- Trace log: swarm-trace.md
- Design contract: architect-output.md + architect-synthesis/synthesis.md
- Fix design: swarm-output/wave3-policy-fix-design.md
- Current session: Session 3 — flaws identified + design produced. Next session fixes CF-1..CF-8.
