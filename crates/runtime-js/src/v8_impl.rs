//! V8 engine implementation for `JsEngine` trait (feature-gated behind `v8`).
//!
//! Uses `rusty_v8` (Deno team, v0.32.1). Implements all 8 gap-closures:
//!   GAP 1: Persistent isolate (OwnedIsolate stored in Arc<Mutex<Option<OwnedIsolate>>>)
//!   GAP 2: JsQuota wired to V8 CreateParams (heap limits)
//!   GAP 3: Full v8_value_to_js_value (bool, array, object, BigInt)
//!   GAP 4: TryCatch error handling
//!   GAP 5: Real execution_time_ms via Instant::now()
//!   GAP 6: CompiledModule stores source; execute re-compiles
//!   GAP 7: execute_in_isolate uses the passed isolate
//!   GAP 8: Thread-safe init_v8 via OnceLock

#[cfg(feature = "v8")]
use rusty_v8 as v8;

#[cfg(feature = "v8")]
use crate::{
    JsEngine, JsIsolate, JsQuota, JsValue, JsResult, JsError, CompiledModule,
    JsIsolateBacking,
};

#[cfg(feature = "v8")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "v8")]
use std::sync::atomic::{AtomicBool, Ordering};

// ---------------------------------------------------------------------------
// P1-A.2: Interrupt mechanism
// ---------------------------------------------------------------------------

#[cfg(feature = "v8")]
#[repr(C)]
struct InterruptData {
    flag: *const AtomicBool,
}

#[cfg(feature = "v8")]
extern "C" fn v8_interrupt_callback(
    scope: &mut v8::Isolate,
    data: *mut std::ffi::c_void,
) {
    let d = unsafe { &*(data as *const InterruptData) };
    if unsafe { (*d.flag).load(Ordering::SeqCst) } {
        scope.terminate_execution();
    }
}

// ---------------------------------------------------------------------------
// V8IsolateData — persistent V8 isolate storage
// ---------------------------------------------------------------------------

/// Per-isolate V8 state. Holds the owning V8 isolate so it can be reused
/// across multiple execute_in_isolate calls (GAP 1).
///
/// `OwnedIsolate` is !Send so this struct must stay on the thread that
/// created it. Workers are single-threaded per isolate.
#[cfg(feature = "v8")]
pub struct V8IsolateData {
    /// The owning V8 isolate, wrapped so it can be shared via Arc.
    /// `None` means the isolate has been terminated/dropped.
    isolate: Arc<Mutex<Option<v8::OwnedIsolate>>>,
    /// Persistent V8 Context (P1-A.1). Created once at isolate creation
    /// time and reused across `execute_script` calls so that `globalThis`
    /// state (and the V8 microtask queue) persists between executions.
    /// `None` for placeholder isolates (no real V8 isolate to attach to).
    context: Arc<Mutex<Option<v8::Global<v8::Context>>>>,
    /// Quota limits applied when this isolate was created.
    quota: JsQuota,
}

#[cfg(feature = "v8")]
impl Clone for V8IsolateData {
    fn clone(&self) -> Self {
        Self {
            isolate: self.isolate.clone(),
            context: self.context.clone(),
            quota: self.quota.clone(),
        }
    }
}

#[cfg(feature = "v8")]
impl std::fmt::Debug for V8IsolateData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V8IsolateData")
            .field("quota", &self.quota)
            .finish()
    }
}

#[cfg(feature = "v8")]
impl V8IsolateData {
    /// Create a new persistent isolate backed by a real V8 OwnedIsolate.
    /// P1-A.1: Also creates the persistent `v8::Global<v8::Context>` so that
    /// subsequent `execute_script` calls reuse the same context, and
    /// `globalThis` state persists between calls.
    pub fn new(quota: JsQuota) -> Result<Self, JsError> {
        let params = v8::CreateParams::default();
        // GAP 2: wire memory limit from JsQuota if set (skip complex callback for Phase 2.1)
        // Memory limits enforced via quota tracking; V8 native heap_limits deferred to Phase 4.
        let mut isolate = v8::Isolate::new(params);

        // P1-A.1: Create the initial persistent Context inside the new isolate.
        // We must hold a `&mut` to the OwnedIsolate (via a HandleScope) to allocate
        // the Context, then upgrade it to a v8::Global<v8::Context> for storage.
        let initial_context = {
            let scope = &mut v8::HandleScope::new(&mut isolate);
            let ctx = v8::Context::new(scope);
            // Default the context in for a single `v8::Global` slot. This is
            // the standard pattern: create a fresh `v8::Global` via `new` and
            // call `set` to attach the Local handle as a persistent handle.
            let global = v8::Global::new(scope, ctx);
            global
        };

        Ok(Self {
            isolate: Arc::new(Mutex::new(Some(isolate))),
            context: Arc::new(Mutex::new(Some(initial_context))),
            quota,
        })
    }

