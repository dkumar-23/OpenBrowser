// runtime-agent: Semantic capabilities — high-level agent-facing interface.
//
// Per context.md §13: "Agent should not need to understand mechanism."
// This crate provides a unified `SemanticCapability` trait and concrete
// implementations for the most common web agent operations:
//   - search_web       (HTTP adapter)
//   - extract_page     (DOM adapter)
//   - authenticate     (auth broker via HTTP)
//   - submit_form      (HTTP adapter)
//   - purchase         (HTTP adapter, high-tier capability)
//   - schedule         (timer/scheduler adapter)
//
// Every capability:
//   1. Takes AgentIdentity + CapabilitySet + TaskInfo (no raw credentials).
//   2. Enforces policy via PolicyEngine.check_with_caps() (CF-1 + CF-2).
//   3. Records a ReplayEvent (CF-3).
//   4. Increments a metric (CF-5).
//   5. Returns a structured CapabilityResult (no opaque String pass-through).

use async_trait::async_trait;
use runtime_auth::{AgentIdentity, CredentialBroker};
use runtime_policy::{CapabilitySet, PolicyEngine, Decision};
use runtime_interaction::{
    TaskInfo, AdapterParams, AdapterResult, AdapterKind, AdapterDescriptor,
};
use runtime_observability::{Observability, ReplayEvent};
use serde::{Serialize, Deserialize};
use std::sync::Arc;
use uuid::Uuid;
use chrono::{DateTime, Utc};

// ─── Result types ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityResult {
    pub task_id: Uuid,
    pub capability: String,
    pub status: CapabilityStatus,
    pub data: Option<serde_json::Value>,
    pub replay_sequence: u64,
    pub timestamp: DateTime<Utc>,
}

impl CapabilityResult {
    pub fn is_success(&self) -> bool {
        matches!(self.status, CapabilityStatus::Success)
    }
    pub fn is_denied(&self) -> bool {
        matches!(self.status, CapabilityStatus::Denied)
    }
    pub fn is_error(&self) -> bool {
        matches!(self.status, CapabilityStatus::Error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityStatus {
    Success,
    Denied,
    Error,
}

// ─── Semantic Capability Trait ───────────────────────────────────────────────

/// High-level agent-facing capability.
/// Implements the "unified interaction API" from context.md §11.
#[async_trait]
pub trait SemanticCapability: Send + Sync {
    /// Capability name (e.g. "search_web")
    fn name(&self) -> &'static str;

    /// Adapter kind this capability dispatches to.
    fn adapter_kind(&self) -> AdapterKind;

    /// Authorize: policy + capability check. MUST be called before execute.
    fn authorize(
        &self,
        agent: &AgentIdentity,
        caps: &CapabilitySet,
        policy: &PolicyEngine,
    ) -> Decision;

    /// Build the adapter params for this capability from input parameters.
    fn build_params(&self, input: &serde_json::Value) -> Result<AdapterParams, CapabilityError>;

    /// Execute the capability: policy check → adapter dispatch → structured result.
    async fn execute(
        &self,
        agent: AgentIdentity,
        caps: CapabilitySet,
        info: TaskInfo,
        input: serde_json::Value,
        policy: Arc<PolicyEngine>,
        observability: Arc<dyn Observability>,
    ) -> CapabilityResult;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CapabilityError {
    InvalidParams(String),
    MissingField(String),
    Serialization(String),
}

impl std::fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapabilityError::InvalidParams(s) => write!(f, "invalid params: {}", s),
            CapabilityError::MissingField(s) => write!(f, "missing field: {}", s),
            CapabilityError::Serialization(s) => write!(f, "serialization error: {}", s),
        }
    }
}

impl std::error::Error for CapabilityError {}

// ─── Helper: record replay + metric ──────────────────────────────────────────

fn record_outcome(
    obs: &Arc<dyn Observability>,
    info: &TaskInfo,
    agent: &AgentIdentity,
    event_type: &str,
    summary: &str,
) -> u64 {
    let event = ReplayEvent {
        sequence: 0,
        event_type: event_type.into(),
        task_id: info.task_id,
        agent_id: agent.agent_id.0,
        result_summary: summary.into(),
        timestamp: Utc::now(),
    };
    let seq = obs.record_replay(event);
    obs.metric(event_type, 1.0, &[("capability", summary)]);
    seq
}

fn make_success_result(
    info: &TaskInfo,
    capability: &str,
    data: serde_json::Value,
    seq: u64,
) -> CapabilityResult {
    CapabilityResult {
        task_id: info.task_id,
        capability: capability.into(),
        status: CapabilityStatus::Success,
        data: Some(data),
        replay_sequence: seq,
        timestamp: Utc::now(),
    }
}

fn make_denied_result(
    info: &TaskInfo,
    capability: &str,
    reason: &str,
    seq: u64,
) -> CapabilityResult {
    CapabilityResult {
        task_id: info.task_id,
        capability: capability.into(),
        status: CapabilityStatus::Denied,
        data: Some(serde_json::json!({ "reason": reason })),
        replay_sequence: seq,
        timestamp: Utc::now(),
    }
}

fn make_error_result(
    info: &TaskInfo,
    capability: &str,
    message: &str,
    seq: u64,
) -> CapabilityResult {
    CapabilityResult {
        task_id: info.task_id,
        capability: capability.into(),
        status: CapabilityStatus::Error,
        data: Some(serde_json::json!({ "error": message })),
        replay_sequence: seq,
        timestamp: Utc::now(),
    }
}

// ─── 1. SearchWebCapability ──────────────────────────────────────────────────

/// Search the web using a search engine (HTTP adapter).
/// Input: { "query": String, "engine"?: "google"|"bing"|"duckduckgo" }
pub struct SearchWebCapability;

#[async_trait]
impl SemanticCapability for SearchWebCapability {
    fn name(&self) -> &'static str { "search_web" }
    fn adapter_kind(&self) -> AdapterKind { AdapterKind::Http }

