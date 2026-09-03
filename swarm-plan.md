# Swarm Workflow — OpenBrowser Architect / Code / Test / Review / Validate / QA

Workspace: /home/linux-user/Documents/Projects/OpenBrowser
Trace log: /home/linux-user/Documents/Projects/OpenBrowser/swarm-trace.md

# Wave 1 — Parallel Discovery & Architecture (independent)
- architect (high): Refine synthesized design (Candidate C) into concrete Rust crate boundaries, interfaces (JsEngine trait, AgentIdentity, Capability, InteractionAdapter, RuntimeKernel), Phase 1 scope.
- scout (low): Recon workspace and context.md for requirements.
- researcher (low): Identify Rust crates / libraries needed (tokio, hyper, wasm/jsc, tracing, metrics, sandbox libs).

# Wave 2 — Sequential Design Lock (depends on Wave 1)
- planner (high): Build Phase 1 implementation plan based on architect refinement + research artifacts. Must specify: crate names (runtime-core, runtime-auth, runtime-policy, runtime-observability, runtime-adapters/http, runtime-cli), file paths, trait signatures, dependency graph. Must include trace/observability hooks.
- validator (high): Validate Phase 1 plan against core vision (Rust, layer boundaries, identity/auth/policy first, lightweight, sandbox, observability, scale-ready, no monolith, no V8-in-policy leakage).

# Wave 3 — Parallel Implementation + Testing + Review (sequential dependency: must pass Wave 2)
- worker (high): Implement Phase 1 crates per plan. Must include: scheduler with quotas/backpressure, worker pool with crash isolation, structured observability (logs/traces with task/agent/delegation IDs), auth/identity stub, policy stub, HTTP adapter stub, CLI entry. All in Rust.
- tester (high): Write tests for implemented Phase 1 crates (unit for scheduler/auth/policy/observability; integration for adapter/CLI). Must test behavior not implementation, include edge cases (quota exceeded, cancellation, crash isolation, identity validation), fast (<10ms for units), isolated.
- reviewer (high): Review worker output for security, architecture (layer boundaries), maintainability, performance, observability presence, Rust safety practices (no unnecessary unsafe, safe abstractions around unsafe). Must check against design-red-flags (no pass-through, no shallow modules, no temporal decomposition, no info leakage, no locked dependency).

# Wave 4 — Sequential Validation + Final Architecture Sign-off (depends on Wave 3)
- validator (high): Full validation of Wave 3 outputs. Validate all 10 rules (Rust, layer boundaries, lightweight/scale, capability/security separation, sandbox isolation, observability, Phase compliance, scale readiness, trace/logging presence). Must check for 100x-1000x readiness (independent worker state, resource quotas, backpressure, cancellation, crash isolation). Must verify replay/debug facilities. If FAIL/PARTIAL: provide concrete fix instructions with file paths.
- architect (high): Final sign-off / ADR update. Confirm design-contract agreement between sketch and implementation. Document any Phase D deviations (surfaces deviations per Phase D rules; triggers Phase E if pattern repeats). Confirm trace/observability artifacts exist.

# Observability / Trace Requirement
Every subagent output must include:
- Completed / Files Changed / Notes
- Trace/Observability: which task/agent/delegation IDs are covered, what logs/traces/metrics are produced, whether replay/debug is supported.
- Feasibility: scale readiness assessment at current stage.
