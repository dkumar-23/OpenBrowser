//! G8 — Auth: Unique Handles (P1)
//!
//! Observable: two handles issued to different scopes have unique opaque
//! bytes — never identical.

use runtime_auth::{AgentId, CredentialBroker, InMemoryBroker};

#[test]
fn auth_handle_unique() {
    let broker = InMemoryBroker::default();
    let agent = AgentId::new();
    let h1 = broker.issue(&agent, "scope1");
    let h2 = broker.issue(&agent, "scope2");
    assert_ne!(
        h1.opaque, h2.opaque,
        "handles for different scopes must have unique opaque bytes"
    );
}