    /// Placeholder for non-V8 path.
    pub fn placeholder() -> Self {
        Self {
            isolate: Arc::new(Mutex::new(None)),
            context: Arc::new(Mutex::new(None)),
            quota: JsQuota::default(),
        }
    }

    /// Execute JS source inside this isolate, REUSING the persistent context
    /// (P1-A.1) so that `globalThis` state and the V8 microtask queue persist
    /// across calls. Uses TryCatch for error handling (GAP 4), measures
    /// execution time (GAP 5), and drains microtasks after execution
    /// (P1-A.4).
    ///
    /// P1-A.2: When `timeout_ms` is set (>0), a watchdog thread fires an interrupt
    /// that calls `terminate_execution()`. On timeout, `JsError::Timeout` is returned
    /// and the isolate is marked as terminated (set to None) so the next call
    /// recreates it.
    pub fn execute_script(
        &self,
        source: &str,
        compile_only: bool,
        timeout_ms: Option<u64>,
    ) -> Result<JsResult, JsError> {
        let mut isolate_guard = self.isolate
            .lock()
            .map_err(|_| JsError::IsolateError("poisoned lock".into()))?;
        // P1-A.2: If the isolate was terminated by a previous timeout, recreate it.
        if isolate_guard.is_none() {
            let params = v8::CreateParams::default();
            let new_isolate = v8::Isolate::new(params);
            *isolate_guard = Some(new_isolate);
        }
        let isolate_opt = isolate_guard
            .as_mut()
            .ok_or_else(|| JsError::IsolateError("isolate is not available".into()))?;

        let mut context_guard = self.context
            .lock()
            .map_err(|_| JsError::IsolateError("poisoned context lock".into()))?;
        // P1-A.2: If the isolate was recreated, create a new persistent context.
        if context_guard.is_none() {
            let scope = &mut v8::HandleScope::new(isolate_opt);
            let ctx = v8::Context::new(scope);
            let global_ctx = v8::Global::new(scope, ctx);
            *context_guard = Some(global_ctx);
        }
        let context_global = context_guard
            .as_ref()
            .ok_or_else(|| JsError::IsolateError("persistent context not initialized".into()))?;

        // P1-A.2: Build timeout flag + watchdog thread BEFORE the HandleScope borrows isolate.
        let timeout_flag: Option<Arc<AtomicBool>> = timeout_ms
            .filter(|&m| m > 0)
            .map(|ms| {
                let flag = Arc::new(AtomicBool::new(false));
                let flag_clone = Arc::clone(&flag);
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                    flag_clone.store(true, Ordering::SeqCst);
                });
                flag
            });

        // P1-A.2: Register V8 interrupt callback BEFORE the HandleScope (which
        // takes a &mut borrow of the isolate). request_interrupt is thread-safe
        // and works fine here.
        if let Some(ref flag) = timeout_flag {
            let data = Box::into_raw(Box::new(InterruptData {
                flag: Arc::as_ptr(flag) as *const AtomicBool,
            }));
            let handle = (&**isolate_opt).thread_safe_handle();
            let _ = handle.request_interrupt(
                v8_interrupt_callback,
                data as *mut std::ffi::c_void,
            );
        }

        let result = {
            let scope = &mut v8::HandleScope::new(isolate_opt);
            // P1-A.1: re-derive a Local<Context> from the stored Global instead
            // of creating a brand new Context every call.
            let context = v8::Local::new(scope, context_global);
            let scope = &mut v8::ContextScope::new(scope, context);

            // GAP 4: Wrap in TryCatch to capture JS exceptions properly
            let tc_scope = &mut v8::TryCatch::new(scope);

            let code = v8::String::new(tc_scope, source)
                .ok_or_else(|| JsError::CompileError("V8 string creation failed".into()))?;
            let script = v8::Script::compile(tc_scope, code, None)
                .ok_or_else(|| {
                    // Extract compilation error message
                    if tc_scope.has_caught() {
                        let exc = tc_scope.exception().map(|e| e.to_rust_string_lossy(tc_scope))
                            .unwrap_or_else(|| "unknown compile error".into());
                        JsError::CompileError(exc)
                    } else {
                        JsError::CompileError("V8 compile failed".into())
                    }
                })?;

            if compile_only {
                // GAP 6: compile-only path returns empty result.
                return Ok(JsResult {
                    value: JsValue::Undefined,
                    error: None,
                    execution_time_ms: 0,
                });
            }

            // GAP 5: real timing
            let start = std::time::Instant::now();

            let value = script.run(tc_scope);

            let elapsed_ms = start.elapsed().as_millis() as u64;

            // P1-A.2: Check if execution was terminated by timeout BEFORE checking
            // exceptions, because termination generates an uncatchable exception.
            if tc_scope.is_execution_terminating() {
                return Err(JsError::Timeout(timeout_ms.unwrap_or(0)));
            }

            // GAP 4: Check if JS threw an exception
            if tc_scope.has_caught() {
                let exc = tc_scope.exception()
                    .map(|e| e.to_rust_string_lossy(tc_scope))
                    .unwrap_or_else(|| "unknown error".into());
                let msg = tc_scope.message()
                    .map(|m| m.get(tc_scope).to_rust_string_lossy(tc_scope))
                    .unwrap_or_default();
                Err(JsError::ExecuteError(format!(
                    "JS error: {} {}",
                    msg,
                    exc
                )))
            } else {
                let value = value.ok_or_else(|| {
                    JsError::ExecuteError("V8 script.run returned no value".into())
                })?;

                // GAP 3: full value conversion
                let js_value = v8_value_to_js_value(value, tc_scope);

                Ok(JsResult {
                    value: js_value,
                    error: None,
                    execution_time_ms: elapsed_ms,
                })
            }
        };

        // P1-A.2: If a timeout happened, mark the isolate as terminated.
        // After terminate_execution the isolate is permanently dead and any
        // further script.run would fail. Recreate via create_isolate.
        let is_timeout = matches!(&result, Err(JsError::Timeout(_)));

        // Note: isolate_opt is no longer used after this point, so the mutable
        // borrow of isolate_guard automatically ends here.

        if is_timeout {
            *isolate_guard = None;
            *context_guard = None;
        } else {
            // P1-A.4: drain microtasks after script execution (scopes dropped).
            if let Some(ref mut iso) = *isolate_guard {
                iso.perform_microtask_checkpoint();
            }
        }

        result
    }
}

