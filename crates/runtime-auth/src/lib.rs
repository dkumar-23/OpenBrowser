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
        Self { opaque: [42u8; 32], broker_id: broker_id.to_string() }
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
pub struct InMemoryBroker { /* stores handles */ }

impl CredentialBroker for InMemoryBroker {
    fn issue(&self, agent: &AgentId, _scope: &str) -> AuthHandle {
        AuthHandle::new("in-memory-broker")
    }
    fn revoke(&self, _handle: &AuthHandle) -> bool { true }
    fn validate(&self, _handle: &AuthHandle) -> bool { true }
}
