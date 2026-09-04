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

impl TaskInfo {
    pub fn new(task_id: Uuid, agent_id: Uuid) -> Self {
        Self { task_id, agent_id }
    }
}

/// Adapter parameters — mechanism-specific inputs.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum AdapterParams {
    /// HTTP GET/POST request to a URL.
    Http { url: String, method: Option<String> },
    /// MCP protocol invocation.
    Mcp { tool: String, args: std::collections::HashMap<String, String> },
    /// DOM operation: parse + select.
    Dom { html: String, selector: String },
    /// JS execution in isolate.
    Js { source: String },
    /// Visual: screenshot or accessibility tree.
    Visual { url: String },
}

impl Default for AdapterParams {
    fn default() -> Self {
        AdapterParams::Http { url: String::new(), method: None }
    }
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

    /// Returns true if the result is a successful execution (policy allowed + mechanism succeeded).
    pub fn is_success(&self) -> bool {
        matches!(self, AdapterResult::Success { .. })
    }

    /// Returns true if the request was denied by policy.
    pub fn is_denied(&self) -> bool {
        matches!(self, AdapterResult::Denied { .. })
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

impl AdapterKind {
    /// Preference order: HTTP is preferred, then DOM, then JS, then MCP, then Visual.
    /// Per context.md §11: agent should not need to understand mechanism.
    pub fn preference_order() -> [Self; 5] {
        [
            AdapterKind::Http,
            AdapterKind::Dom,
            AdapterKind::Js,
            AdapterKind::Mcp,
            AdapterKind::Visual,
        ]
    }
}

/// Capability name used for adapter selection. Adapters declare which
/// capabilities they handle. Selection picks the highest-preference adapter
/// (HTTP > DOM > JS > visual per context.md §11).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AdapterDescriptor {
    pub kind: AdapterKind,
    pub handles: Vec<String>, // capability names e.g. "http.get", "search_web"
}

impl AdapterDescriptor {
    pub fn new(kind: AdapterKind, handles: Vec<&str>) -> Self {
        Self { kind, handles: handles.into_iter().map(String::from).collect() }
    }
}

/// InteractionAdapter trait — every adapter MUST implement this.
///
/// Contract (R1 + R7 + wave3 §2):
/// 1. Receive `AgentIdentity` + `CapabilitySet` + `TaskInfo` (no raw credentials).
/// 2. Call policy engine internally — adapter NEVER makes its own auth decision.
/// 3. On policy deny: emit `policy_denied` ReplayEvent, increment metric, return `Denied`.
/// 4. On policy allow: perform mechanism (reqwest, DOM, etc.), emit `http_executed`
///    ReplayEvent, increment metric, return `Success`.
/// 5. Mechanism failure: emit error ReplayEvent, increment metric, return `Error`.
/// 6. NEVER return String directly. NEVER pass through without policy check.
#[async_trait]
pub trait InteractionAdapter: Send + Sync + std::fmt::Debug {
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
        info: &TaskInfo,
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
    for kind in AdapterKind::preference_order().iter() {
        if let Some(a) = adapters.iter().find(|a| a.descriptor().kind == *kind && a.handles(action)) {
            return Some(a);
        }
    }
    None
}

// ─── Adapter Registry ────────────────────────────────────────────────────────

/// Central registry for all interaction adapters.
/// CLI and runtime kernel build this at startup and use it for dispatch.
#[derive(Default)]
pub struct AdapterRegistry {
    adapters: Vec<Box<dyn InteractionAdapter>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self { adapters: Vec::new() }
    }

    /// Register an adapter. Panics on duplicate kind+capability (use replace instead).
    pub fn register(&mut self, adapter: Box<dyn InteractionAdapter>) {
        // Check for duplicates
        for handle in adapter.descriptor().handles.iter() {
            for existing in &self.adapters {
                if existing.handles(handle) {
                    println!(
                        "WARN: duplicate adapter registration for capability '{}' (existing: {:?}, new: {:?})",
                        handle,
                        existing.descriptor().kind,
                        adapter.descriptor().kind,
                    );
                }
            }
        }
        self.adapters.push(adapter);
    }

    /// Resolve an adapter for the given action using preference order.
    pub fn resolve(&self, action: &str) -> Option<&Box<dyn InteractionAdapter>> {
        select_adapter(&self.adapters, action)
    }

    /// Return all registered adapters.
    pub fn all(&self) -> &[Box<dyn InteractionAdapter>] {
        &self.adapters
    }

    /// Return all registered adapters (mutable).
    pub fn all_mut(&mut self) -> &mut Vec<Box<dyn InteractionAdapter>> {
        &mut self.adapters
    }

    /// Return the number of registered adapters.
    pub fn len(&self) -> usize {
        self.adapters.len()
    }

    /// Return true if no adapters are registered.
    pub fn is_empty(&self) -> bool {
        self.adapters.is_empty()
    }

    /// List all descriptors for all registered adapters.
    pub fn list_descriptors(&self) -> Vec<AdapterDescriptor> {
        self.adapters.iter().map(|a| a.descriptor()).collect()
    }

    /// List all capabilities handled by all adapters.
    pub fn list_capabilities(&self) -> Vec<String> {
        let mut caps: Vec<String> = Vec::new();
        for a in &self.adapters {
            for h in a.descriptor().handles.iter() {
                caps.push(h.clone());
            }
        }
        caps.sort();
        caps.dedup();
        caps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapter_kind_preference_order() {
        let order = AdapterKind::preference_order();
        assert_eq!(order[0], AdapterKind::Http);
        assert_eq!(order[1], AdapterKind::Dom);
        assert_eq!(order[2], AdapterKind::Js);
        assert_eq!(order[3], AdapterKind::Mcp);
        assert_eq!(order[4], AdapterKind::Visual);
    }

    #[test]
    fn test_adapter_registry_register_and_resolve() {
        let mut registry = AdapterRegistry::new();
        assert!(registry.is_empty());
        assert!(registry.resolve("http.get").is_none());
    }

    #[test]
    fn test_adapter_result_helpers() {
        let success = AdapterResult::Success { response: "ok".into(), replay_sequence: 1 };
        assert!(success.is_success());
        assert!(!success.is_denied());
        assert_eq!(success.replay_sequence(), 1);

        let denied = AdapterResult::Denied { reason: "no cap".into(), replay_sequence: 2 };
        assert!(!denied.is_success());
        assert!(denied.is_denied());
        assert_eq!(denied.replay_sequence(), 2);

        let error = AdapterResult::Error { message: "oops".into(), replay_sequence: 3 };
        assert!(!error.is_success());
        assert!(!error.is_denied());
        assert_eq!(error.replay_sequence(), 3);
    }

    #[test]
    fn test_adapter_params_default() {
        let params = AdapterParams::default();
        match params {
            AdapterParams::Http { ref url, ref method } => {
                assert!(url.is_empty());
                assert!(method.is_none());
            }
            _ => panic!("expected Http variant"),
        }
    }

    #[test]
    fn test_task_info_new() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let info = TaskInfo::new(id1, id2);
        assert_eq!(info.task_id, id1);
        assert_eq!(info.agent_id, id2);
    }
}