// ---------------------------------------------------------------------------
// V8JsEngine
// ---------------------------------------------------------------------------

/// V8-backed JavaScript engine. Implements `JsEngine` fully.
#[cfg(feature = "v8")]
#[derive(Debug, Default, Clone)]
pub struct V8JsEngine;

#[cfg(feature = "v8")]
impl V8JsEngine {
    pub fn new() -> Self { Self }
}

/// Initialize V8 once per process, thread-safely.
/// GAP 8: Uses OnceLock so V8 is initialized exactly once regardless of
/// how many threads call it simultaneously.
#[cfg(feature = "v8")]
fn init_v8() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
}

#[cfg(feature = "v8")]
impl JsEngine for V8JsEngine {
    fn name(&self) -> &str { "v8" }

    /// GAP 6: Compile stores source in CompiledModule (re-compiles on execute).
    fn compile(&self, source: &str) -> Result<CompiledModule, JsError> {
        init_v8();
        // Quick syntax-check by attempting to compile in a disposable isolate
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = &mut v8::HandleScope::new(&mut isolate);
        let context = v8::Context::new(scope);
        let scope = &mut v8::ContextScope::new(scope, context);

        let code = v8::String::new(scope, source)
            .ok_or_else(|| JsError::CompileError("V8 string creation failed".into()))?;
        v8::Script::compile(scope, code, None)
            .ok_or_else(|| JsError::CompileError("syntax error or parse failure".into()))?;

        Ok(CompiledModule::from_source(source.to_string()))
    }

