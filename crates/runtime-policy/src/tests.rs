//! Phase 1 §6 regression tests — PolicyEngine check_with_caps contract.
//!
//! These tests verify the CF-2 contract: check_with_caps allows ONLY when
//! both the policy allow_list AND the agent's CapabilitySet contain the action.
//! It MUST NOT rely on LLM self-assertion.

#[cfg(test)]
mod tests {
    
    use crate::{PolicyEngine, CapabilitySet, Capability, Scope, Decision};
    use runtime_auth::{AgentIdentity, HumanId};

    fn make_agent() -> AgentIdentity {
        AgentIdentity::new(HumanId::new())
    }

    #[test]
    fn test_nil_human_is_denied() {
        let mut policy = PolicyEngine::new();
        policy.add_capability("http.get");

        let agent = AgentIdentity::new(HumanId::nil());
        let mut caps = CapabilitySet::new();
        caps.grant(Capability::new("http.get", Scope::All, None));

        match policy.check_with_caps(&agent, &caps, "http.get") {
            Decision::Deny { reason } => {
                assert!(reason.contains("human"), "expected nil-human denial, got: {reason}")
            }
            Decision::Allow => panic!("nil-human identity must be denied"),
        }
    }


    // -------------------------------------------------------------------------
    // §6 Test 1: check_with_caps ALLOWS when CapabilitySet has the action
    // -------------------------------------------------------------------------
    #[test]
    fn test_check_with_caps_allows_when_present() {
        let mut policy = PolicyEngine::new();
        // Register the capability in the policy allow_list
        policy.add_capability("http.get");

        let agent = make_agent();
        let mut caps = CapabilitySet::new();
        caps.grant(Capability::new("http.get", Scope::All, None));

        let decision = policy.check_with_caps(&agent, &caps, "http.get");

        match decision {
            Decision::Allow => {}
            Decision::Deny { reason } => {
                panic!(
                    "agent WITH CapabilitySet('http.get') AND allow_list entry should ALLOW, \
                     got Denied: {reason}"
                );
            }
        }
    }

    // -------------------------------------------------------------------------
    // §6 Test 2: check_with_caps DENIES when CapabilitySet is missing the action
    // -------------------------------------------------------------------------
    #[test]
    fn test_check_with_caps_denies_when_missing() {
        let mut policy = PolicyEngine::new();
        // Register the capability in the policy allow_list
        policy.add_capability("http.get");

        let agent = make_agent();
        let caps = CapabilitySet::new(); // EMPTY — no capabilities

        let decision = policy.check_with_caps(&agent, &caps, "http.get");

        let denied = match decision {
            Decision::Deny { reason } => {
                // Reason must reference missing capability
                assert!(
                    reason.contains("http.get") || reason.contains("capability"),
                    "denial reason should mention the missing action, got: {reason}"
                );
                true
            }
            Decision::Allow => {
                panic!(
                    "agent WITHOUT CapabilitySet('http.get') must be DENIED even if \
                     allow_list contains 'http.get'"
                );
            }
        };
        assert!(denied, "expected Deny variant");
    }

    // -------------------------------------------------------------------------
    // Phase 4: scope enforcement in check_with_caps
    // -------------------------------------------------------------------------
    #[test]
    fn test_read_scope_cannot_authorize_write_class_action() {
        let mut policy = PolicyEngine::new();
        policy.add_capability("http.post");

        let agent = make_agent();
        let mut caps = CapabilitySet::new();
        caps.grant(Capability::new("http.post", Scope::Read, None));

        match policy.check_with_caps(&agent, &caps, "http.post") {
            Decision::Deny { reason } => {
                assert!(
                    reason.contains("capability"),
                    "read-scope cap on write-class action must deny, got: {reason}"
                );
            }
            Decision::Allow => panic!("Read-scope capability must NOT authorize a write-class action"),
        }
    }

