//! Integration tests for V8 interop forms (feature-gated `v8`).
//!
//! Covers the three V8 interop tests from /tmp/swarm-arch-interop.md §6:
//! 1. `test_structured_json_roundtrip` — JS object → `JSON.stringify` roundtrip
//! 2. `test_host_function_from_js`    — Rust function registered as JS global, called from JS
//! 3. `test_v8_promise_async`         — `new Promise(resolve => resolve(42))` → Promise object
//!
//! Uses `rusty_v8` 0.32.1 directly. No `boa` references.

#[cfg(feature = "v8")]
use rusty_v8 as v8;

/// Initialise V8 once (same logic as v8_impl).
#[cfg(feature = "v8")]
fn init_v8_for_tests() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
}

// ---------------------------------------------------------------------------
// Form 1: Structured JSON roundtrip
// ---------------------------------------------------------------------------

#[cfg(feature = "v8")]
#[test]
fn test_structured_json_roundtrip() {
    init_v8_for_tests();
    // Source: build a JS object, return JSON.stringify(obj)
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = &mut v8::HandleScope::new(&mut isolate);
    let context = v8::Context::new(scope);
    {
        let scope = &mut v8::ContextScope::new(scope, context);
        let source = r#"
            (function() {
                const obj = { a: 1, b: true, c: null, d: [1, 2, 3] };
                return JSON.stringify(obj);
            })()
        "#;
        let code = v8::String::new(scope, source).expect("string alloc");
        let script = v8::Script::compile(scope, code, None).expect("compile");
        let value = script.run(scope).expect("run");
        let result_str = value.to_string(scope)
            .expect("string conversion")
            .to_rust_string_lossy(scope);
        assert!(
            result_str.contains(r#""a":1"#),
            "expected `\"a\":1` in JSON, got: {result_str}"
        );
        assert!(
            result_str.contains(r#""b":true"#),
            "expected `\"b\":true` in JSON, got: {result_str}"
        );
        assert!(
            result_str.contains(r#""c":null"#),
            "expected `\"c\":null` in JSON, got: {result_str}"
        );
        assert!(
            result_str.contains("[1,2,3]") || result_str.contains("[1, 2, 3]"),
            "expected nested array, got: {result_str}"
        );
    }
}

// ---------------------------------------------------------------------------
// Form 2: Host function from JS
// ---------------------------------------------------------------------------

#[cfg(feature = "v8")]
fn host_add_cb(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let a = args.get(0).to_number(scope)
        .map(|n| n.value())
        .unwrap_or(0.0);
    let b = args.get(1).to_number(scope)
        .map(|n| n.value())
        .unwrap_or(0.0);
    rv.set(v8::Number::new(scope, a + b).into());
}

#[cfg(feature = "v8")]
#[test]
fn test_host_function_from_js() {
    init_v8_for_tests();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = &mut v8::HandleScope::new(&mut isolate);
    let context = v8::Context::new(scope);
    {
        let scope = &mut v8::ContextScope::new(scope, context);
        // Build a FunctionTemplate from the host callback, then install on global
        let template = v8::FunctionTemplate::new(scope, host_add_cb);
        let function = template.get_function(scope).expect("get_function");
        let global = context.global(scope);
        let key = v8::String::new(scope, "add").expect("key");
        global.set(scope, key.into(), function.into());

        // Call add(2, 3) from JS and verify the result
        let source = "add(2, 3)";
        let code = v8::String::new(scope, source).expect("code");
        let script = v8::Script::compile(scope, code, None).expect("compile");
        let value = script.run(scope).expect("run");
        let num = value.to_number(scope).expect("number result");
        let result = num.value();
        assert!(
            (result - 5.0).abs() < f64::EPSILON,
            "host function add(2,3) should return 5.0, got {result}"
        );
    }
}

// ---------------------------------------------------------------------------
// Form 4: V8 promise async
// ---------------------------------------------------------------------------

#[cfg(feature = "v8")]
#[test]
fn test_v8_promise_async() {
    init_v8_for_tests();
    // Construct a Promise that resolves with 42, verify the returned object
    // is recognised as a Promise.  Full resolved-value extraction would
    // require a microtask queue drain + PromiseResolver; for Phase 1 we
    // confirm Promise construction and identity.
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = &mut v8::HandleScope::new(&mut isolate);
    let context = v8::Context::new(scope);
    {
        let scope = &mut v8::ContextScope::new(scope, context);
        let source = r#"
            new Promise(function(resolve, reject) { resolve(42); })
        "#;
        let code = v8::String::new(scope, source).expect("code");
        let script = v8::Script::compile(scope, code, None).expect("compile");
        let value = script.run(scope).expect("run");
        // The result must be a JS object (Promise instances are objects)
        assert!(
            value.is_object(),
            "Promise expression must return an object, got a non-object"
        );
        // Promise objects are native — verify the toString tag is "object Promise"
        let to_str = value.to_string(scope)
            .expect("to_string")
            .to_rust_string_lossy(scope);
        assert!(
            to_str.contains("Promise"),
            "expected 'Promise' in toString, got: {to_str}"
        );
    }
}