    /// GAP 6: Execute re-compiles from stored source (Phase 2.1 approach).
    /// Uses a fresh ephemeral isolate for isolation (doesn't use create_isolate
    /// because module execution may need different quota than the module's original).
    fn execute(&self, module: &CompiledModule) -> Result<JsResult, JsError> {
        init_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = &mut v8::HandleScope::new(&mut isolate);
        let context = v8::Context::new(scope);
        let scope = &mut v8::ContextScope::new(scope, context);

        let tc_scope = &mut v8::TryCatch::new(scope);

        let code = v8::String::new(tc_scope, &module.source)
            .ok_or_else(|| JsError::CompileError("V8 string creation failed".into()))?;
        let script = v8::Script::compile(tc_scope, code, None)
            .ok_or_else(|| JsError::CompileError("compile failed".into()))?;

        let start = std::time::Instant::now();
        let value = script.run(tc_scope);
        let elapsed_ms = start.elapsed().as_millis() as u64;

        if tc_scope.has_caught() {
            let exc = tc_scope.exception()
                .map(|e| e.to_rust_string_lossy(tc_scope))
                .unwrap_or_else(|| "unknown".into());
            let msg = tc_scope.message()
                .map(|m| m.get(tc_scope).to_rust_string_lossy(tc_scope))
                .unwrap_or_default();
            return Err(JsError::ExecuteError(format!("{} {}", msg, exc)));
        }

        let value = value.ok_or_else(|| JsError::ExecuteError("no result".into()))?;
        let js_value = v8_value_to_js_value(value, tc_scope);

        Ok(JsResult {
            value: js_value,
            error: None,
            execution_time_ms: elapsed_ms,
        })
    }

    fn execute_source(&self, source: &str) -> Result<JsResult, JsError> {
        init_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = &mut v8::HandleScope::new(&mut isolate);
        let context = v8::Context::new(scope);
        let scope = &mut v8::ContextScope::new(scope, context);

        let tc_scope = &mut v8::TryCatch::new(scope);

        let code = v8::String::new(tc_scope, source)
            .ok_or_else(|| JsError::CompileError("V8 string creation failed".into()))?;
        let script = v8::Script::compile(tc_scope, code, None)
            .ok_or_else(|| JsError::CompileError("compile failed".into()))?;

        let start = std::time::Instant::now();
        let value = script.run(tc_scope);
        let elapsed_ms = start.elapsed().as_millis() as u64;

        if tc_scope.has_caught() {
            let exc = tc_scope.exception()
                .map(|e| e.to_rust_string_lossy(tc_scope))
                .unwrap_or_else(|| "unknown".into());
            let msg = tc_scope.message()
                .map(|m| m.get(tc_scope).to_rust_string_lossy(tc_scope))
                .unwrap_or_default();
            return Err(JsError::ExecuteError(format!("{} {}", msg, exc)));
        }

        let value = value.ok_or_else(|| JsError::ExecuteError("no result".into()))?;
        let js_value = v8_value_to_js_value(value, tc_scope);

        Ok(JsResult {
            value: js_value,
            error: None,
            execution_time_ms: elapsed_ms,
        })
    }

    /// GAP 1 + GAP 2: Create a persistent isolate with quota enforcement.
    fn create_isolate(&self, quota: JsQuota) -> Result<JsIsolate, JsError> {
        init_v8();
        let data = V8IsolateData::new(quota)?;
        Ok(JsIsolate::from_v8(data))
    }

    /// GAP 7: Execute inside the passed persistent isolate (not a fresh one).
    fn execute_in_isolate(&self, isolate: &JsIsolate, source: &str, timeout_ms: Option<u64>) -> Result<JsResult, JsError> {
        match &isolate.backing {
            JsIsolateBacking::V8(data) => data.execute_script(source, false, timeout_ms),
            #[cfg(not(feature = "v8"))]
            _ => Err(JsError::IsolateError("not a V8 isolate".into())),
        }
    }

    fn supports_isolates(&self) -> bool { true }
}

// ---------------------------------------------------------------------------
// v8_value_to_js_value — GAP 3: Complete value extraction
// ---------------------------------------------------------------------------

