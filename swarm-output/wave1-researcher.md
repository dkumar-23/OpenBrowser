# Wave 1 — Research (low — browser/internet-based reasoning)

## Rust crate choices for Phase 1

### Async runtime + scheduling + backpressure
- `tokio` is the standard. Supports work-stealing scheduler, cancellation via `tokio::task` abort handles, resource-aware via `tokio_util` / custom queues. For 100x-1000x agent scale, tokio's multi-threaded runtime with independent tasks fits the requirement better than single-threaded `smol`.
- For backpressure: implement queue limits in our `runtime-core` scheduler; tokio provides primitives but our architecture requires explicit quotas.
- Recommendation: `tokio` (full-featured, production-grade, excellent observability via `tracing`).

### HTTP adapter (client + server if needed)
- `hyper` for server, `reqwest` for client. For agent-native interaction, we mainly need a lightweight HTTP client adapter that takes `TaskContext` (with identity/delegation) and returns `InteractionResult`.
- `reqwest` is simpler for Phase 1 (built on hyper, handles cookies/redirects/tls). For server side: `hyper` for future MCP/CDP server interfaces. Recommendation: use `reqwest` in adapter, `hyper` available but not required in Phase 1.
- Recommendation: `reqwest` for adapter (client-only Phase 1); `hyper` reserved for Phase 3/4.

### Sandbox / isolation / quotas
- Process isolation: standard Rust `std::process` + Linux namespaces/cgroups (advanced, Phase 4).
- For Phase 1: worker isolation within tokio tasks (independent state, cancellation, quota tracking in memory). No real OS-level sandbox needed yet, but design the interface so it can be added later (`WorkerGuard` trait).
- `wasmtime` or `wasmer` for future WASM isolation (Phase 5/6).
- Recommendation: in-memory isolation via `Arc<WorkerGuard>` with quota tracking; no external dependency for sandbox primitives in Phase 1 (pure Rust structures).

### Observability / tracing / replay
- `tracing` (official Rust tracing) provides structured spans and subscribers. Works with `opentelemetry` for distributed tracing.
- `metrics` crate for lightweight metrics.
- For replay/debug: custom `ReplayWriter` (JSONL / binary) that writes `ReplayEvent` sequences with deterministic IDs. No dependency needed besides `serde` (if used) or manual serialization.
- Recommendation: `tracing`, `metrics`, `serde_json` (optional for replay).

### Credential / security
- `ring` for cryptographic primitives (hashing, signatures) if needed for AuthHandle opaque tokens.
- `uuid` for IDs (`AgentId`, `TaskId`).
- `chrono` or `time` crate for expiration.
- Recommendation: `uuid`, `ring` (optional stub), `chrono` for expiration.

### JS engine (future Phase 2)
- `v8` crate (rust-v8) is complex and requires native builds. `boa` (pure Rust JS engine) is lightweight but incomplete. For Phase 1: no JS at all. Design `JsEngine` trait so either `v8` or `boa` or `deno_core` can be plugged.
- Recommendation: define trait only in Phase 1; implement `v8` adapter in Phase 2 if needed.

## License / scale compatibility
- All recommended crates (tokio, reqwest, tracing, metrics, uuid, ring, chrono) are MIT/Apache-2.0, FOSS, widely adopted.
- All are tested at very large scale (used in production systems with thousands of concurrent tasks), matching 100x-1000x agent scale target.

## Key insight for architect
Phase 1 does NOT require any heavy dependency. Only `tokio`, `reqwest`, `tracing`, `metrics`, `uuid`, and optional `ring` + `chrono`. This keeps the initial runtime lightweight — exactly what the vision demands.
