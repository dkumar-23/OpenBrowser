//! runtime-js: JavaScript engine abstraction for OpenBrowser runtime.
//!
//! Architecture basis: context.md §4 (JsEngine trait; V8 initial impl) and §5
//! (independent isolates per worker). This crate defines the `JsEngine` trait.
//! Concrete engines (V8, Rhai) implement it behind the abstraction.
//!
//! Design rules:
//! - trait first: no concrete engine without trait boundary
//! - isolate-first: each worker gets its own JS isolate
//! - no engine-type leakage: callers depend on the trait + public types only
//! - V8 engine gated behind `v8` feature

use serde::{Serialize, Deserialize};
use std::sync::Arc;
use thiserror::Error;

#[cfg(feature = "v8")]
mod v8_impl;
#[cfg(feature = "v8")]
pub use v8_impl::V8JsEngine;

// ---------------------------------------------------------------------------
// JsIsolate — opaque handle to a JavaScript isolate
// ---------------------------------------------------------------------------

/// Per-engine isolate data stored inside a `JsIsolate`.
#[cfg(feature = "v8")]
#[derive(Clone)]
pub(crate) enum JsIsolateBacking {
    V8(v8_impl::V8IsolateData),
}

#[cfg(not(feature = "v8"))]
#[derive(Clone)]
pub(crate) enum JsIsolateBacking {
    None,
}

/// Opaque handle to a JavaScript isolate (sandboxed execution context).
///
/// `JsIsolate` is cloneable so it can be shared across a worker's tasks.
/// The backing data is opaque: callers interact exclusively via `JsEngine`.
#[derive(Clone)]
pub struct JsIsolate {
    backing: JsIsolateBacking,
}

impl std::fmt::Debug for JsIsolate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsIsolate").finish_non_exhaustive()
    }
}

impl JsIsolate {
    /// Construct a V8-backed isolate (called by `V8JsEngine::create_isolate`).
    #[cfg(feature = "v8")]
    pub(crate) fn from_v8(data: v8_impl::V8IsolateData) -> Self {
        Self { backing: JsIsolateBacking::V8(data) }
    }

    /// No-arg constructor used by the Noop stub.
    pub(crate) fn new() -> Self {
        #[cfg(feature = "v8")]
        {
            Self { backing: JsIsolateBacking::V8(v8_impl::V8IsolateData::placeholder()) }
        }
        #[cfg(not(feature = "v8"))]
        {
            Self { backing: JsIsolateBacking::None }
        }
    }
}

// ---------------------------------------------------------------------------
// JsValue
// ---------------------------------------------------------------------------

/// Value that can be passed to/from JavaScript.
/// Supports all V8 value types including boolean, array, object, and BigInt.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum JsValue {
    Null,
    Undefined,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsValue>),
    Object(std::collections::HashMap<String, JsValue>),
}

// ---------------------------------------------------------------------------
// JsError
// ---------------------------------------------------------------------------

/// Errors from JavaScript engine operations.
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum JsError {
    #[error("compilation failed: {0}")]
    CompileError(String),

    #[error("execution failed: {0}")]
    ExecuteError(String),

    #[error("isolate error: {0}")]
    IsolateError(String),

    #[error("timeout: execution exceeded {0}ms")]
    Timeout(u64),

    #[error("resource exceeded: {0}")]
    ResourceExceeded(String),

    #[error("engine not initialized: {0}")]
    NotInitialized(String),

    #[error("unsupported feature: {0}")]
    Unsupported(String),
}

// ---------------------------------------------------------------------------
// JsTier2Host — host-feature abstraction (WebSocket / Worker / IndexedDB / WASM)
// ---------------------------------------------------------------------------

/// Opaque handle to an open WebSocket connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsWebSocket {
    pub id: String,
    pub url: String,
}

/// Opaque handle to a spawned Worker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsWorker {
    pub id: String,
}

/// Opaque handle to an open IndexedDB database.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsDbHandle {
    pub id: String,
    pub name: String,
}

/// Opaque handle to a compiled WASM module.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsWasmModule {
    pub id: String,
    pub byte_len: usize,
}

/// Opaque handle to an instantiated WASM module.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsWasmInstance {
    pub module_id: String,
}

