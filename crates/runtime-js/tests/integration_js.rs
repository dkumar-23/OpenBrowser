//! Integration test for Phase 2 §2.1 — V8JsEngine (feature-gated `v8`).
//!
//! Replaces the earlier Boa test. Uses `rusty_v8`. No `boa` references.

use runtime_js::JsEngine;

#[cfg(feature = "v8")]
#[test]
fn test_v8_js_engine_evaluates_arithmetic() {
    use runtime_js::V8JsEngine;

    let engine = V8JsEngine::new();
    let result = engine.execute_source("1 + 2 + 3");
    assert!(
        result.is_ok(),
        "V8JsEngine should execute '1+2+3' successfully: {:?}",
        result
    );
    let res = result.unwrap();
    assert!(
        res.error.is_none(),
        "execution should not throw: error={:?}",
        res.error
    );
    assert!(
        matches!(res.value, runtime_js::JsValue::Number(n) if (n - 6.0).abs() < f64::EPSILON),
        "expected JsValue::Number(6.0), got {:?}",
        res.value
    );
}

#[cfg(not(feature = "v8"))]
#[test]
fn test_v8_js_engine_requires_feature() {
    // This test runs when `v8` is NOT enabled. It instructs the user to
    // run with the `v8` feature so the real integration test above executes.
    eprintln!("SKIP: V8JsEngine test requires `v8` feature.");
    eprintln!("Run with: cargo test --workspace --features v8");
    // We don't panic — this is a skip marker, not a failure.
    // The real test lives behind `#[cfg(feature = "v8")]` above.
}
