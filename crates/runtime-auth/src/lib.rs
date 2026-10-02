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

impl HumanId {
    /// A fresh, real human authority id.
    pub fn new() -> Self { Self(Uuid::new_v4()) }
    /// The nil human — the default placeholder. Brokers reject nil lineage.
    pub fn nil() -> Self { Self(Uuid::nil()) }
    /// Whether this is the nil human (no real human authority behind it).
    pub fn is_nil(&self) -> bool { self.0 == Uuid::nil() }
}

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

impl DelegationChain {
    /// Linkage coherence: each link's `to` equals the next link's `from`.
    pub fn is_linked(&self) -> bool {
        self.links.windows(2).all(|w| w[0].to == w[1].from)
    }

    /// Coherence for the agent carrying this chain, rooted at its human:
    /// - an empty chain means the agent acts under direct human authority; it
    ///   carries no delegation links and is coherent only under a real
    ///   (non-nil) human — the empty chain itself grants no delegated
    ///   authority;
    /// - a non-empty chain must be contiguous, rooted at the carrying
    ///   identity's human authority (`links[0].from` equals the human's id),
    ///   and terminate at `agent`.
    /// A nil human can never be coherent.
    pub fn is_coherent(&self, agent: &AgentId, human: &HumanId) -> bool {
        if human.is_nil() {
            return false;
        }
        if self.links.is_empty() {
            return true;
        }
        let rooted_at_human = self.links.first().map(|l| l.from.0 == human.0).unwrap_or(false);
        rooted_at_human
            && self.is_linked()
            && self.links.last().map(|l| l.to == *agent).unwrap_or(false)
    }

    /// Whether any link is past its `expires_at`.
    pub fn is_expired(&self) -> bool {
        let now = chrono::Utc::now();
        self.links.iter().any(|l| l.expires_at.map(|e| now > e).unwrap_or(false))
    }
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
    /// Low-level issue by raw `AgentId` — carries no human lineage, so it is
    /// only for broker-internal/low-level use. Production callers MUST use
    /// [`CredentialBroker::issue_for_identity`], which enforces human lineage
    /// and rejects the nil human.
    fn issue(&self, agent: &AgentId, scope: &str) -> AuthHandle;
    fn revoke(&self, handle: &AuthHandle) -> bool;
    fn validate(&self, handle: &AuthHandle) -> bool;

    /// Issue with a TTL in seconds (`None` = no expiry). Default: TTL ignored.
    fn issue_with_ttl(&self, agent: &AgentId, scope: &str, ttl_seconds: Option<i64>) -> AuthHandle {
        let _ = ttl_seconds;
        self.issue(agent, scope)
    }

    /// Issue for a full identity. Rejects nil-human lineage: credentials only
    /// ever flow from a real human authority. Default: nil check + delegate.
    fn issue_for_identity(
        &self,
        identity: &AgentIdentity,
        scope: &str,
        ttl_seconds: Option<i64>,
    ) -> Result<AuthHandle, BrokerError> {
        if identity.human.is_nil() {
            return Err(BrokerError::NilHumanLineage);
        }
        Ok(self.issue_with_ttl(&identity.agent_id, scope, ttl_seconds))
    }

    /// Validate with agent + scope binding: the handle must have been issued
    /// to this exact agent for this exact scope. Default: binding ignored,
    /// delegates to `validate()` for backward compatibility.
    fn validate_for(&self, agent: &AgentId, scope: &str, handle: &AuthHandle) -> bool {
        let _ = (agent, scope);
        self.validate(handle)
    }
}

/// Credential issuance failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerError {
    /// The identity has no real human authority (nil HumanId) behind it.
    NilHumanLineage,
}

impl std::fmt::Display for BrokerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BrokerError::NilHumanLineage => write!(f, "nil human lineage rejected"),
        }
    }
}

impl std::error::Error for BrokerError {}

/// In-memory broker (real implementation).
#[derive(Debug, Default)]
pub struct InMemoryBroker {
    handles: std::sync::Mutex<std::collections::HashMap<Vec<u8>, HandleMeta>>,
}

#[allow(dead_code)] // issued_at is kept for audit; validation consults expires_at/revoked only
#[derive(Debug, Clone)]
struct HandleMeta {
    agent_id: AgentId,
    scope: String,
    issued_at: chrono::DateTime<chrono::Utc>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked: bool,
}

