# CF-4 Fix Summary: Real Quota Enforcement in runtime-sandbox

## Problem
WorkerPool (and WorkerGuard) lacked real quota enforcement. The `enforce()` method was a stub returning `true`.

## Changes Made

### `crates/runtime-sandbox/src/lib.rs`

1. **Added `ResourceUsage` struct** (lines after ResourceQuota):
   - Tracks `memory_bytes`, `cpu_ms`, `wall_ms`, `network_bytes`, `requests` per worker.
   - Provides `Default` and equality.

2. **Extended `WorkerGuard`**:
   - Added `usage: ResourceUsage` field.
   - Implemented `WorkerGuard::new(quota) -> Self` constructor.
   - Implemented `add_usage(&mut self, delta: ResourceUsage)` to accumulate usage with saturation.
   - Implemented `enforce(&self) -> bool` that compares each usage counter against the corresponding quota limit and returns `false` if any limit is exceeded.

3. **Enforce logic** checks all five limits:
   - `max_memory_bytes`
   - `max_cpu_ms`
   - `max_wall_ms`
   - `max_network_bytes`
   - `max_requests`

4. **Added unit tests** (7 tests):
   - `enforce_passes_under_quota`
   - `enforce_fails_memory_exceeded`
   - `enforce_fails_cpu_exceeded`
   - `enforce_fails_wall_exceeded`
   - `enforce_fails_network_exceeded`
   - `enforce_fails_requests_exceeded`
   - `enforce_at_limit_is_ok`

## Verification

```bash
cargo test -p runtime-sandbox
```

All 7 tests pass:
```
running 7 tests
test tests::enforce_at_limit_is_ok ... ok
test tests::enforce_fails_memory_exceeded ... ok
test tests::enforce_fails_cpu_exceeded ... ok
test tests::enforce_fails_network_exceeded ... ok
test tests::enforce_fails_requests_exceeded ... ok
test tests::enforce_fails_wall_exceeded ... ok
test tests::enforce_passes_under_quota ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

## Notes
- Usage counters must be updated externally via `add_usage()` before calling `enforce()`.
- `add_usage()` uses saturating addition to prevent overflow.
- The `enforce()` check returns `false` on first exceeded limit; all limits are checked in the defined order.
- Future integration with `WorkerPool` in `runtime-core/src/worker.rs` should call `guard.enforce()` before spawning and update usage periodically.
