//! G30 — Tier 2 JS host features + persistent context reuse (Phase 6.1)
//!
//! Observable:
//! - the default (Noop) host returns `JsError::Unsupported` for every Tier 2
//!   feature without panicking;
//! - the in-memory fake host round-trips WebSocket/Worker/IndexedDB/WASM;
//! - `execute_module_in_isolate` preserves globals across re-compilation,
//!   exposed through the `JsEngine` trait so it is assertable under default
//!   features.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use runtime_js::{
    CompiledModule, InMemoryTier2Host, JsEngine, JsError, JsIsolate, JsQuota, JsResult, JsTier2Host,
    JsValue, NoopJsEngine, NoopJsTier2Host,
};

#[test]
fn noop_engine_and_host_return_unsupported_without_panic() {
    let host = NoopJsTier2Host;
    assert!(matches!(host.websocket_open("ws://x"), Err(JsError::Unsupported(_))));
    assert!(matches!(host.worker_spawn("onmessage=()=>{}"), Err(JsError::Unsupported(_))));
    assert!(matches!(host.indexeddb_open("db"), Err(JsError::Unsupported(_))));
    assert!(matches!(host.wasm_compile(&[0, 1, 2]), Err(JsError::Unsupported(_))));

    let engine = NoopJsEngine;
    assert!(matches!(
        engine.tier2_host().websocket_open("ws://x"),
        Err(JsError::Unsupported(_))
    ));
    assert!(matches!(
        engine.tier2_host().wasm_compile(&[0u8]),
        Err(JsError::Unsupported(_))
    ));
}

#[test]
fn fake_host_roundtrips_websocket_worker_indexeddb_wasm() {
    let host = InMemoryTier2Host::new();

    let ws = host.websocket_open("ws://echo").expect("websocket open");
    assert_eq!(
        host.websocket_send(&ws, "ping").expect("websocket send"),
        JsValue::String("ping".into()),
        "fake websocket must echo the payload"
    );

    let worker = host.worker_spawn("self.onmessage = (e) => e.data").expect("worker spawn");
    assert_eq!(
        host.worker_post_message(&worker, "job").expect("worker post"),
        JsValue::String("worker:job".into())
    );

    let db = host.indexeddb_open("store").expect("indexeddb open");
    host.indexeddb_put(&db, "token", "abc").expect("put");
    assert_eq!(
        host.indexeddb_get(&db, "token").expect("get"),
        Some("abc".to_string())
    );

    let module = host.wasm_compile(&[0, 97, 115, 109]).expect("wasm compile");
    assert_eq!(module.byte_len, 4);
    assert_eq!(
        host.wasm_instantiate(&module).expect("wasm instantiate").module_id,
        module.id
    );
}

#[derive(Debug, Default)]
struct MemoryEngine {
    globals: Mutex<HashMap<String, JsValue>>,
}

impl JsEngine for MemoryEngine {
    fn name(&self) -> &str {
        "memory"
    }

    fn compile(&self, source: &str) -> Result<CompiledModule, JsError> {
        Ok(CompiledModule::from_source(source.to_string()))
    }

    fn execute(&self, _module: &CompiledModule) -> Result<JsResult, JsError> {
        Err(JsError::Unsupported("use execute_in_isolate".into()))
    }

    fn create_isolate(&self, quota: JsQuota) -> Result<JsIsolate, JsError> {
        NoopJsEngine.create_isolate(quota)
    }

    fn execute_in_isolate(
        &self,
        _isolate: &JsIsolate,
        source: &str,
        _timeout_ms: Option<u64>,
    ) -> Result<JsResult, JsError> {
        let mut globals = self.globals.lock().unwrap();
        if let Some(rest) = source.strip_prefix("set:") {
            let (key, value) = rest.split_once('=').unwrap_or((rest, ""));
            globals.insert(key.to_string(), JsValue::String(value.to_string()));
            Ok(JsResult { value: JsValue::String(value.to_string()), error: None, execution_time_ms: 0 })
        } else if let Some(key) = source.strip_prefix("get:") {
            let value = globals.get(key).cloned().unwrap_or(JsValue::Undefined);
            Ok(JsResult { value, error: None, execution_time_ms: 0 })
        } else {
            Err(JsError::ExecuteError("unknown source".into()))
        }
    }
}

#[test]
fn trait_level_context_reuse_survives_recompilation() {
    let engine = MemoryEngine::default();
    let isolate = engine.create_isolate(JsQuota::default()).expect("isolate");

    let set_module = engine.compile("set:session=abc123").expect("compile set");
    engine
        .execute_module_in_isolate(&isolate, &set_module, None)
        .expect("first execute");

    // A freshly compiled module must observe the global from the prior call.
    let get_module = engine.compile("get:session").expect("compile get");
    let res = engine
        .execute_module_in_isolate(&isolate, &get_module, None)
        .expect("second execute");
    assert_eq!(
        res.value,
        JsValue::String("abc123".into()),
        "global must persist across re-compilation in the same isolate"
    );
}

#[tokio::test]
async fn unsupported_tier2_call_is_graceful_and_bounded() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let host = NoopJsTier2Host;
        assert!(matches!(host.indexeddb_get(&runtime_js::JsDbHandle { id: "x".into(), name: "d".into() }, "k"), Err(JsError::Unsupported(_))));
        let engine = NoopJsEngine;
        assert!(matches!(engine.tier2_host().worker_spawn(""), Err(JsError::Unsupported(_))));
    })
    .await
    .expect("unsupported tier2 call must complete without hanging");
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn v8_execute_persists_globals_across_recompilation() {
    tokio::time::timeout(Duration::from_secs(10), async {
        use runtime_js::V8JsEngine;
        let engine = V8JsEngine::new();
        engine
            .execute(&engine.compile("globalThis.__phase6 = 42").expect("compile set"))
            .expect("execute set");
        let res = engine
            .execute(&engine.compile("globalThis.__phase6").expect("compile get"))
            .expect("execute get");
        assert!(
            matches!(res.value, JsValue::Number(n) if (n - 42.0).abs() < 0.001),
            "expected persisted 42.0, got {:?}",
            res.value
        );
    })
    .await
    .expect("v8 persistence test must not hang");
}