/// Host-feature boundary for Tier 2 JavaScript features.
///
/// This trait keeps the engine free of hard coupling to any host runtime:
/// concrete hosts (real browser, in-memory fake) implement it in Rust and are
/// injected. Every method has a default implementation returning
/// `JsError::Unsupported`, so an engine with no host configured fails
/// gracefully instead of panicking.
pub trait JsTier2Host: Send + Sync + std::fmt::Debug {
    /// Open a WebSocket connection.
    fn websocket_open(&self, url: &str) -> Result<JsWebSocket, JsError> {
        Err(JsError::Unsupported(format!("websocket_open: {}", url)))
    }

    /// Send a message on an open WebSocket; returns the (echoed) payload.
    fn websocket_send(&self, socket: &JsWebSocket, message: &str) -> Result<JsValue, JsError> {
        Err(JsError::Unsupported(format!(
            "websocket_send: {} ({})",
            socket.id, message
        )))
    }

    /// Spawn a Worker running `script`.
    fn worker_spawn(&self, script: &str) -> Result<JsWorker, JsError> {
        Err(JsError::Unsupported(format!("worker_spawn: {} bytes", script.len())))
    }

    /// Post a message to a Worker; returns the worker's reply.
    fn worker_post_message(&self, worker: &JsWorker, message: &str) -> Result<JsValue, JsError> {
        Err(JsError::Unsupported(format!(
            "worker_post_message: {} ({})",
            worker.id, message
        )))
    }

    /// Open an IndexedDB database.
    fn indexeddb_open(&self, name: &str) -> Result<JsDbHandle, JsError> {
        Err(JsError::Unsupported(format!("indexeddb_open: {}", name)))
    }

    /// Store a key/value pair in an IndexedDB database.
    fn indexeddb_put(&self, db: &JsDbHandle, key: &str, value: &str) -> Result<(), JsError> {
        let _ = (key, value);
        Err(JsError::Unsupported(format!("indexeddb_put: {}", db.id)))
    }

    /// Read a key from an IndexedDB database.
    fn indexeddb_get(&self, db: &JsDbHandle, key: &str) -> Result<Option<String>, JsError> {
        Err(JsError::Unsupported(format!(
            "indexeddb_get: {} ({})",
            db.id, key
        )))
    }

    /// Compile a WASM module from raw bytes.
    fn wasm_compile(&self, bytes: &[u8]) -> Result<JsWasmModule, JsError> {
        Err(JsError::Unsupported(format!("wasm_compile: {} bytes", bytes.len())))
    }

    /// Instantiate a compiled WASM module.
    fn wasm_instantiate(&self, module: &JsWasmModule) -> Result<JsWasmInstance, JsError> {
        Err(JsError::Unsupported(format!("wasm_instantiate: {}", module.id)))
    }
}

/// No-op host: every Tier 2 operation returns `JsError::Unsupported`.
#[derive(Debug, Default, Clone)]
pub struct NoopJsTier2Host;

impl JsTier2Host for NoopJsTier2Host {}

/// Minimal in-memory Tier 2 host suitable for tests.
///
/// Provides an echo WebSocket, a trivial Worker, an in-memory key/value store,
/// and a mock WASM module. Implemented entirely in Rust behind the trait; it is
/// never wired into V8 internals.
#[derive(Debug, Default)]
pub struct InMemoryTier2Host {
    inner: std::sync::Mutex<InMemoryTier2State>,
}

#[derive(Debug, Default)]
struct InMemoryTier2State {
    next_id: u64,
    stores: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
}

impl InMemoryTier2Host {
    pub fn new() -> Self {
        Self::default()
    }

    fn next_id(&self) -> String {
        let mut state = self.inner.lock().unwrap();
        state.next_id += 1;
        format!("h{}", state.next_id)
    }
}

impl JsTier2Host for InMemoryTier2Host {
    fn websocket_open(&self, url: &str) -> Result<JsWebSocket, JsError> {
        Ok(JsWebSocket { id: self.next_id(), url: url.to_string() })
    }

    fn websocket_send(&self, _socket: &JsWebSocket, message: &str) -> Result<JsValue, JsError> {
        Ok(JsValue::String(message.to_string()))
    }

    fn worker_spawn(&self, _script: &str) -> Result<JsWorker, JsError> {
        Ok(JsWorker { id: self.next_id() })
    }

