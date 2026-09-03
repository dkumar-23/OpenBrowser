# OpenBrowser — Coding Standard & Reference
# Author: dipesh <dipesh.kr96@gmail.com>
# Date: 2026-09-04 | Session 3 / Phase 1
# Status: DRAFT — aligns with Candidate C (Hybrid) architecture

---

## 1. Purpose
This file defines the coding standard for OpenBrowser Phase 1–6 implementation.
It references browsed external documentation and binds them to the project's
design contract (`context.md` + `architect-output.md` + `architect-synthesis/synthesis.md`).

---

## 2. External References (Browsed)

| Topic | Source URL / Doc | Project Use |
|-------|-----------------|-------------|
| Rust concurrency / threads / async | `https://doc.rust-lang.org/book/ch16-01-threads.html` | WorkerPool per-worker isolation, cancellation tokens |
| Async Rust patterns (tokio, backpressure) | `https://rust-lang.github.io/async-book/` | Scheduler submit/backpressure; adapter `execute` is async |
| HTTP protocol / request lifecycle | `https://developer.mozilla.org/en-US/docs/Web/HTTP/Overview` | `HttpAdapter` sequence: identity → policy → reqwest → replay |
| Rust std::sync (Arc, RwLock, Mutex) | `https://doc.rust-lang.org/std/sync/` | `WorkerPool` state; `ReplayWriter` sequence lock |
| Structured observability (tracing) | `https://tracing.rs/` | `TraceObservability`; structured JSON logs |
| Metrics crate (counter/gauge) | `https://metrics-rs.github.io/` | CF-5 fix: real metric increments |
| reqwest HTTP adapter patterns | `https://docs.rs/reqwest/` | `runtime-adapters-http` uses reqwest-wrapped client |
| Capability / delegation security | Capability-model docs (macaroons/policies) | CF-2: `PolicyEngine::check()` with `CapabilitySet` + chain |

See `BROWSED-REFERENCES.md` for full extracted summary.

---

## 3. First-Principles Rules (from architecture)

These rules override any convenience or benchmark optimization.

### R1 — Explicit Interfaces (No Pass-Through)
Every layer exposes a trait/struct interface. No method passes through
without enforcement. Example: `HttpAdapter::execute()` must return
`AdapterResult`, not `String`. Policy check must happen before `reqwest`.

### R2 — Capability-First Auth (Context.md §7)
`PolicyEngine` receives `AgentIdentity` + `CapabilitySet`. It never relies
on agent self-assertion. Delegation chain expiration is verified.

### R3 — Modular Boundaries (No Cross-Import Violations)
- `runtime-adapters-http` may import `runtime-policy`, `runtime-auth`, `runtime-observability`
- `runtime-adapters-http` MUST NOT import `runtime-core` scheduler
- `runtime-policy` MUST NOT import adapter/network/core
- `runtime-auth` MUST NOT import policy (only identity)

### R4 — Independent Worker State (Context.md §5 / §15)
`WorkerPool` carries `HashMap<Uuid, WorkerState>` protected by `RwLock`.Each worker has independent `ResourceQuota`, cancellation token, and crash isolation.

### R5 — Deterministic Replay (Context.md §18 / §19)
`ReplayWriter` writes JSONL with a single authoritative sequence source (`writer.next_seq()`).Replay events must contain `sequence`, `event_type`, `task_id`, `agent_id`, `timestamp`.

### R6 — Metrics Are Measurable (Not Just Logs)
`metric()` uses `metrics` crate (counter / histogram / gauge). `tracing::info!`is acceptable for structured event logging but NOT a substitute for metrics.

### R7 — Adapter Trait Before Implementation
`InteractionAdapter` trait must exist before any adapter (HTTP/MCP/visual)implements it. This prevents repeated policy-bypass errors.

---

## 4. Rust / Code Conventions

| Area | Standard | Reference |
|------|----------|-----------|
| Async | `async fn` + `tokio::spawn` + `Arc` for shared state | `async-book` |
| Error | Use `Result<T, AdapterResult>` or `Decision::Deny`; never silently ignore policy denial | `context.md` §7 |
| Concurrency | `Arc<RwLock<HashMap>>` for worker state; `Mutex<u64>` for sequence | `std::sync` docs |
| Observability | `tracing::info!` for structured events + `ReplayWriter` for audit + `metrics::counter!` for quant | `tracing.rs`, `metrics-rs` |
| Network | `reqwest::Client` (connection pool) — never create new client per request | `reqwest` docs |
| Serialization | `serde_json` for replay JSONL; `serde` derive for `ReplayEvent`, `TraceContext` | Workspace `Cargo.toml` |

---

## 5. Phase 1 Implementation Order (Aligned with Design)

Based on `swarm-output/wave3-policy-fix-design.md` and architecture:

1. `runtime-interaction` — `InteractionAdapter` trait (CF-6)
2. `runtime-adapters-http` — implement trait; fix sequence (CF-1)
3. `runtime-policy` — add `CapabilitySet` to `check()` (CF-2)
4. `runtime-observability` — fix `ReplayWriter` sequence + real `metric()` (CF-3, CF-5)
5. `runtime-core` — implement `WorkerPool` state + quotas (CF-4)
6. `runtime-cli` — submit via `scheduler.submit()` (CF-7)
7. `.graphify/graph.json` — refresh (CF-8)

---

## 6. Integration Test (Must Pass Before Phase 2)

```rust
// Agent WITHOUT capability
let agent = AgentIdentity::new("agent-bad", CapabilitySet::new());
let res = adapter.execute(&agent, "http_get", "http://example.com").await;
assert!(matches!(res, AdapterResult::Denied { .. }));
assert!(!network_called);
assert!(replay_file_contains("policy_denied"));
assert!(metric_counter("policy_denied") > 0);

// Agent WITH capability
let agent = AgentIdentity::new("agent-good", caps_with("http_get"));
let res = adapter.execute(&agent, "http_get", "http://example.com").await;
assert!(matches!(res, AdapterResult::Success { .. }));
assert!(replay_file_contains("http_executed"));
```

---

## 7. References to Project Files

- Design contract: `context.md`
- Architecture synthesis: `architect-synthesis/synthesis.md`
- Architect output: `architect-output.md`
- Phase tracker / pass conditions: `PHASE-TRACKER.md`
- Session / wave trace: `swarm-trace.md`, `SESSION-LOG.md`
- Fix design: `swarm-output/wave3-policy-fix-design.md`
- Graph state: `.graphify/graph.json`, `.graphify/GRAPH_REPORT.md`
- Code targets: `crates/runtime-adapters-http/src/lib.rs`, `runtime-policy/src/lib.rs`, `runtime-observability/src/lib.rs`, `runtime-core/src/worker.rs`

---

*This standard is binding for Phase 1 true pass. Deviations must be
documented per Phase D rules (`architect-output.md` Phase D).*
