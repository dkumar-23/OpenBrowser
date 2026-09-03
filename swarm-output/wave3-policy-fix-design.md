# Wave 3 — Policy Enforcement Fix Design (5 RPM constraint noted)
Agent: architect (high thinking) | Skill references: pstack-architect REFERENCE.md (red flags, redesign rules), pstack-swarm (parallel not needed — single focused fix), multiagent (collaboration for cross-layer contract)

## Critical Finding Confirmed
`crates/runtime-adapters-http/src/lib.rs` holds `Arc<PolicyEngine>` but executes `self.client.get(url)` without calling any `policy.check()`. This violates the core vision (context.md point 7: "An LLM saying 'I am allowed' is never sufficient authorization. Authorization must be enforced by the runtime.")

## Design Fix (No code written — contract only)

### 1. Adapter Execute Flow Contract (exact sequence)
`HttpAdapter::execute(agent_identity: &AgentIdentity, action: &str, url: &str) -> AdapterResult`

Sequence (must execute in this exact order; any deviation triggers Phase E redesign):
1. Receive `AgentIdentity` (contains agent_id, human, delegation_chain).
2. Call `self.policy.check(agent_identity, action)`.
3. If `Decision::Deny { reason }`: 
   - Do NOT call `self.client.get()`.
   - Create `ReplayEvent { sequence: next_seq(), event_type: "policy_denied", task_id: ..., agent_id: ..., result_summary: reason.clone() }`.
   - Call `self.observability.record_replay(...)`.
   - Return `AdapterResult::Denied { reason, replay_sequence: seq }`.
4. If `Decision::Allow`:
   - Call `self.client.get(url)`.
   - Create `ReplayEvent { sequence: next_seq(), event_type: "http_executed", task_id: ..., agent_id: ..., result_summary: "success" }`.
   - Call `self.observability.record_replay(...)`.
   - Return `AdapterResult::Success { response: body, replay_sequence: seq }`.

### 2. InteractionAdapter Trait Requirement (Phase 3 prerequisite)
To prevent future adapters (MCP, CDP, visual) from repeating the same bypass, define this trait in `runtime-interaction` (to be created):

```
trait InteractionAdapter: Send + Sync {
    fn execute(
        &self,
        agent: &AgentIdentity,
        action: Capability,
        ctx: &TaskContext,
        params: &AdapterParams,
    ) -> InteractionResult;
}
```
Every adapter MUST implement it. The adapter selects mechanism; the agent selects capability; the runtime selects adapter based on capability + mechanism availability. This satisfies the vision's point 11 ("agent should not need to understand mechanism") and point 12 ("MCP/CDP as adapters, never core").

### 3. Layer Boundary Enforcement (red flags check)
- `runtime-adapters-http` may import `runtime-policy` and `runtime-auth`. It MUST NOT import `runtime-core` scheduler or `runtime-js`. (Verified: current imports are policy, auth, core, network, observability — core import is only for RuntimeKernel which adapter does NOT actually use. Remove `runtime-core` from adapter dependencies to eliminate coupling risk.)
- `runtime-policy` MUST NOT import any adapter, network, or core scheduler. (Verified.)
- `runtime-auth` MUST NOT import policy (only identity — verified). But `AgentIdentity` needs capability attachment? No — capabilities live in `runtime-policy` (`CapabilitySet`). The adapter must receive both identity and capability set separately. This prevents info leakage (red flag: no internal rules exposed to callers).

### 4. Replay / Audit Trail (current stub improvement)
Current `ReplayWriter` is a stub (`tracing::debug!`). The fix: create `ReplayWriter` struct in `runtime-observability` with a `File` handle (JSONL format) that writes monotonically increasing sequence IDs tied to `TraceContext`. Every `record_replay()` writes to disk. Next session must verify: replay file exists at `~/.local/share/openbrowser/replay.jsonl` with deterministic sequence IDs. This satisfies the vision's auditability/replay requirement (point 18, 19).

### 5. Scale Readiness Check (100x-1000x agent readiness)
Current `HttpAdapter` creates no new thread per call; it uses `reqwest` which uses `tokio`'s connection pool. This is fine. But `WorkerPool` is empty (`WorkerPool::new()` does nothing; `spawn()` just `tokio::spawn`). For 100x-1000x concurrency, `WorkerPool` must enforce:
- Per-worker `ResourceQuota` (memory, CPU time, wall time, network bytes, max requests).
- Independent cancellation token per worker.
- No shared mutable state between workers (current `Arc<RwLock<WorkerPool>>` is okay but `WorkerPool` has no internal state — it needs at least a `HashMap<Uuid, WorkerContext>` protected by `RwLock`).
Without this, a malicious agent could submit infinite concurrent tasks and exhaust memory/network. This is a **critical flaw** if not fixed before Phase 2.

### 6. What to Update / Improve (prioritized list for next session)
A. Fix adapter policy bypass (design above — highest priority).
B. Create `InteractionAdapter` trait and plug `HttpAdapter` into it.
C. Implement real replay writer (file-based JSONL).
D. Implement real metrics (counter/gauge increment per adapter call, per policy deny).
E. Fix `WorkerPool` to enforce quotas and independent state.
F. Refresh `.graphify/graph.json` with new entities: `InteractionAdapter`, `ReplayWriter` (with status updated from stub to implemented), `HttpAdapter` (with relation to policy check).
G. Update `swarm-output/` with this design.
H. Update `PHASE-TRACKER.md`: Phase 1 pass conditions should include "Policy enforced in adapter" and "Replay writer writes to file".
I. Next phase (Phase 2: Web Compatibility) should NOT start until A-H are verified.

### 7. Design Red Flags Re-Check (post-fix)
- [PASS] No pass-through: adapter must call policy before mechanism.
- [PASS] No shallow modules: policy, auth, adapter, core, sandbox each have real responsibility.
- [PASS] No temporal decomposition: adapter design is mechanism-independent; policy is capability-independent.
- [PASS] No info leakage: adapter receives identity + capability; adapter does NOT receive raw credential strings; adapter does NOT expose internal mechanism selection to agent.
- [PASS] No locked dependency: `JsEngine` trait deferred; adapter is pluggable; network is behind trait if needed.
- [PASS] Replaceable internals: `HttpAdapter` is one adapter; future adapters (MCP, visual) can implement `InteractionAdapter`.
- [PASS] Lightweight: Phase 1 avoids V8/DOM; adapter uses existing reqwest pool.

### 8. Multi-Session Continuity Note
This design document (`swarm-output/wave3-policy-fix-design.md`) must be read by the next session before any implementation. It replaces `swarm-output/wave2-planner.md` for Phase 1 refinement. The next session should: (1) Read this file. (2) Fix adapter code per section 1. (3) Implement replay writer per section 4. (4) Implement worker quotas per section 5. (5) Verify with `cargo test -p runtime-adapters-http` (new integration test: deny without network call). (6) Verify replay file exists. (7) Update tracker and graph.

---
Note: 5 RPM constraint enforced. This design was produced in a single focused subagent call (1 of 5 available RPM). All references to skills/workflows are explicit. No code written.
