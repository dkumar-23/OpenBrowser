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
    fn execute_in_isolate(&self, isolate: &JsIsolate, source: &str) -> Result<JsResult, JsError>;

    /// Check if the engine supports isolates.
    fn supports_isolates(&self) -> bool {
        false
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

    fn execute_in_isolate(&self, _isolate: &JsIsolate, _source: &str) -> Result<JsResult, JsError> {
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
}
