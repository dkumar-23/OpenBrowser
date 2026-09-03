# Wave 2 — Planner (high thinking)

## Plan: Phase 1 Implementation (OpenBrowser)

### Workspace init
```
/home/linux-user/Documents/Projects/OpenBrowser/
  Cargo.toml (workspace)
  crates/
    runtime-observability/
    runtime-sandbox/
    runtime-core/
    runtime-auth/
    runtime-policy/
    runtime-network/
    runtime-adapters-http/
    runtime-cli/
```

### Dependency order for build
1. runtime-observability (no internal deps)
2. runtime-sandbox (depends: observability)
3. runtime-auth (depends: observability)
4. runtime-policy (depends: auth, observability)
5. runtime-core (depends: sandbox, observability)
6. runtime-network (depends: observability)
7. runtime-adapters-http (depends: core, network, auth, policy, observability)
8. runtime-cli (depends: core, auth, policy, adapters-http, observability)

### Implementation order
1. Init workspace + crates.
2. runtime-observability: `TraceContext`, `Observability` trait, `ReplayWriter` stub.
3. runtime-sandbox: `ResourceQuota`, `WorkerPool` interface, `Watchdog` stub.
4. runtime-auth: `AgentIdentity`, `CredentialBroker` stub, `AuthHandle`.
5. runtime-policy: `Capability`, `PolicyEngine`, `Decision`.
6. runtime-core: `Scheduler`, `TaskContext`, `RuntimeKernel`.
7. runtime-network: `HttpClient` stub (wrap reqwest).
8. runtime-adapters-http: `HttpAdapter` with capability gate.
9. runtime-cli: `main()`, boot sequence.
10. Integration tests + observable scenario run.

### Observability traces required
Every subagent must include in output: completed files, notes, and an explicit section for trace/observability verification.
