//! Phase 1 §6 regression tests — PolicyEngine check_with_caps contract.
//!
//! These tests verify the CF-2 contract: check_with_caps allows ONLY when
//! both the policy allow_list AND the agent's CapabilitySet contain the action.
//! It MUST NOT rely on LLM self-assertion.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PolicyEngine, CapabilitySet, Capability, Scope, Decision};
    use runtime_auth::{AgentIdentity, HumanId};

    fn make_agent() -> AgentIdentity {
        AgentIdentity::new(HumanId::default())
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
}
