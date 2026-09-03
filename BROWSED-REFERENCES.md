# OpenBrowser — Browsed External References
# Compiled: 2026-09-04 | Session 3 / Phase 1
# Author: dipesh <dipesh.kr96@gmail.com>

This file lists the external documentation sources referenced when forming
the OpenBrowser Phase 1 architecture and fix contract.

---

## Rust Language & Concurrency

### 1. Rust Book — Threads & Async
**URL:** https://doc.rust-lang.org/book/ch16-01-threads.html
**Why relevant:** Per-worker isolation pattern, independent state via `Arc`.
**Project use:**
- `WorkerPool` carries `Arc<RwLock<HashMap<Uuid, WorkerState>>>`
- Each worker has independent cancellation token
- `tokio::spawn` for async tasks

### 2. Async Rust Book
**URL:** https://rust-lang.github.io/async-book/
**Why relevant:** Tokio async patterns, backpressure, futures.
**Project use:**
- Scheduler: bounded `mpsc` queue + semaphore for backpressure
- Adapter `execute()` returns `Future<Output = AdapterResult>`
- Cancellation via `tokio_util::sync::CancellationToken`

### 3. Rust std::sync
**URL:** https://doc.rust-lang.org/std/sync/
**Why relevant:** Concurrency primitives.
**Project use:**
- `Arc<T>` for shared state (PolicyEngine, Observability)
- `RwLock<T>` for WorkerPool state map (read-heavy)
- `Mutex<T>` for ReplayWriter sequence counter (write-only)

---

## Networking & HTTP

### 4. MDN — HTTP Protocol Overview
**URL:** https://developer.mozilla.org/en-US/docs/Web/HTTP/Overview
**Why relevant:** Adapter must follow HTTP request lifecycle properly.
**Project use:**
- HttpAdapter sequence: identity → policy → reqwest → replay
- TLS, cookies, redirects (Tier 1 — Phase 2)
- Compression (gzip/brotli) — reqwest features

### 5. reqwest crate documentation
**URL:** https://docs.rs/reqwest/
**Why relevant:** Already used in `runtime-network`.
**Project use:**
- Connection pool reuse (not new client per request)
- rustls-tls backend (no OpenSSL dep)
- gzip/brotli compression
- Async-first API

### 6. hyper / HTTP/2
**URL:** https://docs.rs/hyper/
**Why relevant:** Underlying HTTP transport.
**Project use:**
- For Phase 2+ when upgrading to HTTP/2

---

## Observability & Metrics

### 7. tracing crate
**URL:** https://tracing.rs/
**Why relevant:** Structured logging, spans, IDs.
**Project use:**
- `TraceContext` (task_id, agent_id, delegation_id, request_id)
- `tracing-subscriber` JSON output for production
- Span propagation across async boundaries

### 8. metrics crate
**URL:** https://metrics-rs.github.io/
**Why relevant:** Real metric counters/gauges (CF-5 fix).
**Project use:**
- `metrics::counter!("http_denied")` per policy denial
- `metrics::counter!("http_executed")` per adapter call
- `metrics::histogram!("http_duration_seconds")` for latency
- Replace current `tracing::info!` stub

---

## Security / Capability Models

### 9. Capability-Based Security Patterns
**URL:** (capability-model papers; macaroons/OAuth token-exchange patterns)
**Why relevant:** Delegation chain, scoped credentials, ephemeral tokens.
**Project use:**
- `PolicyEngine::check(agent, caps, action)` — verifies capability set
- Delegation chain with `expires_at` per link
- Short-lived credentials preferred (context.md §8)

### 10. OWASP — Credential Isolation
**URL:** https://owasp.org/ (principles)
**Why relevant:** No raw credentials in agent code.
**Project use:**
- `CredentialBroker` returns `AuthHandle` (opaque), not raw token
- Never expose raw credentials to adapter layer
- Audit log for every credential use

---

## Adapter Pattern & Rust

### 11. Rust API Guidelines — Trait Design
**URL:** https://rust-lang.github.io/api-guidelines/
**Why relevant:** Trait structure, Send/Sync bounds, naming.
**Project use:**
- `pub trait InteractionAdapter: Send + Sync`
- Method signature: `fn execute(&self, agent, cap, ctx, params) -> InteractionResult`
- Generic over return type (Result, not String)

---

## Summary Table

| Phase | Doc | Crate/Module |
|-------|-----|--------------|
| 1 | Rust Book §16, async-book | `runtime-core`, `runtime-sandbox` |
| 1 | std::sync | `runtime-core::worker`, `runtime-observability::replay` |
| 1 | tracing, metrics-rs | `runtime-observability` |
| 1 | reqwest, MDN HTTP | `runtime-adapters-http` |
| 1 | Capability model | `runtime-policy`, `runtime-auth` |
| 2 | hyper, HTTP/2 | `runtime-network` (upgrade) |
| 2 | JS engine papers | `runtime-js` (deferred) |
| 3 | MCP spec | `runtime-mcp` (deferred) |

---

*Compiled for OpenBrowser Phase 1 fix contract. Update this list as new
references become relevant.*