    #[test]
    fn test_write_scope_authorizes_read_class_action() {
        let mut policy = PolicyEngine::new();
        policy.add_capability("http.get");

        let agent = make_agent();
        let mut caps = CapabilitySet::new();
        caps.grant(Capability::new("http.get", Scope::Write, None));

        assert!(matches!(
            policy.check_with_caps(&agent, &caps, "http.get"),
            Decision::Allow
        ));
    }

    #[test]
    fn test_all_scope_covers_write_class_action() {
        let mut policy = PolicyEngine::new();
        policy.add_capability("http.post");

        let agent = make_agent();
        let mut caps = CapabilitySet::new();
        caps.grant(Capability::new("http.post", Scope::All, None));

        assert!(matches!(
            policy.check_with_caps(&agent, &caps, "http.post"),
            Decision::Allow
        ));
    }

    // -------------------------------------------------------------------------
    // Phase 4: policy_ref cross-check
    // -------------------------------------------------------------------------
    #[test]
    fn test_policy_ref_covering_namespace_allows() {
        let mut policy = PolicyEngine::new();
        policy.add_capability("http.get");

        let agent = make_agent();
        let mut caps = CapabilitySet::new();
        caps.grant(Capability::new("http.get", Scope::All, None).with_policy_ref("http"));

        assert!(matches!(
            policy.check_with_caps(&agent, &caps, "http.get"),
            Decision::Allow
        ));
    }

    #[test]
    fn test_policy_ref_mismatching_namespace_denies() {
        let mut policy = PolicyEngine::new();
        policy.add_capability("http.get");

        let agent = make_agent();
        let mut caps = CapabilitySet::new();
        caps.grant(Capability::new("http.get", Scope::All, None).with_policy_ref("dom"));

        assert!(matches!(
            policy.check_with_caps(&agent, &caps, "http.get"),
            Decision::Deny { .. }
        ));
    }

    // -------------------------------------------------------------------------
    // Phase 4: delegation chain coherence + expiry in check()
    // -------------------------------------------------------------------------
    #[test]
    fn test_incoherent_delegation_chain_denies() {
        let mut policy = PolicyEngine::new();
        policy.add_capability("http.get");

        let mut agent = make_agent();
        agent.delegation_chain.links = vec![
            runtime_auth::DelegationLink {
                from: runtime_auth::AgentId::new(),
                to: runtime_auth::AgentId::new(),
                granted_at: chrono::Utc::now(),
                expires_at: None,
            },
            runtime_auth::DelegationLink {
                from: runtime_auth::AgentId::new(),
                to: agent.agent_id,
                granted_at: chrono::Utc::now(),
                expires_at: None,
            },
        ];

        match policy.check(&agent, "http.get") {
            Decision::Deny { reason } => {
                assert!(reason.contains("incoherent"), "expected incoherent-chain denial, got: {reason}");
            }
            Decision::Allow => panic!("broken delegation chain must be denied"),
        }
    }

    #[test]
    fn test_expired_delegation_chain_denies() {
        let mut policy = PolicyEngine::new();
        policy.add_capability("http.get");

        let mut agent = make_agent();
        agent.delegation_chain.links = vec![runtime_auth::DelegationLink {
            from: runtime_auth::AgentId(agent.human.0),
            to: agent.agent_id,
            granted_at: chrono::Utc::now(),
            expires_at: Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
        }];

        match policy.check(&agent, "http.get") {
            Decision::Deny { reason } => {
                assert!(reason.contains("expired"), "expected expiry denial, got: {reason}");
            }
            Decision::Allow => panic!("expired delegation chain must be denied"),
        }
    }

    #[test]
    fn test_coherent_chain_allows() {
        let mut policy = PolicyEngine::new();
        policy.add_capability("http.get");

        let mut agent = make_agent();
        let root = runtime_auth::AgentId(agent.human.0);
        agent.delegation_chain.links = vec![runtime_auth::DelegationLink {
            from: root,
            to: agent.agent_id,
            granted_at: chrono::Utc::now(),
            expires_at: Some(chrono::Utc::now() + chrono::Duration::seconds(3600)),
        }];

        assert!(matches!(policy.check(&agent, "http.get"), Decision::Allow));
    }
}