impl InMemoryBroker {
    fn issue_inner(
        &self,
        agent: &AgentId,
        scope: &str,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> AuthHandle {
        let handle = AuthHandle::new("in-memory-broker");
        let mut handles = self.handles.lock().unwrap();
        handles.insert(handle.opaque.to_vec(), HandleMeta {
            agent_id: *agent,
            scope: scope.to_string(),
            issued_at: chrono::Utc::now(),
            expires_at,
            revoked: false,
        });
        handle
    }
}

impl CredentialBroker for InMemoryBroker {
    fn issue(&self, agent: &AgentId, scope: &str) -> AuthHandle {
        self.issue_inner(agent, scope, None)
    }
    fn issue_with_ttl(&self, agent: &AgentId, scope: &str, ttl_seconds: Option<i64>) -> AuthHandle {
        let expires_at = ttl_seconds.map(|s| chrono::Utc::now() + chrono::Duration::seconds(s));
        self.issue_inner(agent, scope, expires_at)
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
    fn validate_for(&self, agent: &AgentId, scope: &str, handle: &AuthHandle) -> bool {
        let handles = self.handles.lock().unwrap();
        if let Some(meta) = handles.get(&handle.opaque.to_vec()) {
            if meta.agent_id != *agent || meta.scope != scope { return false; }
            if meta.revoked { return false; }
            if let Some(expires) = meta.expires_at {
                if chrono::Utc::now() > expires { return false; }
            }
            true
        } else { false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(links: Vec<(AgentId, AgentId, Option<chrono::DateTime<chrono::Utc>>)>) -> DelegationChain {
        DelegationChain {
            links: links.into_iter().map(|(from, to, expires_at)| DelegationLink {
                from, to, granted_at: chrono::Utc::now(), expires_at,
            }).collect(),
        }
    }

    #[test]
    fn broker_issue_without_ttl_never_expires() {
        let broker = InMemoryBroker::default();
        let h = broker.issue(&AgentId::new(), "read");
        assert!(broker.validate(&h));
    }

    #[test]
    fn broker_issue_with_ttl_expires() {
        let broker = InMemoryBroker::default();
        let agent = AgentId::new();
        let h = broker.issue_with_ttl(&agent, "read", Some(-1));
        assert!(!broker.validate(&h), "handle issued with past TTL must be invalid immediately");
        let h2 = broker.issue_with_ttl(&agent, "read", Some(3600));
        assert!(broker.validate(&h2), "handle issued with future TTL must be valid");
    }

    #[test]
    fn broker_validate_for_honors_agent_binding() {
        let broker = InMemoryBroker::default();
        let agent = AgentId::new();
        let other = AgentId::new();
        let h = broker.issue(&agent, "read");
        assert!(broker.validate_for(&agent, "read", &h));
        assert!(!broker.validate_for(&other, "read", &h), "handle must be bound to its issuing agent");
    }

    #[test]
    fn broker_validate_for_honors_scope_binding() {
        let broker = InMemoryBroker::default();
        let agent = AgentId::new();
        let h = broker.issue(&agent, "read");
        assert!(broker.validate_for(&agent, "read", &h));
        assert!(!broker.validate_for(&agent, "write", &h), "handle must not authorize a cross-scope use");
    }

    #[test]
    fn broker_validate_for_honors_revocation() {
        let broker = InMemoryBroker::default();
        let agent = AgentId::new();
        let h = broker.issue(&agent, "read");
        assert!(broker.validate_for(&agent, "read", &h));
        assert!(broker.revoke(&h));
        assert!(!broker.validate_for(&agent, "read", &h));
    }

    #[test]
    fn broker_issue_for_identity_rejects_nil_human() {
        let broker = InMemoryBroker::default();
        let identity = AgentIdentity::default();
        assert_eq!(identity.human, HumanId::nil());
        match broker.issue_for_identity(&identity, "read", None) {
            Err(BrokerError::NilHumanLineage) => {}
            Ok(_) => panic!("nil-human lineage must be rejected at issue"),
        }
    }

    #[test]
    fn broker_issue_for_identity_accepts_real_human() {
        let broker = InMemoryBroker::default();
        let identity = AgentIdentity::new(HumanId(Uuid::new_v4()));
        let h = broker.issue_for_identity(&identity, "read", Some(3600)).expect("real human must be accepted");
        assert!(broker.validate_for(&identity.agent_id, "read", &h));
    }

    #[test]
    fn human_id_helpers() {
        assert!(HumanId::nil().is_nil());
        assert!(!HumanId(Uuid::new_v4()).is_nil());
    }

    #[test]
    fn empty_chain_is_coherent_only_under_real_human() {
        let agent = AgentId::new();
        let c = DelegationChain::default();
        assert!(c.is_coherent(&agent, &HumanId::new()));
        assert!(!c.is_coherent(&agent, &HumanId::nil()), "nil human can never be coherent");
        assert!(!c.is_expired());
    }

    #[test]
    fn chain_rooted_at_human_and_terminating_at_agent_is_coherent() {
        let human = HumanId::new();
        let root = AgentId(human.0);
        let b = AgentId::new();
        let c = chain(vec![(root, b, None)]);
        assert!(c.is_coherent(&b, &human));
        assert!(!c.is_coherent(&root, &human), "chain must terminate at the carrying agent");
    }

    #[test]
    fn chain_rooted_at_arbitrary_agent_is_incoherent() {
        let human = HumanId::new();
        let arbitrary = AgentId::new();
        let b = AgentId::new();
        let c = chain(vec![(arbitrary, b, None)]);
        assert!(
            !c.is_coherent(&b, &human),
            "a chain not rooted at the carrying identity's human must be incoherent"
        );
    }

    #[test]
    fn broken_linkage_is_incoherent() {
        let human = HumanId::new();
        let a = AgentId(human.0);
        let b = AgentId::new();
        let c = AgentId::new();
        let d = AgentId::new();
        let broken = chain(vec![(a, b, None), (c, d, None)]);
        assert!(!broken.is_coherent(&d, &human));
    }

    #[test]
    fn chain_expiry_is_per_link() {
        let a = AgentId::new();
        let b = AgentId::new();
        let c = AgentId::new();
        let now = chrono::Utc::now();
        let expired = chain(vec![(a, b, Some(now - chrono::Duration::seconds(1))), (b, c, None)]);
        assert!(expired.is_expired());
        let future = chain(vec![(a, b, Some(now + chrono::Duration::seconds(3600)))]);
        assert!(!future.is_expired());
    }
}
