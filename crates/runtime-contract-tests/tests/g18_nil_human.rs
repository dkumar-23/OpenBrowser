//! G18 — Nil-Human Identity Cannot Mint Credentials (Priority P1)
//!
//! Observable: `issue_for_identity` rejects an identity whose `HumanId` is nil
//! and accepts one backed by a real human; `PolicyEngine` denies a nil-human
//! identity even when the action is allow-listed and a capability is present.

use runtime_auth::{AgentIdentity, BrokerError, CredentialBroker, HumanId, InMemoryBroker};
use runtime_policy::{Capability, CapabilitySet, Decision, PolicyEngine, Scope};
use uuid::Uuid;

#[test]
fn nil_human_identity_cannot_mint_credentials() {
    let broker = InMemoryBroker::default();

    let nil_identity = AgentIdentity::default();
    assert!(nil_identity.human.is_nil(), "default identity must carry the nil human");

    match broker.issue_for_identity(&nil_identity, "read", None) {
        Err(BrokerError::NilHumanLineage) => {}
        Ok(_) => panic!("nil-human lineage must not mint credentials"),
    }

    let real = AgentIdentity::new(HumanId(Uuid::new_v4()));
    let handle = broker
        .issue_for_identity(&real, "read", Some(3600))
        .expect("real-human lineage must mint credentials");
    assert!(
        broker.validate_for(&real.agent_id, "read", &handle),
        "credential minted for a real human must validate"
    );
}

#[test]
fn nil_human_identity_is_denied_by_policy() {
    let mut policy = PolicyEngine::new();
    policy.add_capability("http.get");

    let nil_identity = AgentIdentity::default();
    let mut caps = CapabilitySet::new();
    caps.grant(Capability::new("http.get", Scope::All, None));

    assert!(
        matches!(
            policy.check_with_caps(&nil_identity, &caps, "http.get"),
            Decision::Deny { .. }
        ),
        "nil-human identity must be denied even with an allow-listed action and capability"
    );

    let real = AgentIdentity::new(HumanId(Uuid::new_v4()));
    assert!(
        matches!(
            policy.check_with_caps(&real, &caps, "http.get"),
            Decision::Allow
        ),
        "real-human identity with the capability must be allowed"
    );
}