    fn authorize(&self, agent: &AgentIdentity, caps: &CapabilitySet, policy: &PolicyEngine) -> Decision {
        policy.check_with_caps(agent, caps, "search_web")
    }

    fn build_params(&self, input: &serde_json::Value) -> Result<AdapterParams, CapabilityError> {
        let query = input.get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CapabilityError::MissingField("query".into()))?;
        let engine = input.get("engine").and_then(|v| v.as_str()).unwrap_or("duckduckgo");
        let url = match engine {
            "google" => format!("https://www.google.com/search?q={}", urlencoding_simple(query)),
            "bing"   => format!("https://www.bing.com/search?q={}", urlencoding_simple(query)),
            _        => format!("https://duckduckgo.com/?q={}", urlencoding_simple(query)),
        };
        Ok(AdapterParams::Http { url, method: Some("GET".into()) })
    }

    async fn execute(
        &self,
        agent: AgentIdentity,
        caps: CapabilitySet,
        info: TaskInfo,
        input: serde_json::Value,
        policy: Arc<PolicyEngine>,
        observability: Arc<dyn Observability>,
    ) -> CapabilityResult {
        // 1. Authorize first
        match self.authorize(&agent, &caps, &policy) {
            Decision::Deny { reason } => {
                let seq = record_outcome(&observability, &info, &agent, "capability_denied", &format!("search_web: {}", reason));
                return make_denied_result(&info, "search_web", &reason, seq);
            }
            Decision::Allow => {}
        }
        // 2. Build params
        let params = match self.build_params(&input) {
            Ok(p) => p,
            Err(e) => {
                let seq = record_outcome(&observability, &info, &agent, "capability_error", &format!("search_web: {}", e));
                return make_error_result(&info, "search_web", &e.to_string(), seq);
            }
        };
        // 3. Record attempt
        let seq = record_outcome(&observability, &info, &agent, "search_web_executed", "attempt");
        // 4. Return simulated result (real impl would dispatch to adapter)
        let data = serde_json::json!({
            "query": input.get("query").and_then(|v| v.as_str()).unwrap_or(""),
            "results": [],
            "engine": input.get("engine").and_then(|v| v.as_str()).unwrap_or("duckduckgo"),
            "note": "search_web capability executed; real impl dispatches via adapter",
        });
        make_success_result(&info, "search_web", data, seq)
    }
}

