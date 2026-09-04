# OpenBrowser — Phase Tracker
# Designed for MULTIPLE SESSIONS. Each phase = achievable task, completion state, pass conditions.
# Next session: read SESSION-LOG.md first, then PHASE-TRACKER.md

WORKSPACE: /home/linux-user/Documents/Projects/OpenBrowser
SESSION: 2026-09-04 (Session 4 active — Phase 2.4 complete)

---

## PHASE 1 — RUNTIME KERNEL
STATUS: TRUE PASS / 100% (Session 3 complete — all 8 CF fixed, integration tests pass, R1-R7 verified)

### 1.1 Workspace init [PASS]
- [x] Cargo workspace at /crates/
- [x] 8 Phase 1 crates
- [x] Workspace deps pinned

### 1.2 runtime-observability [PASS-but-stub]
- [x] TraceContext with all 4 IDs
- [x] Observability trait
- [x] TraceObservability + init_tracing()
- [FLAW CF-3] ReplayWriter is no-op (only debug!); needs JSONL file
- [FLAW CF-5] metric() is no-op; needs counter/gauge

### 1.3 runtime-sandbox [PASS]
- [x] ResourceQuota, Watchdog, WorkerGuard

### 1.4 runtime-auth [PASS]
- [x] AgentId, HumanId, DelegationChain, AuthHandle, CredentialBroker

### 1.5 runtime-policy [PASS-but-incomplete]
- [x] Capability, CapabilitySet, PolicyEngine, Decision
- [FLAW CF-2] PolicyEngine.check ignores agent CapabilitySet and delegation chain

### 1.6 runtime-core [PARTIAL]
- [x] TaskContext (trace, quota, cancel, policy)
- [x] Scheduler (bounded mpsc queue, backpressure semaphore, cancellation) — 3 tests PASS
- [FLAW CF-4] WorkerPool is empty stub; needs per-worker state and quota enforcement
- [FLAW CF-7] CLI doesn't submit TaskContext to scheduler

### 1.7 runtime-network [PASS]
- [x] HttpClient stub (reqwest-wrapped)

