//! G9 — Auth Revocation (P1)
//!
//! Observable: revoke removes the handle; validate returns false afterwards.

use runtime_auth::{AgentId, CredentialBroker, InMemoryBroker};

#[test]
fn auth_revoke_works() {
    let broker = InMemoryBroker::default();
    let agent = AgentId::new();
    let h = broker.issue(&agent, "scope");
    assert!(broker.validate(&h), "new handle should be valid");
    assert!(broker.revoke(&h), "revoke should return true");
    assert!(!broker.validate(&h), "revoked handle must fail validation");
}