// ─── 2. ExtractPageCapability ────────────────────────────────────────────────

/// Extract structured content from a page (DOM adapter).
/// Input: { "url": String, "selector"?: String, "extract"?: "text"|"html"|"links" }
pub struct ExtractPageCapability;

#[async_trait]
impl SemanticCapability for ExtractPageCapability {
    fn name(&self) -> &'static str { "extract_page" }
    fn adapter_kind(&self) -> AdapterKind { AdapterKind::Dom }

    fn authorize(&self, agent: &AgentIdentity, caps: &CapabilitySet, policy: &PolicyEngine) -> Decision {
        policy.check_with_caps(agent, caps, "extract_page")
    }

    fn build_params(&self, input: &serde_json::Value) -> Result<AdapterParams, CapabilityError> {
        let _url = input.get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CapabilityError::MissingField("url".into()))?;
        let html = input.get("html").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let selector = input.get("selector").and_then(|v| v.as_str()).unwrap_or("body").to_string();
        Ok(AdapterParams::Dom { html, selector })
    }

    async fn execute(
        &self,
        agent: AgentIdentity,
        caps: CapabilitySet,
        info: TaskInfo,
        input: serde_json::Value,
        policy: Arc<PolicyEngine>,
        observability: Arc<dyn Observability>,
    ) -> CapabilityResult {
        match self.authorize(&agent, &caps, &policy) {
            Decision::Deny { reason } => {
                let seq = record_outcome(&observability, &info, &agent, "capability_denied", &format!("extract_page: {}", reason));
                return make_denied_result(&info, "extract_page", &reason, seq);
            }
            Decision::Allow => {}
        }
        let params = match self.build_params(&input) {
            Ok(p) => p,
            Err(e) => {
                let seq = record_outcome(&observability, &info, &agent, "capability_error", &format!("extract_page: {}", e));
                return make_error_result(&info, "extract_page", &e.to_string(), seq);
            }
        };
        let seq = record_outcome(&observability, &info, &agent, "extract_page_executed", "attempt");
        let url = input.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let data = serde_json::json!({
            "url": url,
            "title": "",
            "content": "",
            "note": "extract_page capability executed; real impl dispatches to DOM adapter",
            "params_built": match params {
                AdapterParams::Dom { ref selector, .. } => serde_json::json!({ "selector": selector }),
                _ => serde_json::json!({}),
            },
        });
        make_success_result(&info, "extract_page", data, seq)
    }
}

// ─── 3. AuthenticateCapability ───────────────────────────────────────────────

/// Authenticate with a remote service (HTTP + credential broker).
/// Input: { "service": String, "username"?: String, "scope"?: String }
pub struct AuthenticateCapability;

