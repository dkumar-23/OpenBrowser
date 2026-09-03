# Candidate B: Agent-Native Capability-First (Inverted Stack)

Usage sketch (caller first):
```
agent registers with capability set -> runtime issues scoped credential handle
agent invokes high-level capability search_web(query) -> runtime picks mechanism (structured API > HTTP > DOM > visual) without agent knowing
execution runs inside sandboxed worker with independent identity/delegation chain
```

Module map:
- runtime-agent (agent identity, sub-agent identity, delegation chain, capability registry)
- runtime-policy (policy engine, authorization decision, audit trail, revocation)
- runtime-auth (credential broker, token exchange, ephemeral credentials, secret handles)
- runtime-capabilities (capability model, scoped credentials, permission enforcement)
- runtime-core (scheduler, worker isolation, resource quotas, cancellation, watchdog)
- runtime-interaction (unified interaction API: navigate/inspect/query/execute_script/click/fill/submit/screenshot/extract/call_api/invoke_tool)
- runtime-adapters (HTTP adapter, DOM adapter, JS adapter, MCP adapter, CDP adapter, visual adapter)
- runtime-observability (logs, traces, replay, security events)
- runtime-sandbox (process/worker/isolate isolation, quotas)

Type sketch:
```
struct Capability {
    name: String,
    scope: Scope,
    expiration: Option<Instant>,
    delegation: DelegationChain,
}

struct AgentIdentity {
    agent_id: AgentId,
    sub_agent: Option<AgentId>,
    human_identity: HumanId,
    delegation_chain: Vec<Delegation>,
}

trait InteractionAdapter {
    execute(task: AgentTask, action: SemanticAction) -> InteractionResult;
}

struct RuntimeAgent {
    agent_id: AgentId,
    capabilities: CapabilitySet,
    auth_handle: AuthHandle,
}
```

Rationale:
- Aligns directly with thesis: "future browser is secure execution/interoperability layer."
- Capability/auth is not an add-on (Phase 4) but the entry point (Phase 1).
- Browser/web compatibility is implemented as replaceable adapters (HTTP, DOM, JS) rather than the architectural center.
- Keeps semantic actions (search_web, extract_page, purchase, schedule) as first-class, with low-level browser primitives only as adapter fallbacks.
- Enables faster path to agent-native operation without requiring full browser stack first.

Design red flags screened:
- No pass-through: InteractionAdapter abstracts mechanism selection; callers don't choose DOM vs HTTP directly.
- No temporal decomposition: adapters split by protocol/mechanism, not by phase order.
- No shallow modules: auth/capability/policy are substantial subsystems.
- Info leakage: agent identity stays in agent layer; adapter layer doesn't receive raw credentials, only capability-checked handles.
