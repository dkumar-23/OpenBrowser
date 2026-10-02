//! G16 — Expired Handle Rejected (Priority P1)
//!
//! Observable: a handle minted with a non-positive TTL fails validation
//! immediately (both `validate` and `validate_for`), while a long-TTL handle
//! remains valid.

use runtime_auth::{AgentId, CredentialBroker, InMemoryBroker};

#[test]
fn expired_handle_is_rejected() {
    let broker = InMemoryBroker::default();
    let agent = AgentId::new();

    let negative = broker.issue_with_ttl(&agent, "read", Some(-1));
    assert!(!broker.validate(&negative), "negative-TTL handle must be invalid");
    assert!(
        !broker.validate_for(&agent, "read", &negative),
        "expired handle must fail validate_for even with matching agent+scope"
    );

    let zero = broker.issue_with_ttl(&agent, "read", Some(0));
    std::thread::sleep(std::time::Duration::from_millis(2));
    assert!(!broker.validate(&zero), "zero-TTL handle must expire immediately");

    let valid = broker.issue_with_ttl(&agent, "read", Some(3600));
    assert!(broker.validate(&valid), "future-TTL handle must stay valid");
}
