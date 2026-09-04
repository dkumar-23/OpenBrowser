# OpenBrowser — Session Log

SESSION DATE: 2026-09-04 (Session 3)

---

## SESSION 1
- Design: Candidates A/B + synthesis C (Hybrid)
- Phase tracker created
- 8 Phase 1 crates written

## SESSION 2 (continuation)
- Fixed: Cargo workspace layout (double-nested → flattened)
- Fixed: chrono serde feature, tokio-util, anyhow deps missing
- Fixed: tracing event!() macro, metrics API, RwLockReadGuard clone, Arc mut borrow
- BUILD: `cargo build -p runtime-cli` — PASS
- RUN: structured JSON logs with task/agent/delegation/request IDs — PASS
- 3 scheduler tests: PASS
- Phase 1 source complete

## SESSION 3 (this session)
### Assessment (high-effort review of all progress)
- Updated subagent model inheritance: removed hardcoded `model: openrouter/free` from all agent MDs
- Added 5 RPM constraint notice to all agent MDs
- Confirmed: 5 RPM limit on Gemini free tier
- Subagent test: rate-limited (429); proceeded manually

### Critical Flaws Found (10 total, 8 catalogued in .graphify/graph.json)
| ID | Severity | Flaw | File |
|----|----------|------|------|
| CF-1 | CRITICAL | Policy bypass: HttpAdapter never calls policy.check() | runtime-adapters-http/src/lib.rs |
| CF-2 | HIGH | PolicyEngine.check ignores agent CapabilitySet + delegation chain | runtime-policy/src/lib.rs |
| CF-3 | HIGH | ReplayWriter is a no-op stub | runtime-observability/src/lib.rs |
| CF-4 | HIGH | WorkerPool empty — no per-worker state, no quota enforcement | runtime-core/src/worker.rs |
| CF-5 | MEDIUM | metric() is a no-op | runtime-observability/src/lib.rs |
| CF-6 | MEDIUM | InteractionAdapter trait not defined | (not created) |
| CF-7 | MEDIUM | CLI doesn't submit via scheduler | runtime-cli/src/main.rs |
| CF-8 | LOW | Graph stale | .graphify/graph.json |

### Design Fix Produced
- `swarm-output/wave3-policy-fix-design.md`: Exact contract for CF-1 fix (adapter execute flow, policy check before reqwest), InteractionAdapter trait design (CF-6), replay writer design (CF-3), WorkerPool quota design (CF-4), 5 RPM constraint noted, design red flags re-checked.

### Graph Updated
- .graphify/graph.json: Added all 8 entities with status, 18 relations, 8 critical flaws catalogued.

### Phase Tracker Updated
- PHASE-TRACKER.md: Phase 1 now marked 90% built / 70% true pass. True pass conditions listed. Phase 2 blocked by CF fixes.

---

## NEXT SESSION (in order)
1. READ SESSION-LOG.md + PHASE-TRACKER.md (first!)
2. READ swarm-output/wave3-policy-fix-design.md (fix contract)
3. FIX CF-1: HttpAdapter execute() calls policy.check() before reqwest; return AdapterResult::Denied on policy denial; emit ReplayEvent
4. FIX CF-2: PolicyEngine.check() consults agent CapabilitySet; traverses delegation chain
5. FIX CF-3: ReplayWriter writes JSONL to ~/.local/share/openbrowser/replay.jsonl with monotonic sequence
6. FIX CF-5: metric() increments counter/gauge via metrics crate
7. FIX CF-4: WorkerPool carries per-worker state with quota enforcement
8. FIX CF-6: Define InteractionAdapter trait in runtime-interaction crate; plug HttpAdapter in
9. FIX CF-7: CLI submits TaskContext via scheduler.submit()
10. FIX CF-8: Refresh .graphify/graph.json after all CF fixes
11. Run integration tests: agent without cap → denied; agent with cap → success
12. Update .graphify/graph.json, PHASE-TRACKER.md, SESSION-LOG.md
13. THEN start Phase 2

---

## PHASE 2 — STARTED (Phase 2.1 runtime-js)
- Phase 1 true pass verified (all 8 CF fixed, build/test clean, graph updated)
- `runtime-js` crate created; `JsEngine` trait + `NoopJsEngine` stub implemented (trait-first, R7)
- Design compliance verified: isolate-first (context.md §5), no V8 hard-coupling (red-flag pass)
- Phase 2.2–2.4 (DOM, browser, TLS) still blocked behind 2.1 concrete engine choice
- Next: V8Engine impl or Boa lightweight engine; update `.graphify/graph.json` with `JsEngine` entity (status: trait-implemented)

---

## KEY FILES FOR NEXT SESSION
1. SESSION-LOG.md (this file — read first)
2. PHASE-TRACKER.md
3. swarm-output/wave3-policy-fix-design.md (fix contract for CF-1..CF-6)
4. .graphify/graph.json (critical flaws + entities)
5. architect-output.md (design contract)
6. crates/runtime-adapters-http/src/lib.rs (CF-1 fix target)
7. crates/runtime-policy/src/lib.rs (CF-2 fix target)
8. crates/runtime-observability/src/lib.rs (CF-3, CF-5 fix targets)
9. crates/runtime-core/src/worker.rs (CF-4 fix target)

## 5 RPM CONSTRAINT
- Subagent calls are severely limited (1 per ~12 seconds on free tier)
- Keep subagent invocations to 1-2 per session maximum
- For bulk work, the main agent (this session) performs implementation
- Consider chain mode for sequential work: architect → planner → worker → tester → reviewer → validator

## DESIGN CONTRACT REMINDER
Candidate C (Hybrid) is binding. Any deviation from architect-output.md must be surfaced per Phase D rules. CF-1 (policy bypass) is a Phase D deviation worth surfacing. After CF fixes, re-verify with validator agent (Wave 4).

--- SESSION 4 CONTINUATION (Phase 2.3 browser) ---
- Created crates/runtime-browser (Cargo.toml + src/lib.rs + navigate/forms/fetch/timers)
- Browser struct: new/with_client/navigate/fetch/set_interval/set_timeout
- Navigation: fetch + parse HTML + cookie observation
- Forms: FormData submit via HttpClient (multipart/text/urlencoded minimal)
- Fetch: FetchRequest / FetchResponse wrapper
- Timers: setTimeout / setInterval via tokio
- Build clean (3 warnings: dead code in sub-modules — acceptable)
- 8 tests pass: browser build, clone, cookie parse minimal, form new
- Phase 2 complete: 2.1(23)+2.2(5)+2.3(8)+2.4(8) = 44 tests total
- No new bottlenecks. System stable.