    fn worker_post_message(&self, _worker: &JsWorker, message: &str) -> Result<JsValue, JsError> {
        Ok(JsValue::String(format!("worker:{}", message)))
    }

    fn indexeddb_open(&self, name: &str) -> Result<JsDbHandle, JsError> {
        let id = self.next_id();
        self.inner
            .lock()
            .unwrap()
            .stores
            .entry(id.clone())
            .or_default();
        Ok(JsDbHandle { id, name: name.to_string() })
    }

    fn indexeddb_put(&self, db: &JsDbHandle, key: &str, value: &str) -> Result<(), JsError> {
        let mut state = self.inner.lock().unwrap();
        let store = state.stores.entry(db.id.clone()).or_default();
        store.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn indexeddb_get(&self, db: &JsDbHandle, key: &str) -> Result<Option<String>, JsError> {
        let state = self.inner.lock().unwrap();
        Ok(state
            .stores
            .get(&db.id)
            .and_then(|store| store.get(key).cloned()))
    }

    fn wasm_compile(&self, bytes: &[u8]) -> Result<JsWasmModule, JsError> {
        Ok(JsWasmModule { id: self.next_id(), byte_len: bytes.len() })
    }

    fn wasm_instantiate(&self, module: &JsWasmModule) -> Result<JsWasmInstance, JsError> {
        Ok(JsWasmInstance { module_id: module.id.clone() })
    }
}

// ---------------------------------------------------------------------------
// CompiledModule
// ---------------------------------------------------------------------------

/// Opaque handle to a compiled JavaScript module / function.
/// Stores the source string; re-compilation happens on execute for Phase 2.1.
/// Future: store compiled bytecode when V8 script serialization is available.
#[derive(Clone, Debug)]
pub struct CompiledModule {
    source: String,
}

impl CompiledModule {
    pub fn from_source(source: String) -> Self {
        Self { source }
    }
    pub fn source(&self) -> &str {
        &self.source
    }
}

// ---------------------------------------------------------------------------
// JsQuota
// ---------------------------------------------------------------------------

/// Resource usage limits for an isolate.
#[derive(Clone, Debug, Default)]
pub struct JsQuota {
    /// Max CPU time in milliseconds.
    pub max_cpu_ms: Option<u64>,
    /// Max memory in bytes.
    pub max_memory_bytes: Option<u64>,
    /// Max instructions (if supported by engine).
    pub max_instructions: Option<u64>,
}

// ---------------------------------------------------------------------------
// JsResult
// ---------------------------------------------------------------------------

/// Result of a JavaScript execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsResult {
    /// The return value of the script.
    pub value: JsValue,
    /// Whether any error was thrown during execution.
    pub error: Option<String>,
    /// Execution time in milliseconds.
    pub execution_time_ms: u64,
}

// ---------------------------------------------------------------------------
// JsEngine
// ---------------------------------------------------------------------------

/// Trait for JavaScript engine implementations.
///
/// Contract: every engine MUST implement compile/execute/create_isolate/execute_in_isolate.
/// Isolates are fully sandboxed (no shared state). Policy is handled by the caller
/// (runtime-adapters-http), not by the engine.
pub trait JsEngine: Send + Sync + std::fmt::Debug {
    /// Engine name (e.g., "v8", "rhai").
    fn name(&self) -> &str;

    /// Compile JavaScript source code into a module.
    fn compile(&self, source: &str) -> Result<CompiledModule, JsError>;

    /// Execute a compiled module and return the result.
    fn execute(&self, module: &CompiledModule) -> Result<JsResult, JsError>;

    /// Execute raw source without pre-compilation (convenience for simple scripts).
    fn execute_source(&self, source: &str) -> Result<JsResult, JsError> {
        let module = self.compile(source)?;
        self.execute(&module)
    }

    /// Create a new isolated execution context.
    /// Isolates are fully sandboxed: no shared state between isolates.
    fn create_isolate(&self, quota: JsQuota) -> Result<JsIsolate, JsError>;

    /// Execute source in a specific isolate (if supported by engine).
    fn execute_in_isolate(&self, isolate: &JsIsolate, source: &str, timeout_ms: Option<u64>) -> Result<JsResult, JsError>;

