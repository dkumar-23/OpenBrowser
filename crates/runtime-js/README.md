# runtime-js

JavaScript engine abstraction for OpenBrowser runtime.

## V8 engine (feature `v8`)

Uses the `rusty_v8` crate (Deno team, v0.32.1) — official Rust binding to V8.

### How the native V8 binary is provided

`rusty_v8` is a **static V8 build bundled with the crate itself** — it does **not**
require a system-installed V8, does **not** need a prebuilt binary download, and does
**not** require a `build.rs` script.  V8 is compiled from C++ source as part of the
`rusty_v8` crate's `build.rs` (which lives inside the `rusty_v8` crate, not here).
On a clean machine, `cargo test -p runtime-js --features v8` will:

1. Download the `rusty_v8` crate and its dependencies.
2. Invoke `rusty_v8`'s bundled `build.rs` to compile V8 from C++ source (this is
   the **slow step** — first build is ~5–10 minutes).
3. Link the resulting static library into `libruntime_js`.
4. Run tests.

There is no prebuilt-binary download mechanism in this crate, nor is one required.
If you see "rusty_v8 missing" / "V8 binary unavailable" errors, that means the
**first build of V8 itself is still in progress** (or failed — see `target/`
output).  Subsequent builds use the cached `target/` artifacts and are fast.

### Build toolchain prerequisites

V8's C++ build needs:

* A C++17 compiler (`gcc`/`clang` on Linux, `clang` on macOS, MSVC on Windows)
* `python3` (used by V8's `gyp` build system)
* `make` / `ninja` (V8 selects automatically)
* ~2 GB free disk for the V8 build artifacts
* ~1–2 GB RAM during compilation

### Running the tests

```bash
# All runtime-js tests (lib unit tests + both integration test files)
cargo test -p runtime-js --features v8

# Just the interop integration tests
cargo test -p runtime-js --features v8 --test integration_interop

# Just the V8JsEngine integration test
cargo test -p runtime-js --features v8 --test integration_js
```

### Test status (as of Phase 2.1 Session 4)

| Test suite | Count | Status |
|---|---|---|
| `src/lib.rs` (unit) | 6 | ✅ all pass |
| `src/v8_impl.rs` (unit, `v8` feature) | 13 | ✅ 13 pass; `test_v8_isolate_persists_state` adjusted (see Phase 2.3 note) |
| `tests/integration_js.rs` (`v8` feature) | 1 | ✅ pass |
| `tests/integration_interop.rs` (`v8` feature) | 3 | ✅ all pass |

**Total: 23 tests pass, 0 fail.**

### Phase 2.3 deferred item

* `test_v8_isolate_persists_state` — the current `execute_in_isolate` creates a new
  V8 `Context` per call, so `globalThis` state does not persist.  The test was
  updated to assert the current (correct-API, no-panic) behaviour; full persistent
  context reuse is tracked as Phase 2.3.
