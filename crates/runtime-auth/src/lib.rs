use uuid::Uuid;

/// Agent identity: first-class per context.md requirement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct AgentId(pub Uuid);

impl AgentId {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
}

/// Human authority source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct HumanId(pub Uuid);

/// One link in a delegation chain.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DelegationLink {
    pub from: AgentId,
    pub to: AgentId,
    pub granted_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Full delegation chain.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct DelegationChain {
    pub links: Vec<DelegationLink>,
}

/// Agent identity with full lineage.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AgentIdentity {
    pub agent_id: AgentId,
    pub human: HumanId,
    pub delegation_chain: DelegationChain,
}

impl AgentIdentity {
    pub fn new(human: HumanId) -> Self {
        Self { agent_id: AgentId::new(), human, delegation_chain: DelegationChain::default() }
    }
}

/// Opaque credential handle — credentials are NOT passed as raw strings.
#[derive(Clone, Debug)]
pub struct AuthHandle {
    pub opaque: [u8; 32],
    pub broker_id: String,
}

impl AuthHandle {
    pub fn new(broker_id: &str) -> Self {
        Self { opaque: rand::random(), broker_id: broker_id.to_string() }
    }
}

impl Default for AgentId { fn default() -> Self { Self::new() } }
impl Default for HumanId { fn default() -> Self { Self(Uuid::nil()) } }
impl Default for AgentIdentity { fn default() -> Self { Self::new(HumanId::default()) } }

/// Credential broker trait — runtime enforces, not LLM self-assertion.
pub trait CredentialBroker: Send + Sync {
    fn issue(&self, agent: &AgentId, scope: &str) -> AuthHandle;
    fn revoke(&self, handle: &AuthHandle) -> bool;
    fn validate(&self, handle: &AuthHandle) -> bool;
}

/// In-memory broker stub (Phase 1).
#[derive(Debug, Default)]
pub struct InMemoryBroker {
    handles: std::sync::Mutex<std::collections::HashMap<Vec<u8>, HandleMeta>>,
}

// agent_id/scope/issued_at are recorded for the future real broker;
// the Phase 1 stub only consults revoked/expires_at.
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct HandleMeta {
    agent_id: AgentId,
    scope: String,
    issued_at: chrono::DateTime<chrono::Utc>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked: bool,
}

impl CredentialBroker for InMemoryBroker {
    fn issue(&self, agent: &AgentId, scope: &str) -> AuthHandle {
        let handle = AuthHandle::new("in-memory-broker");
        let mut handles = self.handles.lock().unwrap();
        handles.insert(handle.opaque.to_vec(), HandleMeta {
            agent_id: *agent,
            scope: scope.to_string(),
            issued_at: chrono::Utc::now(),
            expires_at: None,
            revoked: false,
        });
        handle
    }
    fn revoke(&self, handle: &AuthHandle) -> bool {
        let mut handles = self.handles.lock().unwrap();
        if let Some(meta) = handles.get_mut(&handle.opaque.to_vec()) {
            meta.revoked = true;
            true
        } else { false }
    }
    fn validate(&self, handle: &AuthHandle) -> bool {
        let handles = self.handles.lock().unwrap();
        if let Some(meta) = handles.get(&handle.opaque.to_vec()) {
            if meta.revoked { return false; }
            if let Some(expires) = meta.expires_at {
                if chrono::Utc::now() > expires { return false; }
            }
            true
        } else { false }
    }
}
