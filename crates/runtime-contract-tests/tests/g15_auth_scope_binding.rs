//! G15 — Scoped Handle Binding (Priority P1)
//!
//! Observable: a handle issued for one scope fails `validate_for` under a
//! different scope and succeeds only under its issuing scope.

use runtime_auth::{AgentId, CredentialBroker, InMemoryBroker};

#[test]
fn scoped_handle_rejects_cross_scope_use() {
    let broker = InMemoryBroker::default();
    let agent = AgentId::new();
    let handle = broker.issue(&agent, "read");

    assert!(
        broker.validate_for(&agent, "read", &handle),
        "handle must validate under its issuing scope"
    );
    assert!(
        !broker.validate_for(&agent, "write", &handle),
        "handle must NOT validate under a different scope"
    );
}
