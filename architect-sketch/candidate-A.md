# Candidate A: Layered Compatibility-First (Vertical Stack)

Usage sketch (caller first):
```
agent calls navigate(url) -> runtime selects mechanism (HTTP/DOM/JS/visual)
agent calls extract(query) -> runtime returns structured result
agent is identified; delegation/policy checked at entry
```

Module map:
- runtime-core (scheduler, isolation, observability)
- runtime-network (HTTP, cookies, TLS, redirects)
- runtime-js (JsEngine abstraction -> V8 initially)
- runtime-dom (parser, mutation, events, selectors)
- runtime-browser (navigation, forms, cookies, basic Web APIs)
- runtime-agent (semantic actions: search_web, extract_page, submit_form)
- runtime-auth (identity, delegation, policy engine)
- runtime-mcp (adapter)
- runtime-cdp (adapter)
- runtime-sandbox (quota, crash isolation, watchdog)
- runtime-observability (structured logs, traces, replay)

Type sketch:
```
trait JsEngine { ... }
struct V8Engine { ... }

struct RuntimeKernel {
    scheduler: Scheduler,
    workers: Vec<Worker>,
    observability: Observability,
}

struct AgentTask {
    agent_id: AgentId,
    delegation_chain: Vec<Delegation>,
    capabilities: CapabilitySet,
    limits: ResourceLimits,
}

trait BrowserInterface {
    navigate(url);
    inspect(query) -> ExtractedData;
    execute_script(code);
    click(selector);
    submit_form(form);
    screenshot();
}
```

Rationale:
- Matches Phase 1-6 development order (vertical build).
- Keeps browser compatibility as a first-class pathway from the start.
- Agent interface grows after core web platform exists, avoiding premature abstraction.
- Replaceability preserved: JsEngine trait; adapters for MCP/CDP.

Design red flags screened:
- No pass-through: BrowserInterface concentrates semantic actions rather than exposing internal rules.
- No temporal decomposition: modules split by architectural boundary (scheduler vs browser vs agent), not by time.
- No shallow modules: each crate owns a real subsystem.
- Info leakage: agent identity/delegation does not leak into JS engine layer; policy enforced independently.
