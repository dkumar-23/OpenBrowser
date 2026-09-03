// runtime-interaction: Unified interaction API + adapter selection (CF-6 fix)
//
// Architecture basis: context.md §11 (unified interaction API; agent does not need
// to know mechanism) and §21 (crates only when real boundary).
// This crate defines the `InteractionAdapter` trait. All adapters (HTTP, MCP,
// visual) MUST implement it. This is the only way to prevent the CF-1
// policy-bypass from repeating in future adapters.

use async_trait::async_trait;
use runtime_auth::AgentIdentity;
use runtime_policy::CapabilitySet;
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Minimal task info passed across adapter boundary — no core dependency.
/// The adapter only needs task/agent IDs for replay/logging.
#[derive(Clone, Debug)]
pub struct TaskInfo {
    pub task_id: Uuid,
    pub agent_id: Uuid,
}

/// Adapter parameters — mechanism-specific inputs.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum AdapterParams {
    /// HTTP request to a URL (used by HttpAdapter).
    Http { url: String, method: Option<String> },
}

/// Adapter outcome — explicit, no pass-through.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum AdapterResult {
    /// Policy denied the request. No network call was made. Replay event recorded.
    Denied { reason: String, replay_sequence: u64 },
    /// Policy allowed; network call succeeded. Replay event recorded.
    Success { response: String, replay_sequence: u64 },
    /// Adapter-level error (network failure, parse, etc.) — distinct from policy denial.
    Error { message: String, replay_sequence: u64 },
}

impl AdapterResult {
    pub fn replay_sequence(&self) -> u64 {
        match self {
            AdapterResult::Denied { replay_sequence, .. } => *replay_sequence,
            AdapterResult::Success { replay_sequence, .. } => *replay_sequence,
            AdapterResult::Error { replay_sequence, .. } => *replay_sequence,
        }
    }
}

/// Adapter identifier — supports adapter selection (HTTP > DOM > JS > visual per context.md §11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AdapterKind {
    Http,
    Dom,
    Js,
    Mcp,
    Visual,
}

/// Capability name used for adapter selection. Adapters declare which
/// capabilities they handle. Selection picks the highest-preference adapter
/// (HTTP > DOM > JS > visual).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AdapterDescriptor {
    pub kind: AdapterKind,
    pub handles: Vec<String>, // capability names e.g. "http.get"
}

/// InteractionAdapter trait — every adapter MUST implement this.
///
/// Contract (R1 + R7 + wave3 §2):
/// 1. Receive `AgentIdentity` + `CapabilitySet` + `TaskContext` (no raw credentials).
/// 2. Call policy engine internally — adapter NEVER makes its own auth decision.
/// 3. On policy deny: emit `policy_denied` ReplayEvent, increment metric, return `Denied`.
/// 4. On policy allow: perform mechanism (reqwest, DOM, etc.), emit `http_executed`
///    ReplayEvent, increment metric, return `Success`.
/// 5. Mechanism failure: emit error ReplayEvent, increment metric, return `Error`.
/// 6. NEVER return String. NEVER pass through without policy check.
#[async_trait]
pub trait InteractionAdapter: Send + Sync {
    /// Adapter identity — used for selection + logging.
    fn descriptor(&self) -> AdapterDescriptor;

    /// Whether this adapter handles the given capability name.
    fn handles(&self, action: &str) -> bool {
        self.descriptor().handles.iter().any(|h| h == action)
    }

    /// Execute the action. MUST follow the contract above.
    async fn execute(
        &self,
        agent: &AgentIdentity,
        caps: &CapabilitySet,
        ctx: &TaskInfo,
        params: &AdapterParams,
    ) -> AdapterResult;
}

/// Interaction event — semantic-level observation (distinct from replay file).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct InteractionEvent {
    pub task_id: Uuid,
    pub agent_id: Uuid,
    pub adapter_kind: AdapterKind,
    pub capability: String,
    pub outcome: InteractionOutcome,
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum InteractionOutcome {
    Denied { reason: String },
    Success,
    Error { message: String },
}

/// Adapter selection: given a capability name and a list of available adapters,
/// return the first that handles it (HTTP > DOM > JS > visual per context.md §11).
pub fn select_adapter<'a>(
    adapters: &'a [Box<dyn InteractionAdapter>],
    action: &str,
) -> Option<&'a Box<dyn InteractionAdapter>> {
    // Preference order from context.md.
    let preference = [
        AdapterKind::Http,
        AdapterKind::Dom,
        AdapterKind::Js,
        AdapterKind::Mcp,
        AdapterKind::Visual,
    ];
    for kind in preference.iter() {
        if let Some(a) = adapters.iter().find(|a| a.descriptor().kind == *kind && a.handles(action)) {
            return Some(a);
        }
    }
    None
}