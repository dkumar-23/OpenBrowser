//! G20 — Delegation Chain Coherence & Expiry (Priority P0)
//!
//! Observable: `check` denies an incoherent or expired delegation chain and
//! allows a coherent, unexpired chain rooted at the carrying identity's human
//! and terminating at the carrying agent. A chain rooted at an arbitrary
//! agent is denied.

use runtime_auth::{AgentId, AgentIdentity, DelegationLink, HumanId};
use runtime_policy::{Decision, PolicyEngine};
use uuid::Uuid;

fn link(
    from: AgentId,
    to: AgentId,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
) -> DelegationLink {
    DelegationLink { from, to, granted_at: chrono::Utc::now(), expires_at }
}

#[test]
fn delegation_chain_coherence_and_expiry_are_enforced() {
    let mut policy = PolicyEngine::new();
    policy.add_capability("http.get");

    let human = HumanId(Uuid::new_v4());
    let mut agent = AgentIdentity::new(human);
    let human_root = AgentId(human.0);

    agent.delegation_chain.links = vec![link(
        human_root,
        agent.agent_id,
        Some(chrono::Utc::now() + chrono::Duration::seconds(3600)),
    )];
    assert!(
        matches!(policy.check(&agent, "http.get"), Decision::Allow),
        "coherent, unexpired chain rooted at the human must allow"
    );

    let arbitrary_root = AgentId::new();
    agent.delegation_chain.links = vec![link(arbitrary_root, agent.agent_id, None)];
    match policy.check(&agent, "http.get") {
        Decision::Deny { reason } => assert!(
            reason.contains("incoherent"),
            "expected arbitrary-root chain denial, got: {reason}"
        ),
        Decision::Allow => panic!("chain rooted at an arbitrary agent must deny"),
    }

    let orphan = AgentId::new();
    agent.delegation_chain.links = vec![
        link(human_root, orphan, None),
        link(AgentId::new(), agent.agent_id, None),
    ];
    match policy.check(&agent, "http.get") {
        Decision::Deny { reason } => assert!(
            reason.contains("incoherent"),
            "expected incoherent-chain denial, got: {reason}"
        ),
        Decision::Allow => panic!("incoherent chain must deny"),
    }

    agent.delegation_chain.links = vec![link(
        human_root,
        agent.agent_id,
        Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
    )];
    match policy.check(&agent, "http.get") {
        Decision::Deny { reason } => assert!(
            reason.contains("expired"),
            "expected expiry denial, got: {reason}"
        ),
        Decision::Allow => panic!("expired chain must deny"),
    }
}