    /// Check if the engine supports isolates.
    fn supports_isolates(&self) -> bool {
        false
    }

    /// Access the Tier 2 host features (WebSocket / Worker / IndexedDB / WASM).
    ///
    /// Default: a no-op host whose operations all return
    /// `JsError::Unsupported`. Engines that have a real host override this.
    fn tier2_host(&self) -> Arc<dyn JsTier2Host> {
        Arc::new(NoopJsTier2Host)
    }

    /// Execute a compiled module inside a persistent isolate.
    ///
    /// Contract: engines that support isolates MUST reuse the isolate's
    /// execution context so globals set by one call survive into subsequent
    /// calls on the same isolate, even across re-compilation of the module.
    /// The default implementation delegates to `execute_in_isolate`, which
    /// preserves this guarantee for every engine that implements it.
    fn execute_module_in_isolate(
        &self,
        isolate: &JsIsolate,
        module: &CompiledModule,
        timeout_ms: Option<u64>,
    ) -> Result<JsResult, JsError> {
        self.execute_in_isolate(isolate, module.source(), timeout_ms)
    }
}

// ---------------------------------------------------------------------------
// NoopJsEngine — Phase 1 stub
// ---------------------------------------------------------------------------

/// Stub engine: compile/execute return errors until a real engine is enabled.
#[derive(Debug, Default)]
pub struct NoopJsEngine;

impl JsEngine for NoopJsEngine {
    fn name(&self) -> &str { "noop" }

    fn compile(&self, _source: &str) -> Result<CompiledModule, JsError> {
        Err(JsError::NotInitialized(
            "No JavaScript engine initialized. Enable the `v8` feature.".into(),
        ))
    }

    fn execute(&self, _module: &CompiledModule) -> Result<JsResult, JsError> {
        Err(JsError::NotInitialized(
            "No JavaScript engine initialized. Enable the `v8` feature.".into(),
        ))
    }

    fn create_isolate(&self, _quota: JsQuota) -> Result<JsIsolate, JsError> {
        Ok(JsIsolate::new())
    }

    fn execute_in_isolate(&self, _isolate: &JsIsolate, _source: &str, _timeout_ms: Option<u64>) -> Result<JsResult, JsError> {
        Err(JsError::NotInitialized(
            "No JavaScript engine initialized. Enable the `v8` feature.".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noop_engine_rejects_execution() {
        let engine = NoopJsEngine;
        assert_eq!(engine.name(), "noop");
        assert!(!engine.supports_isolates());

        let result = engine.execute_source("console.log('hello')");
        assert!(result.is_err());

        let compile_err = engine.compile("1 + 1");
        assert!(matches!(compile_err, Err(JsError::NotInitialized(_))));
    }

    #[test]
    fn test_js_result_serialization() {
        let result = JsResult {
            value: JsValue::Number(42.0),
            error: None,
            execution_time_ms: 5,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("42"));
    }

    #[test]
    fn test_js_value_serialization() {
        let mut obj = std::collections::HashMap::new();
        obj.insert("foo".into(), JsValue::String("bar".into()));
        let v = JsValue::Object(obj);
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("\"foo\""));
    }

    #[test]
    fn test_js_value_bool_serialization() {
        let v = JsValue::Bool(true);
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("true"));
    }

    #[test]
    fn test_js_value_array_serialization() {
        let v = JsValue::Array(vec![JsValue::Number(1.0), JsValue::Number(2.0)]);
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("1"));
    }

    #[test]
    fn test_compiled_module_stores_source() {
        let cm = CompiledModule::from_source("1 + 1".into());
        assert_eq!(cm.source(), "1 + 1");
    }

    #[test]
    fn test_noop_tier2_host_returns_unsupported() {
        let host = NoopJsTier2Host;
        assert!(matches!(host.websocket_open("ws://x"), Err(JsError::Unsupported(_))));
        assert!(matches!(host.worker_spawn("onmessage=()=>{}"), Err(JsError::Unsupported(_))));
        assert!(matches!(host.indexeddb_open("db"), Err(JsError::Unsupported(_))));
        assert!(matches!(host.wasm_compile(&[0u8, 1, 2]), Err(JsError::Unsupported(_))));
    }

