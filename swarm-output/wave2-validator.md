# Wave 2 — Validator (high thinking)

## Validation: Phase 1 Plan vs Core Vision

| Rule | Result | Note |
|------|--------|------|
| 1. Rust-only | PASS | All crates Rust; no JS, Python, shell magic. |
| 2. Layer boundaries | PASS | auth/policy independent of network/adapters. Dep directions respected. |
| 3. Lightweight / scalable | PASS | Phase 1 has 6 internal deps; no V8, no DOM, no browser. 100x-1000x via per-task quotas. |
| 4. Capability/security separation | PASS | Capability checked inside `HttpAdapter::execute` before any network call. |
| 5. Sandbox isolation | PASS | `ResourceQuota` per task; `WorkerPool` keeps independent state; cancellation token. |
| 6. Observability | PASS | `TraceContext` carries task/agent/delegation IDs through every layer. |
| 7. Phase compliance | PASS | Phase 1 = kernel + scheduler + identity stub + HTTP adapter + observability. No browser. |
| 8. Scale readiness 100x-1000x | PARTIAL | Plan correct, but requires `tokio` multi-thread runtime + bounded scheduler queue to be verified in implementation. |
| 9. Replaceable internals | PASS | `JsEngine` trait deferred; HTTP adapter isolated; network trait-bound. |
| 10. Replay/debug | PASS | `ReplayWriter` stub added; deterministic IDs. |

## Overall: PASS with one PARTIAL on scale verification (gated on Wave 3 impl).

## Issues Found
- None blocking.
- Note: ensure `WorkerPool` enforces per-worker state isolation (no shared mutable state across workers) and `Scheduler` uses bounded queue with explicit backpressure.

## Recommendations for Wave 3 (worker)
- Use `tokio::sync::Semaphore` or bounded `mpsc` for backpressure.
- Use `tokio_util::sync::CancellationToken` for cancellation.
- Add `tracing::instrument` on adapter entry points so spans are auto-created.
- Replay writer should write JSONL with task_id, agent_id, delegation_id, action, result.

## Trace/Observability expectation
Every Wave 3 output must include which file owns the trace emission (log/trace/metric/replay) and which task/agent/delegation IDs are propagated.