### 1.8 runtime-adapters-http [CRITICAL FLAW]
- [x] HttpAdapter struct
- [FLAW CF-1] policy Arc stored but never called before network request
- [FLAW CF-6] Not plugged into InteractionAdapter trait (trait doesn't exist)

### 1.9 runtime-cli [INCOMPLETE]
- [x] main() wiring
- [FLAW CF-7] Direct adapter call instead of scheduler.submit(TaskContext)

### PHASE 1 TRUE PASS CONDITIONS (must complete before Phase 2)
- [x] CF-1: HttpAdapter calls policy.check_with_caps() before reqwest; returns AdapterResult
- [x] CF-2: PolicyEngine.check_with_caps() consults agent CapabilitySet; delegation chain verified
- [x] CF-3: ReplayWriter writes JSONL file with monotonic sequence (writer.next_seq sole source)
- [x] CF-4: WorkerPool enforces per-worker ResourceQuota (HashMap+RwLock+Quota+Cancel)
- [x] CF-5: metric() increments counter via metrics crate
- [x] CF-6: InteractionAdapter trait defined; HttpAdapter implements
- [x] CF-7: CLI submits via scheduler.submit(TaskContext)
- [x] CF-8: Graph refreshed (new entities + relations + status)
- [x] Integration test: agent without cap → Denied, replay event recorded, metric>0
- [x] Integration test: agent with cap → Success, replay event recorded
- [x] Reviewer sign-off: R1-R7 PASS, build clean, 11 tests pass

DESIGN FIX REFERENCE: `swarm-output/wave3-policy-fix-design.md`

---

## PHASE 2 — WEB COMPATIBILITY
STATUS: STARTED (Phase 1 true pass verified; Phase 2.1 active — Session 4)

### 2.1 runtime-js [IN PROGRESS — Session 4]
- [x] Crate `runtime-js` created (workspace member)
- [x] `JsEngine` trait defined (compile/execute/isolate + `JsQuota` + `JsValue` + `JsResult`)
- [x] `NoopJsEngine` stub (Phase 1 trait-first, no engine hard-coupling — R7 / red-flag pass)
- [x] `JsIsolate` independent sandbox concept (context.md §5)
- [x] **ENGINE SWITCHED TO V8** (rusty_v8 0.32.1) — Deno's V8 binding, full ECMAScript compliance, native multi-isolate, fetch/DOM-ready
- [x] Feature flag: `v8 = ["dep:rusty_v8"]`; boa (optional alt) requires `--no-default-features`
- [x] **V8JsEngine impl** — `v8_impl.rs` complete; all 8 gap-closures implemented (see loop log)
- [x] **GAP 1 (persistent isolate)** — `Arc<Mutex<Option<OwnedIsolate>>>` stored; `execute_in_isolate` uses it
- [x] **GAP 2 (quota)** — `JsQuota` wired to `CreateParams`; memory limit set
- [x] **GAP 3 (conversion)** — `bool`, `array`, `object`, `BigInt` all handled (BigInt simplified)
- [x] **GAP 4 (TryCatch)** — JS exceptions caught → `JsError::ExecuteError`
- [x] **GAP 5 (timing)** — `Instant::now()` measures real `execution_time_ms`
- [x] **GAP 6 (module)** — `CompiledModule::from_source()` stores source; `execute` re-compiles
- [x] **GAP 7 (execute_in_isolate)** — uses passed `JsIsolate` (not fresh isolate)
- [x] **GAP 8 (init)** — `std::sync::Once` for V8 initialization
- [x] **Integration tests** — 19 pass (1 deferred: persistent context reuse; not blocking Phase 2.2)
- [x] **Reviewer sign-off** — R1-R7 PASS; CF-1..CF-8 PASS; red-flags PASS
- [ ] Persistent context reuse (new `Context::new` each call) — deferred Phase 2.3
- [ ] Phase 2.2 `runtime-dom` — next

### 2.2 runtime-dom [PASS — Session 4]
- [x] HTML parser (minimal tokenizer: tags, text, comments, quoted attrs)
- [x] DOM tree (`DomNode`: Document, Element, Text, Comment) — `Arc<RwLock<DomNode>>` for concurrent mutation
- [x] Selectors: tag, `#id`, `.class` (multi-class via whitespace split)
- [x] Mutation: `append_child`, `remove_child`, `set_text`
- [x] `EventEmitter` with `on`/`emit`, thread-safe callbacks
- [x] `HtmlParser::parse(&str) -> Result<Arc<RwLock<DomNode>>, DomError>`
- [x] 5 tests pass: tag parse, text+comment, id selector, class selector, event emit
- [ ] (Future) Nesting for closing tags (currently flat)
- [ ] (Future) Full HTML5 spec (script/style/raw-text, foreign content)

### 2.3 runtime-browser
- [ ] Navigation, cookies, forms, timers, fetch

### 2.4 runtime-network (upgrade) [PASS — Session 4]
- [x] TLS via `rustls-tls` (reqwest workspace default)
- [x] Cookie store via `cookies` reqwest feature + builder flag
- [x] Redirect policy: `reqwest::redirect::Policy::limited(10)` via builder
- [x] Compression: `gzip` + `brotli` via reqwest features
- [x] Observability: `TraceContext` injected in all requests; `tracing::info_span!` with method, url, status
- [x] `HttpClient` backed by `reqwest::Client` with full builder pattern (`ClientBuilder`)
- [x] `execute(Request) -> anyhow::Result<Response>` — general HTTP
- [x] `get(url) -> anyhow::Result<String>` — convenience wrapper
- [x] `execute_with_trace(Request, TraceContext)` — explicit context propagation
- [x] Structured `Response` with status, headers, body (bytes) for replay/logging
- [x] `Request` fluent API: `.header()`, `.text()`, `.json()`, `.timeout()`
- [x] `Response` helpers: `.text()`, `.json()`, `.is_success()`, `.header()`
- [x] 8 tests pass (6 unit + 2 integration with mockito)
- [x] Build clean: `cargo build -p runtime-network`
- [ ] (Future) Connection pooling / keepalive tuning
- [ ] (Future) Proxy support

---

## PHASE 3 — AGENT INTERFACE
### 3.1 runtime-interaction [NEW — created during CF-6 fix]
- [ ] InteractionAdapter trait
- [ ] Adapter selection logic (HTTP > DOM > JS > visual)

### 3.2 Semantic capabilities
- [ ] search_web(), extract_page(), authenticate(), submit_form(), purchase(), schedule()

### 3.3 runtime-mcp
- [ ] MCP adapter (server + client)

---

## PHASE 4 — SECURITY HARDENING
### 4.1 Credential broker (real) [Real impl upgrades Phase 1 stub]
### 4.2 Policy engine (full) [Completes CF-2]
### 4.3 Sandbox hardening (OS-level)

---

## PHASE 5 — SCALE
### 5.1 Distributed scheduler
### 5.2 Worker pool across processes
### 5.3 Global quotas + backpressure [Completes CF-4]

---

## PHASE 6 — ADVANCED BROWSER
### 6.1 Tier 2: Workers, Service Workers, IndexedDB, WebSockets, WASM
### 6.2 Tier 3: CSS, Layout, Rendering, Canvas, WebGL
### 6.3 Visual fallback

---

## OBSERVABILITY / TRACE / GRAPH STATE
- Graph: .graphify/graph.json (refreshed with critical flaws)
- Trace log: swarm-trace.md
- Design contract: architect-output.md + architect-synthesis/synthesis.md
- Fix design: swarm-output/wave3-policy-fix-design.md
- Current session: Session 3 — flaws identified + design produced. Next session fixes CF-1..CF-8.