    #[test]
    fn test_noop_engine_tier2_returns_unsupported_without_panic() {
        let engine = NoopJsEngine;
        let host = engine.tier2_host();
        assert!(matches!(host.websocket_open("ws://x"), Err(JsError::Unsupported(_))));
        assert!(matches!(host.wasm_compile(&[0u8]), Err(JsError::Unsupported(_))));
    }

    #[test]
    fn test_inmemory_tier2_host_roundtrips_all_features() {
        let host = InMemoryTier2Host::new();

        let ws = host.websocket_open("ws://echo").expect("open");
        assert_eq!(ws.url, "ws://echo");
        assert_eq!(
            host.websocket_send(&ws, "ping").expect("send"),
            JsValue::String("ping".into())
        );

        let worker = host.worker_spawn("self.onmessage = (e) => e.data").expect("spawn");
        assert_eq!(
            host.worker_post_message(&worker, "job").expect("post"),
            JsValue::String("worker:job".into())
        );

        let db = host.indexeddb_open("store").expect("open db");
        host.indexeddb_put(&db, "k", "v").expect("put");
        assert_eq!(host.indexeddb_get(&db, "k").expect("get"), Some("v".into()));
        assert_eq!(host.indexeddb_get(&db, "missing").expect("get"), None);

        let module = host.wasm_compile(&[0, 97, 115, 109]).expect("compile");
        assert_eq!(module.byte_len, 4);
        let instance = host.wasm_instantiate(&module).expect("instantiate");
        assert_eq!(instance.module_id, module.id);
    }

    /// Trait-level guarantee: `execute_module_in_isolate` reuses the isolate's
    /// context so globals survive across re-compilation. This mock engine makes
    /// the guarantee assertable under default features (no V8 needed).
    #[test]
    fn test_execute_module_in_isolate_reuses_context() {
        #[derive(Debug, Default)]
        struct MemoryEngine {
            globals: std::sync::Mutex<std::collections::HashMap<String, JsValue>>,
        }

        impl JsEngine for MemoryEngine {
            fn name(&self) -> &str { "memory" }

            fn compile(&self, source: &str) -> Result<CompiledModule, JsError> {
                Ok(CompiledModule::from_source(source.to_string()))
            }

            fn execute(&self, _module: &CompiledModule) -> Result<JsResult, JsError> {
                Err(JsError::Unsupported("use execute_in_isolate".into()))
            }

            fn create_isolate(&self, _quota: JsQuota) -> Result<JsIsolate, JsError> {
                Ok(JsIsolate::new())
            }

            fn execute_in_isolate(
                &self,
                _isolate: &JsIsolate,
                source: &str,
                _timeout_ms: Option<u64>,
            ) -> Result<JsResult, JsError> {
                let mut globals = self.globals.lock().unwrap();
                if let Some(rest) = source.strip_prefix("set:") {
                    let mut parts = rest.splitn(2, '=');
                    let key = parts.next().unwrap_or_default().to_string();
                    let val = parts.next().unwrap_or_default().to_string();
                    globals.insert(key, JsValue::String(val.clone()));
                    Ok(JsResult { value: JsValue::String(val), error: None, execution_time_ms: 0 })
                } else if let Some(key) = source.strip_prefix("get:") {
                    let value = globals.get(key).cloned().unwrap_or(JsValue::Undefined);
                    Ok(JsResult { value, error: None, execution_time_ms: 0 })
                } else {
                    Err(JsError::ExecuteError("unknown source".into()))
                }
            }
        }

        let engine = MemoryEngine::default();
        let iso = engine.create_isolate(JsQuota::default()).expect("isolate");

        let set_module = engine.compile("set:token=abc123").expect("compile set");
        let first = engine
            .execute_module_in_isolate(&iso, &set_module, None)
            .expect("first execute");
        assert_eq!(first.value, JsValue::String("abc123".into()));

        // Re-compiled module reading the global set by the previous call.
        let get_module = engine.compile("get:token").expect("compile get");
        let second = engine
            .execute_module_in_isolate(&iso, &get_module, None)
            .expect("second execute");
        assert_eq!(
            second.value,
            JsValue::String("abc123".into()),
            "global must persist across re-compilation in the same isolate"
        );
    }
}
pub mod adapter;
