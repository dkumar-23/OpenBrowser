use std::sync::Mutex;
use std::path::PathBuf;
use uuid::Uuid;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use metrics;

/// Every important operation carries these IDs — fulfills observability requirement.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
pub struct TraceContext {
    pub task_id: Uuid,
    pub agent_id: Uuid,
    pub delegation_id: Option<Uuid>,
    pub request_id: Uuid,
    pub timestamp: DateTime<Utc>,
}

impl TraceContext {
    pub fn new(agent_id: Uuid, delegation_id: Option<Uuid>) -> Self {
        Self {
            task_id: Uuid::new_v4(),
            agent_id,
            delegation_id,
            request_id: Uuid::new_v4(),
            timestamp: Utc::now(),
        }
    }
}

/// Replay event — supports deterministic replay/debug.
/// CF-3 FIX: now writes to JSONL file with monotonic sequence.
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct ReplayEvent {
    pub sequence: u64,
    pub event_type: String,
    pub task_id: Uuid,
    pub agent_id: Uuid,
    pub result_summary: String,
    pub timestamp: DateTime<Utc>,
}

/// Phase 1 gap G13: lifecycle observability event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LifecycleEvent {
    pub task_id: Uuid,
    pub agent_id: Uuid,
    pub delegation_id: Option<Uuid>,
    pub event_type: String, // "created" | "queued" | "started" | "policy_decision" | "completed" | "failed" | "cancelled" | "timed_out" | "resource_exceeded"
    pub timestamp: DateTime<Utc>,
    pub details: Option<serde_json::Value>,
}

/// Structured observability trait; all layers must implement.
pub trait Observability: Send + Sync + std::fmt::Debug {
    fn log_structured(&self, level: LogLevel, event: &str, ctx: &TraceContext, kv: &[(&str, &str)]);
    fn trace_span(&self, span: &str, ctx: &TraceContext);
    fn metric(&self, name: &str, value: f64, kv: &[(&str, &str)]);
    fn record_replay(&self, event: ReplayEvent) -> u64;
    /// G13: record a lifecycle event.
    fn record_lifecycle(&self, event: LifecycleEvent);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Trace, Debug, Info, Warn, Error,
}

/// Replay writer: CF-3 FIX — JSONL file with monotonic sequence.
#[derive(Debug)]
pub struct ReplayWriter {
    path: PathBuf,
    sequence: Mutex<u64>,
}

impl ReplayWriter {
    pub fn new(path: PathBuf) -> Self {
        // Ensure parent dir exists
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        Self { path, sequence: Mutex::new(0) }
    }

    fn next_seq(&self) -> u64 {
        let mut guard = self.sequence.lock().unwrap();
        let s = *guard;
        *guard = s + 1;
        s
    }

    pub fn write(&self, event: &ReplayEvent) -> std::io::Result<()> {
        let json = serde_json::to_string(event)?;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{}", json)
    }
}

/// Default in-memory + tracing subscriber implementation.
#[derive(Debug)]
pub struct TraceObservability {
    replay_writer: Option<ReplayWriter>,
}

impl TraceObservability {
    pub fn with_replay(path: PathBuf) -> Self {
        Self { replay_writer: Some(ReplayWriter::new(path)) }
    }
    pub fn without_replay() -> Self {
        Self { replay_writer: None }
    }
}

impl Default for TraceObservability {
    fn default() -> Self { Self::without_replay() }
}

impl Observability for TraceObservability {
    fn log_structured(&self, _level: LogLevel, event: &str, ctx: &TraceContext, _kv: &[(&str, &str)]) {
        tracing::info!(
            event = %event,
            task_id = %ctx.task_id,
            agent_id = %ctx.agent_id,
            delegation_id = ?ctx.delegation_id,
            request_id = %ctx.request_id,
        );
    }
    fn trace_span(&self, span: &str, ctx: &TraceContext) {
        let _ = tracing::info_span!("work", span = %span, task_id = %ctx.task_id, agent_id = %ctx.agent_id);
    }

    // CF-5 FIX: metric() now increments a counter via the metrics crate.
    fn metric(&self, name: &str, value: f64, _kv: &[(&str, &str)]) {
        // CF-5 FIX: emit metric via metrics crate counter
        let owned = name.to_string();
        metrics::counter!(owned).increment(value as u64);
    }

    // CF-3 FIX: record_replay now writes to JSONL file.
    fn record_replay(&self, event: ReplayEvent) -> u64 {
        tracing::debug!(
            replay_type = %event.event_type,
            task_id = %event.task_id,
            agent_id = %event.agent_id,
        );
        if let Some(ref writer) = self.replay_writer {
            // CF-3 FIX: writer is single sequence source — call next_seq() ONCE
            let seq = writer.next_seq();
            let mut e = event;
            e.sequence = seq;
            if let Err(err) = writer.write(&e) {
                tracing::warn!(replay_write_error = %err, "failed to write replay event");
            }
            seq
        } else {
            0
        }
    }

    // G13: emit a structured lifecycle event.
    fn record_lifecycle(&self, event: LifecycleEvent) {
        tracing::info!(
            target: "lifecycle",
            event_type = %event.event_type,
            task_id = %event.task_id,
            agent_id = %event.agent_id,
            delegation_id = ?event.delegation_id,
            timestamp = %event.timestamp,
            details = ?event.details,
            "lifecycle_event"
        );
    }
}

/// Initialize tracing subscriber with JSON for structured logging.
pub fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
        .try_init();
}
