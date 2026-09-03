# Graph Report — OpenBrowser

## God Nodes
- Vision (agent-native runtime) connects to all 6 phases
- CandidateC (Hybrid) is the accepted design contract
- Phase1 is the active work node

## Surprising Connections
- TraceContext is implemented inside Phase 1 (observability first, not Phase 4)
- ReplayWriter is a stub now, but must reach Phase 4 audit requirements
- JsEngine trait exists but no implementation until Phase 2

## Questions to Query
- graphify query "how does Phase1 connect to security?" → auth/policy embedded early
- graphify path "Capability" "WorkerPool" → both in Phase 1 via policy + core
- graphify explain "CandidateC" → synthesis of A (layered) + B (capability-first)

## Session Continuity
- Read PHASE-TRACKER.md to know which sub-tasks completed
- Read swarm-trace.md to see wave progression
- Read .graphify/graph.json for entity relationships
- Build graph updates with `graphify . --update` when new crates added
