use std::collections::HashSet;
use chrono::{DateTime, Utc, Duration};

/// Capability: scoped, expiring, enforced by runtime (not LLM self-assertion).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Capability {
    pub name: String,
    pub scope: Scope,
    pub expiration: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Scope {
    #[default]
    None,
    Read,
    Write,
    All,
}

impl Capability {
    pub fn new(name: &str, scope: Scope, ttl_seconds: Option<i64>) -> Self {
        Self {
            name: name.to_string(),
            scope,
            expiration: ttl_seconds.map(|s| Utc::now() + Duration::seconds(s)),
        }
    }
    pub fn is_expired(&self) -> bool {
        self.expiration.map(|e| Utc::now() > e).unwrap_or(false)
    }
}

/// Set of capabilities for an agent.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct CapabilitySet {
    pub caps: Vec<Capability>,
}

impl CapabilitySet {
    pub fn new() -> Self { Self { caps: Vec::new() } }
    pub fn grant(&mut self, cap: Capability) { self.caps.push(cap); }
    pub fn has(&self, name: &str) -> bool {
        self.caps.iter().any(|c| c.name == name && !c.is_expired())
    }
}

/// Policy decision — explicit allow or deny with reason.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Decision {
    Allow,
    Deny { reason: String },
}

/// Policy engine: enforces capabilities independently of LLM reasoning.
#[derive(Debug)]
pub struct PolicyEngine {
    allow_list: HashSet<String>,
}

impl PolicyEngine {
    pub fn new() -> Self { Self { allow_list: HashSet::new() } }
    pub fn add_capability(&mut self, cap: &str) { self.allow_list.insert(cap.to_string()); }
    /// Check if the given agent is authorized for the given action.
    /// Consults allow_list AND validates delegation chain expiration.
    pub fn check(&self, agent: &runtime_auth::AgentIdentity, action: &str) -> Decision {
        // CF-2 FIX: traverse delegation chain for expiration + allow_list check
        for link in &agent.delegation_chain.links {
            // Check expiration on each delegation link
            if let Some(expires) = link.expires_at {
                if chrono::Utc::now() > expires {
                    return Decision::Deny {
                        reason: format!("delegation {:?} expired", link.from),
                    };
                }
            }
        }
        // Check allow_list for the action
        if self.allow_list.contains(action) {
            Decision::Allow
        } else {
            Decision::Deny {
                reason: format!("agent {:?} lacks capability: {}", agent.agent_id, action),
            }
        }
    }

    pub fn check_with_caps(&self, agent: &runtime_auth::AgentIdentity, caps: &CapabilitySet, action: &str) -> Decision {
        let base = self.check(agent, action);
        match base {
            Decision::Allow => {
                if caps.has(action) {
                    Decision::Allow
                } else {
                    Decision::Deny { reason: "capability missing in agent CapabilitySet".into() }
                }
            }
            other => other,
        }
    }
}

impl Default for PolicyEngine { fn default() -> Self { Self::new() } }

mod tests;