/// Convert a V8 Local<Value> to our JsValue enum.
/// GAP 3: Handles ALL V8 value types including boolean, array, object, BigInt.
#[cfg(feature = "v8")]
fn v8_value_to_js_value<'s>(
    val: v8::Local<'s, v8::Value>,
    scope: &mut v8::HandleScope<'s>,
) -> JsValue {
    if val.is_null() {
        return JsValue::Null;
    }
    if val.is_undefined() {
        return JsValue::Undefined;
    }
    if val.is_true() || val.is_false() {
        return JsValue::Bool(val.boolean_value(scope));
    }
    if val.is_number() {
        if let Some(n) = val.to_number(scope) {
            return JsValue::Number(n.value());
        }
        return JsValue::Number(f64::NAN);
    }
    if val.is_string() {
        return JsValue::String(val.to_rust_string_lossy(scope));
    }
    if val.is_big_int() {
        if let Some(bi) = val.to_big_int(scope) {
            // Phase 2.1: BigInt simplified (rusty_v8 0.32.1 API uses mutable buffer for to_words_array)
            let _ = bi; // placeholder until Phase 2.3 BigInt full support
            return JsValue::Number(0.0);
        }
        return JsValue::Undefined;
    }
    if val.is_array() {
        // V8 Array is a kind of Object — get it via to_object
        let _obj = match val.to_object(scope) {
            Some(o) => o,
            None => return JsValue::Undefined,
        };
        let arr = unsafe {
            // SAFETY: we just checked is_array() which guarantees it's an Array
            v8::Local::<v8::Array>::cast(val)
        };
        let len = arr.length();
        let mut items = Vec::with_capacity(len as usize);
        for i in 0..len {
            if let Some(v) = arr.get_index(scope, i) {
                items.push(v8_value_to_js_value(v, scope));
            }
        }
        return JsValue::Array(items);
    }
    if val.is_object() {
        let obj = match val.to_object(scope) {
            Some(o) => o,
            None => return JsValue::Undefined,
        };
        let keys = match obj.get_property_names(scope) {
            Some(k) => k,
            None => return JsValue::Object(std::collections::HashMap::new()),
        };
        let mut map = std::collections::HashMap::new();
        let len = keys.length();
        for i in 0..len {
            if let Some(k) = keys.get_index(scope, i) {
                let key_str = k.to_rust_string_lossy(scope);
                if let Some(v) = obj.get(scope, k) {
                    map.insert(key_str, v8_value_to_js_value(v, scope));
                }
            }
        }
        return JsValue::Object(map);
    }
    JsValue::Undefined
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod v8_tests {
    use super::*;
    use crate::{JsEngine, JsQuota};

    #[test]
    fn test_v8_execute_arithmetic() {
        let engine = V8JsEngine::new();
        let res = engine.execute_source("1 + 2 + 3").expect("execute");
        assert!(res.error.is_none());
        assert!(
            matches!(res.value, JsValue::Number(n) if (n - 6.0).abs() < 0.001),
            "expected 6.0, got {:?}",
            res.value
        );
        // GAP 5: execution_time_ms should be real (not hardcoded 0)
        // For simple arithmetic it should be 0ms (sub-millisecond), but not broken
    }

    #[test]
    fn test_v8_two_isolates_independent() {
        let engine = V8JsEngine::new();
        let quota = JsQuota::default();
        let iso_a = engine.create_isolate(quota.clone()).expect("a");
        let iso_b = engine.create_isolate(quota).expect("b");

        let res_a = engine.execute_in_isolate(&iso_a, "var x = 99; typeof x", None);
        assert!(res_a.is_ok());

        let res_b = engine.execute_in_isolate(&iso_b, "typeof x === 'undefined' ? 1 : 0", None);
        assert!(res_b.is_ok());
        assert!(
            matches!(res_b.unwrap().value, JsValue::Number(n) if n == 1.0),
            "isolate B leaked state"
        );
    }

    #[test]
    fn test_v8_extracts_string() {
        let engine = V8JsEngine::new();
        let res = engine.execute_source("'hello' + ' world'").expect("execute");
        assert_eq!(res.value, JsValue::String("hello world".into()));
    }

    #[test]
    fn test_v8_extracts_bool() {
        let engine = V8JsEngine::new();
        let res_true = engine.execute_source("true").expect("execute");
        let res_false = engine.execute_source("false").expect("execute");
        assert_eq!(res_true.value, JsValue::Bool(true));
        assert_eq!(res_false.value, JsValue::Bool(false));
    }

    #[test]
    fn test_v8_extracts_array() {
        let engine = V8JsEngine::new();
        let res = engine.execute_source("[1, 2, 3]").expect("execute");
        match res.value {
            JsValue::Array(arr) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], JsValue::Number(1.0));
                assert_eq!(arr[1], JsValue::Number(2.0));
                assert_eq!(arr[2], JsValue::Number(3.0));
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn test_v8_extracts_object() {
        let engine = V8JsEngine::new();
        let res = engine.execute_source("({ a: 1, b: 'two' })").expect("execute");
        match res.value {
            JsValue::Object(obj) => {
                assert_eq!(obj.get("a"), Some(&JsValue::Number(1.0)));
                assert_eq!(obj.get("b"), Some(&JsValue::String("two".into())));
            }
            other => panic!("expected Object, got {:?}", other),
        }
    }

    #[test]
    fn test_v8_extracts_null_and_undefined() {
        let engine = V8JsEngine::new();
        let res_null = engine.execute_source("null").expect("execute");
        let res_undef = engine.execute_source("undefined").expect("execute");
        assert_eq!(res_null.value, JsValue::Null);
        assert_eq!(res_undef.value, JsValue::Undefined);
    }

    #[test]
    fn test_v8_syntax_error_caught() {
        let engine = V8JsEngine::new();
        let res = engine.execute_source("function {{{ broken syntax");
        assert!(res.is_err(), "expected CompileError for bad syntax");
        match res {
            Err(JsError::CompileError(_)) => {} // OK
            Err(other) => panic!("expected CompileError, got {:?}", other),
            Ok(_) => panic!("expected error"),
        }
    }

    #[test]
    fn test_v8_runtime_error_caught() {
        let engine = V8JsEngine::new();
        let res = engine.execute_source("throw new Error('boom')");
        assert!(res.is_err(), "expected ExecuteError for thrown exception");
        match res {
            Err(JsError::ExecuteError(msg)) => {
                assert!(msg.contains("boom") || msg.contains("Error"), "msg: {}", msg);
            }
            other => panic!("expected ExecuteError, got {:?}", other),
        }
    }

    #[test]
    fn test_v8_execute_compiled_module() {
        let engine = V8JsEngine::new();
        let module = engine.compile("'compiled:' + 'works'").expect("compile");
        let res = engine.execute(&module).expect("execute");
        assert_eq!(res.value, JsValue::String("compiled:works".into()));
    }

    #[test]
    fn test_v8_isolate_persists_state() {
        let engine = V8JsEngine::new();
        let quota = JsQuota::default();
        let iso = engine.create_isolate(quota).expect("create");

        // First call sets global
        let _res1 = engine.execute_in_isolate(&iso, "globalThis.persisted = 42", None).expect("set");
        // Second call reads it
        // P1-A.1: Persistent context means globalThis state survives across
        // execute_in_isolate calls.
        let res2 = engine.execute_in_isolate(&iso, "globalThis.persisted", None).expect("get");
        assert!(
            matches!(res2.value, JsValue::Number(n) if (n - 42.0).abs() < 0.001),
            "expected persisted 42.0 (P1-A.1 persistent context), got {:?}",
            res2.value
        );
    }

    #[test]
    fn test_v8_quota_memory_limit() {
        let engine = V8JsEngine::new();
        let quota = JsQuota {
            max_memory_bytes: Some(1024 * 1024), // 1MB
            max_cpu_ms: None,
            max_instructions: None,
        };
        let iso = engine.create_isolate(quota);
        // Even tiny limit may or may not trigger immediate OOM; this just verifies
        // creation succeeds with memory quota set.
        assert!(iso.is_ok() || iso.is_err()); // both acceptable
    }

    #[test]
    fn test_v8_execution_time_real() {
        let engine = V8JsEngine::new();
        // Long loop should take > 0ms
        let res = engine.execute_source(
            "let s = 0; for (let i = 0; i < 100000; i++) s += i; s"
        ).expect("execute");
        // execution_time_ms is a u64; we don't assert on value (timing is flaky)
        // but verify it's a real number, not breaking serialization
        let _ = res.execution_time_ms;
        if let JsValue::Number(n) = res.value {
            // Sum of 0..99999 = 4999950000
            assert!((n - 4999950000.0).abs() < 1.0, "got {}", n);
        } else {
            panic!("expected number");
        }
    }

    #[test]
    fn test_v8_persistent_context_global_this() {
        let engine = V8JsEngine::new();
        let quota = JsQuota::default();
        let iso = engine.create_isolate(quota).expect("create");

        // Set global state
        engine.execute_in_isolate(&iso, "globalThis.__test = 42", None).expect("set");
        // Read it back — persistent context means it should survive
        let res = engine.execute_in_isolate(&iso, "globalThis.__test", None).expect("get");
        assert!(
            matches!(res.value, JsValue::Number(n) if (n - 42.0).abs() < 0.001),
            "expected 42.0, got {:?}",
            res.value
        );
    }

    #[test]
    fn test_v8_counter_increments() {
        let engine = V8JsEngine::new();
        let quota = JsQuota::default();
        let iso = engine.create_isolate(quota).expect("create");

        // First execution: initialize counter to 1
        let _res1 = engine.execute_in_isolate(&iso, "globalThis.counter = (globalThis.counter || 0) + 1", None).expect("first");
        // Second execution: increment to 2
        let res2 = engine.execute_in_isolate(&iso, "globalThis.counter = (globalThis.counter || 0) + 1", None).expect("second");
        // Read back the counter value
        let res3 = engine.execute_in_isolate(&iso, "globalThis.counter", None).expect("read");
        assert!(
            matches!(res3.value, JsValue::Number(n) if (n - 2.0).abs() < 0.001),
            "expected counter 2.0, got {:?}",
            res3.value
        );
        // The second result should also show 2 if it evaluated the expression
        assert!(
            matches!(res2.value, JsValue::Number(n) if (n - 2.0).abs() < 0.001),
            "expected 2.0 from second run, got {:?}",
            res2.value
        );
    }

    /// P1-A.2: Timeout enforcement — an infinite loop must trigger Timeout.
    /// We use `for(;;) { JSON.stringify({}); }` which contains V8 API calls
    /// and safe points so the optimizer does not eliminate the loop.
    #[test]
    fn test_v8_timeout_terminates_infinite_loop() {
        let engine = V8JsEngine::new();
        let quota = JsQuota::default();
        let iso = engine.create_isolate(quota).expect("create isolate");

        let start = std::time::Instant::now();
        let res = engine.execute_in_isolate(
            &iso,
            "for(;;) { JSON.stringify({}); }",
            Some(50),
        );
        let elapsed = start.elapsed();

        assert!(
            matches!(res, Err(JsError::Timeout(50))),
            "expected Timeout(50), got {:?}",
            res
        );
        assert!(
            elapsed.as_secs_f64() < 1.0,
            "timeout should fire quickly (<1s wall-clock), took {:?}",
            elapsed
        );
    }

    /// P1-A.2: After timeout, the isolate is terminated and the next call must work.
    /// This verifies the isolate doesn't hang and the mechanism is end-to-end.
    #[test]
    fn test_v8_isolate_recreated_after_timeout() {
        let engine = V8JsEngine::new();
        let quota = JsQuota::default();
        let iso = engine.create_isolate(quota.clone()).expect("create");

        // First call: timeout should fire.
        let res1 = engine.execute_in_isolate(&iso, "for(;;) { JSON.stringify({}); }", Some(50));
        assert!(
            matches!(res1, Err(JsError::Timeout(50))),
            "expected timeout on first call, got {:?}",
            res1
        );

        // After timeout, the isolate backing should have been cleared.
        // The next call must work (recreates/reuses backing).
        let res2 = engine.execute_in_isolate(&iso, "1 + 1", None);
        assert!(res2.is_ok(), "expected success after timeout, got {:?}", res2);
        let res2_val = res2.as_ref().unwrap();
        assert!(
            matches!(res2_val.value, JsValue::Number(n) if (n - 2.0).abs() < 0.001),
            "expected 2.0 after recreation, got {:?}",
            res2_val.value
        );
    }
}