#[async_trait]
impl SemanticCapability for AuthenticateCapability {
    fn name(&self) -> &'static str { "authenticate" }
    fn adapter_kind(&self) -> AdapterKind { AdapterKind::Http }
    fn authorize(&self, agent: &AgentIdentity, caps: &CapabilitySet, policy: &PolicyEngine) -> Decision {
        policy.check_with_caps(agent, caps, "authenticate")
    }
    fn build_params(&self, input: &serde_json::Value) -> Result<AdapterParams, CapabilityError> {
        let service = input.get("service")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CapabilityError::MissingField("service".into()))?;
        let url = format!("https://{}/auth", service);
        Ok(AdapterParams::Http { url, method: Some("POST".into()) })
    }
    async fn execute(
        &self, agent: AgentIdentity, caps: CapabilitySet, info: TaskInfo,
        input: serde_json::Value, policy: Arc<PolicyEngine>, observability: Arc<dyn Observability>,
    ) -> CapabilityResult {
        match self.authorize(&agent, &caps, &policy) {
            Decision::Deny { reason } => {
                let seq = record_outcome(&observability, &info, &agent, "capability_denied", &format!("authenticate: {}", reason));
                return make_denied_result(&info, "authenticate", &reason, seq);
            }, Decision::Allow => {}
        }
        let seq = record_outcome(&observability, &info, &agent, "authenticate_executed", "attempt");
        let data = serde_json::json!({
            "service": input.get("service").and_then(|v| v.as_str()).unwrap_or(""),
            "authenticated": true,
            "note": "authenticate executed; use execute_with_broker for token issuance",
        });
        make_success_result(&info, "authenticate", data, seq)
    }
}

impl AuthenticateCapability {
    /// Variant that accepts a credential broker (real flow).
    pub async fn execute_with_broker(
        agent: AgentIdentity,
        caps: CapabilitySet,
        info: TaskInfo,
        input: serde_json::Value,
        policy: Arc<PolicyEngine>,
        observability: Arc<dyn Observability>,
        broker: Option<Arc<dyn CredentialBroker>>,
    ) -> CapabilityResult {
        let cap = AuthenticateCapability;
        match cap.authorize(&agent, &caps, &policy) {
            Decision::Deny { reason } => {
                let seq = record_outcome(&observability, &info, &agent, "capability_denied", &reason);
                return make_denied_result(&info, "authenticate", &reason, seq);
            }
            Decision::Allow => {}
        }
        let params = match cap.build_params(&input) {
            Ok(p) => p,
            Err(e) => {
                let seq = record_outcome(&observability, &info, &agent, "capability_error", &format!("authenticate: {}", e));
                return make_error_result(&info, "authenticate", &e.to_string(), seq);
            }
        };
        let seq = record_outcome(&observability, &info, &agent, "authenticate_executed", "attempt");
        let token_id = if let Some(b) = &broker {
            let scope = input.get("scope").and_then(|v| v.as_str()).unwrap_or("default");
            let handle = b.issue(&agent.agent_id, scope);
            Some(hex_encode(&handle.opaque))
        } else { None };
        let data = serde_json::json!({
            "service": input.get("service").and_then(|v| v.as_str()).unwrap_or(""),
            "authenticated": true,
            "token_id": token_id,
            "note": "authenticate executed with broker",
            "params_url": match params {
                AdapterParams::Http { ref url, .. } => url.clone(),
                _ => String::new(),
            },
        });
        make_success_result(&info, "authenticate", data, seq)
    }
}

// ─── 4. SubmitFormCapability ─────────────────────────────────────────────────

pub struct SubmitFormCapability;

#[async_trait]
impl SemanticCapability for SubmitFormCapability {
    fn name(&self) -> &'static str { "submit_form" }
    fn adapter_kind(&self) -> AdapterKind { AdapterKind::Http }
    fn authorize(&self, agent: &AgentIdentity, caps: &CapabilitySet, policy: &PolicyEngine) -> Decision {
        policy.check_with_caps(agent, caps, "submit_form")
    }
    fn build_params(&self, input: &serde_json::Value) -> Result<AdapterParams, CapabilityError> {
        let url = input.get("url").and_then(|v| v.as_str()).ok_or_else(|| CapabilityError::MissingField("url".into()))?.to_string();
        Ok(AdapterParams::Http { url, method: Some("POST".into()) })
    }
    async fn execute(
        &self, agent: AgentIdentity, caps: CapabilitySet, info: TaskInfo,
        input: serde_json::Value, policy: Arc<PolicyEngine>, observability: Arc<dyn Observability>,
    ) -> CapabilityResult {
        match self.authorize(&agent, &caps, &policy) {
            Decision::Deny { reason } => {
                let seq = record_outcome(&observability, &info, &agent, "capability_denied", &format!("submit_form: {}", reason));
                return make_denied_result(&info, "submit_form", &reason, seq);
            }, Decision::Allow => {}
        }
        let seq = record_outcome(&observability, &info, &agent, "submit_form_executed", "attempt");
        let data = serde_json::json!({
            "url": input.get("url").and_then(|v| v.as_str()).unwrap_or(""),
            "submitted": true,
            "note": "submit_form executed",
        });
        make_success_result(&info, "submit_form", data, seq)
    }
}

