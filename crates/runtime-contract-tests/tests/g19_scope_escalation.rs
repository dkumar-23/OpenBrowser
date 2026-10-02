//! G19 — Scope Escalation Denied (Priority P0)
//!
//! Observable: a `Scope::Read` capability cannot authorize a write-class
//! action via `check_with_caps`; `Scope::All` can; and `has_scoped` agrees
//! with `Scope::covers`.

use runtime_auth::{AgentIdentity, HumanId};
use runtime_policy::{Capability, CapabilitySet, Decision, PolicyEngine, Scope};
use uuid::Uuid;

#[test]
fn read_scope_cannot_authorize_write_class_action() {
    let mut policy = PolicyEngine::new();
    policy.add_capability("http.post");

    let agent = AgentIdentity::new(HumanId(Uuid::new_v4()));

    let mut read_caps = CapabilitySet::new();
    read_caps.grant(Capability::new("http.post", Scope::Read, None));
    assert!(
        matches!(
            policy.check_with_caps(&agent, &read_caps, "http.post"),
            Decision::Deny { .. }
        ),
        "Read-scope capability must NOT authorize the write-class action http.post"
    );

    let mut all_caps = CapabilitySet::new();
    all_caps.grant(Capability::new("http.post", Scope::All, None));
    assert!(
        matches!(
            policy.check_with_caps(&agent, &all_caps, "http.post"),
            Decision::Allow
        ),
        "All-scope capability must authorize the write-class action http.post"
    );
}

#[test]
fn has_scoped_matches_scope_covers() {
    let mut read_caps = CapabilitySet::new();
    read_caps.grant(Capability::new("op", Scope::Read, None));
    assert!(read_caps.has_scoped("op", &Scope::Read));
    assert!(!read_caps.has_scoped("op", &Scope::Write), "Read must not cover Write");
    assert!(!read_caps.has_scoped("op", &Scope::All), "Read must not cover All");

    let mut write_caps = CapabilitySet::new();
    write_caps.grant(Capability::new("op", Scope::Write, None));
    assert!(write_caps.has_scoped("op", &Scope::Read), "Write must cover Read");
    assert!(write_caps.has_scoped("op", &Scope::Write));
    assert!(!write_caps.has_scoped("op", &Scope::All), "Write must not cover All");

    let mut all_caps = CapabilitySet::new();
    all_caps.grant(Capability::new("op", Scope::All, None));
    assert!(all_caps.has_scoped("op", &Scope::Read));
    assert!(all_caps.has_scoped("op", &Scope::Write));
    assert!(all_caps.has_scoped("op", &Scope::All));
}
