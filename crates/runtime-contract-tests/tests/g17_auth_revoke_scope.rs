//! G17 — Revoked Handle Denied (Priority P1)
//!
//! Observable: `validate_for` denies a handle once it is revoked, even when
//! agent and scope still match.

use runtime_auth::{AgentId, CredentialBroker, InMemoryBroker};

#[test]
fn revoked_handle_denied_by_validate_for() {
    let broker = InMemoryBroker::default();
    let agent = AgentId::new();
    let handle = broker.issue(&agent, "read");

    assert!(broker.validate_for(&agent, "read", &handle), "fresh handle must validate");
    assert!(broker.revoke(&handle), "revoke should succeed");
    assert!(
        !broker.validate_for(&agent, "read", &handle),
        "revoked handle must fail validate_for even with matching agent+scope"
    );
    assert!(!broker.validate(&handle), "revoked handle must fail validate");
}