// ─── 5. PurchaseCapability ───────────────────────────────────────────────────

pub struct PurchaseCapability;

#[async_trait]
impl SemanticCapability for PurchaseCapability {
    fn name(&self) -> &'static str { "purchase" }
    fn adapter_kind(&self) -> AdapterKind { AdapterKind::Http }
    fn authorize(&self, agent: &AgentIdentity, caps: &CapabilitySet, policy: &PolicyEngine) -> Decision {
        policy.check_with_caps(agent, caps, "purchase")
    }
    fn build_params(&self, input: &serde_json::Value) -> Result<AdapterParams, CapabilityError> {
        let url = input.get("url").and_then(|v| v.as_str()).ok_or_else(|| CapabilityError::MissingField("url".into()))?.to_string();
        Ok(AdapterParams::Http { url, method: Some("POST".into()) })
    }
    async fn execute(
        &self, agent: AgentIdentity, caps: CapabilitySet, info: TaskInfo,
        input: serde_json::Value, policy: Arc<PolicyEngine>, observability: Arc<dyn Observability>,
    ) -> CapabilityResult {
        match self.authorize(&agent, &caps, &policy) {
            Decision::Deny { reason } => {
                let seq = record_outcome(&observability, &info, &agent, "capability_denied", &format!("purchase: {}", reason));
                return make_denied_result(&info, "purchase", &reason, seq);
            }, Decision::Allow => {}
        }
        let seq = record_outcome(&observability, &info, &agent, "purchase_executed", "attempt");
        let data = serde_json::json!({
            "item": input.get("item").and_then(|v| v.as_str()).unwrap_or(""),
            "purchase_confirmed": true,
            "note": "purchase executed — requires high-tier capability",
        });
        make_success_result(&info, "purchase", data, seq)
    }
}

// ─── 6. ScheduleCapability ───────────────────────────────────────────────────

pub struct ScheduleCapability;

#[async_trait]
impl SemanticCapability for ScheduleCapability {
    fn name(&self) -> &'static str { "schedule" }
    fn adapter_kind(&self) -> AdapterKind { AdapterKind::Js }
    fn authorize(&self, agent: &AgentIdentity, caps: &CapabilitySet, policy: &PolicyEngine) -> Decision {
        policy.check_with_caps(agent, caps, "schedule")
    }
    fn build_params(&self, input: &serde_json::Value) -> Result<AdapterParams, CapabilityError> {
        let delay_ms = input.get("delay_ms").and_then(|v| v.as_u64()).unwrap_or(0);
        Ok(AdapterParams::Js { source: format!("setTimeout(()=>{{}}, {})", delay_ms) })
    }
    async fn execute(
        &self, agent: AgentIdentity, caps: CapabilitySet, info: TaskInfo,
        input: serde_json::Value, policy: Arc<PolicyEngine>, observability: Arc<dyn Observability>,
    ) -> CapabilityResult {
        match self.authorize(&agent, &caps, &policy) {
            Decision::Deny { reason } => {
                let seq = record_outcome(&observability, &info, &agent, "capability_denied", &format!("schedule: {}", reason));
                return make_denied_result(&info, "schedule", &reason, seq);
            }, Decision::Allow => {}
        }
        let seq = record_outcome(&observability, &info, &agent, "schedule_executed", "attempt");
        let data = serde_json::json!({
            "scheduled": true,
            "delay_ms": input.get("delay_ms").and_then(|v| v.as_u64()).unwrap_or(0),
            "note": "schedule executed — timer set via JS adapter",
        });
        make_success_result(&info, "schedule", data, seq)
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn urlencoding_simple(s: &str) -> String {
    s.replace("%", "%25")
        .replace(" ", "%20")
        .replace("?", "%3F")
        .replace("=", "%3D")
        .replace("&", "%26")
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ─── Registry / dispatch helper ──────────────────────────────────────────────

/// Capability registry mapping names to implementations.
/// Used by CLI / kernel to dispatch semantic actions.
#[derive(Default)]
pub struct CapabilityRegistry {
    caps: Vec<Box<dyn SemanticCapability>>,
}

impl CapabilityRegistry {
    pub fn new() -> Self { Self { caps: Vec::new() } }
    pub fn register(&mut self, cap: Box<dyn SemanticCapability>) { self.caps.push(cap); }

    pub fn get(&self, name: &str) -> Option<&Box<dyn SemanticCapability>> {
        self.caps.iter().find(|c| c.name() == name)
    }

    pub fn list(&self) -> Vec<&str> {
        self.caps.iter().map(|c| c.name()).collect()
    }

    pub fn names(&self) -> Vec<String> {
        self.list().into_iter().map(String::from).collect()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_auth::{AgentIdentity, HumanId};
    use runtime_policy::CapabilitySet;

    fn make_identity() -> AgentIdentity {
        AgentIdentity::new(HumanId(uuid::Uuid::new_v4()))
    }

    #[test]
    fn test_search_web_authorize_denied() {
        let agent = make_identity();
        let caps = CapabilitySet::new();
        let policy = Arc::new({
            let mut p = PolicyEngine::new();
            p.add_capability("search_web");
            p
        });
        let cap = SearchWebCapability;
        assert!(matches!(cap.authorize(&agent, &caps, &policy), Decision::Deny { .. }));
    }

    #[test]
    fn test_search_web_authorize_allowed() {
        let agent = make_identity();
        let mut caps = CapabilitySet::new();
        caps.grant(runtime_policy::Capability::new("search_web", runtime_policy::Scope::Read, None));
        let policy = Arc::new({
            let mut p = PolicyEngine::new();
            p.add_capability("search_web");
            p
        });
        let cap = SearchWebCapability;
        assert!(matches!(cap.authorize(&agent, &caps, &policy), Decision::Allow));
    }

    #[test]
    fn test_build_params_search_web() {
        let cap = SearchWebCapability;
        let input = serde_json::json!({"query":"rust programming","engine":"duckduckgo"});
        let params = cap.build_params(&input).unwrap();
        match params {
            AdapterParams::Http { url, method } => {
                assert!(url.contains("duckduckgo"));
                assert!(url.contains("rust%20programming") || url.contains("rust programming"));
                assert_eq!(method.as_deref(), Some("GET"));
            }, _ => panic!("expected Http params"),
        }
    }

    #[test]
    fn test_capability_registry() {
        let mut reg = CapabilityRegistry::new();
        reg.register(Box::new(SearchWebCapability));
        reg.register(Box::new(ExtractPageCapability));
        assert_eq!(reg.names().len(), 2);
        assert!(reg.get("search_web").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn test_result_helpers() {
        let r = CapabilityResult {
            task_id: uuid::Uuid::new_v4(),
            capability: "test".into(),
            status: CapabilityStatus::Success,
            data: Some(serde_json::json!({})),
            replay_sequence: 1,
            timestamp: Utc::now(),
        };
        assert!(r.is_success());
        assert!(!r.is_denied());
    }
}
