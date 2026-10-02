use std::collections::HashSet;
use std::sync::Arc;
use chrono::{DateTime, Utc, Duration};

/// Capability: scoped, expiring, enforced by runtime (not LLM self-assertion).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Capability {
    pub name: String,
    pub scope: Scope,
    pub expiration: Option<DateTime<Utc>>,
    /// Optional reference to the policy namespace under which this capability
    /// was granted (e.g. "http"). When present, the capability only
    /// authorizes actions covered by that namespace.
    #[serde(default)]
    pub policy_ref: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Scope {
    #[default]
    None,
    Read,
    Write,
    All,
}

impl Scope {
    /// Whether `self` is sufficient for an action that requires `required`.
    /// Ordering: `All` covers everything; `Write` covers `Read` and `Write`;
    /// `Read` covers only `Read`; `None` covers only `None`.
    pub fn covers(&self, required: &Scope) -> bool {
        match (self, required) {
            (Scope::All, _) => true,
            (Scope::Write, Scope::Write) | (Scope::Write, Scope::Read) => true,
            (Scope::Read, Scope::Read) => true,
            (Scope::None, Scope::None) => true,
            _ => false,
        }
    }
}

/// Explicit action → required-scope registry.
///
/// Write-class actions mutate state or carry authority: the mutating HTTP
/// verbs (`http.post`, `http.put`, `http.patch`, `http.delete`) and the
/// semantic authority-bearing actions `purchase`, `submit_form`,
/// `authenticate`, `mcp.invoke`, and `schedule`. Read-class actions only
/// observe: `http.get`, `search_web`, `extract_page`.
///
/// The registry is **default-deny**: an action that is not listed is
/// unclassified (`None`), and `check_with_caps` refuses to authorize it. An
/// unrecognized action is never treated as read-class, so a `Scope::Read`
/// capability can never silently authorize an unknown mutation.
fn required_scope(action: &str) -> Option<Scope> {
    match action {
        "http.get" | "search_web" | "extract_page" => Some(Scope::Read),
        "http.post" | "http.put" | "http.patch" | "http.delete"
        | "purchase" | "submit_form" | "authenticate" | "mcp.invoke" | "schedule" => {
            Some(Scope::Write)
        }
        _ => None,
    }
}

/// Whether `action` falls under the policy namespace `policy_ref`: the ref
/// matches the action itself or is a dot-delimited prefix of it.
fn policy_ref_covers(policy_ref: &str, action: &str) -> bool {
    action == policy_ref || action.starts_with(&format!("{}.", policy_ref))
}

impl Capability {
    pub fn new(name: &str, scope: Scope, ttl_seconds: Option<i64>) -> Self {
        Self {
            name: name.to_string(),
            scope,
            expiration: ttl_seconds.map(|s| Utc::now() + Duration::seconds(s)),
            policy_ref: None,
        }
    }

    /// Attach a policy namespace reference to this capability.
    pub fn with_policy_ref(mut self, policy_ref: impl Into<String>) -> Self {
        self.policy_ref = Some(policy_ref.into());
        self
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
    /// Scope-aware membership: the capability must exist, be unexpired, its
    /// scope must cover `required`, and — when it carries a `policy_ref` —
    /// the action must fall under that policy namespace.
    pub fn has_scoped(&self, name: &str, required: &Scope) -> bool {
        self.caps.iter().any(|c| {
            c.name == name
                && !c.is_expired()
                && c.scope.covers(required)
                && c.policy_ref.as_ref().map_or(true, |r| policy_ref_covers(r, name))
        })
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
    /// System-wide vocabulary of known capability names. This is the
    /// runtime's fixed capability vocabulary — an action not in this list
    /// is denied before per-agent CapabilitySets are consulted.
    allow_list: HashSet<String>,
    observability: Option<Arc<dyn runtime_observability::Observability>>,
}

impl PolicyEngine {
    pub fn new() -> Self { Self { allow_list: HashSet::new(), observability: None } }
    pub fn add_capability(&mut self, cap: &str) { self.allow_list.insert(cap.to_string()); }

    /// Inject an Observability sink: every Allow/Deny decision is then
    /// emitted as a `policy_decision` lifecycle event plus a
    /// `policy_decision` metric through the injected sink only.
    pub fn with_observability(mut self, obs: Arc<dyn runtime_observability::Observability>) -> Self {
        self.observability = Some(obs);
        self
    }

    /// Check if the given agent is authorized for the given action.
    /// Consults allow_list, validates delegation chain coherence + expiry,
    /// and logs the decision when an Observability sink is injected.
    pub fn check(&self, agent: &runtime_auth::AgentIdentity, action: &str) -> Decision {
        let decision = self.evaluate(agent, action);
        self.log_decision(agent, action, &decision);
        decision
    }

    fn evaluate(&self, agent: &runtime_auth::AgentIdentity, action: &str) -> Decision {
        if agent.human.is_nil() {
            return Decision::Deny {
                reason: "nil human authority: no verified human lineage".into(),
            };
        }
        if !agent.delegation_chain.is_coherent(&agent.agent_id, &agent.human) {
            return Decision::Deny {
                reason: "incoherent delegation chain".into(),
            };
        }
        if agent.delegation_chain.is_expired() {
            return Decision::Deny {
                reason: format!("delegation chain expired for agent {:?}", agent.agent_id),
            };
        }
        if self.allow_list.contains(action) {
            Decision::Allow
        } else {
            Decision::Deny {
                reason: format!("agent {:?} lacks capability: {}", agent.agent_id, action),
            }
        }
    }

    fn log_decision(&self, agent: &runtime_auth::AgentIdentity, action: &str, decision: &Decision) {
        if let Some(obs) = &self.observability {
            let (decision_str, reason) = match decision {
                Decision::Allow => ("allow", None),
                Decision::Deny { reason } => ("deny", Some(reason.as_str())),
            };
            let details = serde_json::json!({
                "action": action,
                "decision": decision_str,
                "reason": reason,
            });
            obs.record_lifecycle(runtime_observability::LifecycleEvent {
                task_id: uuid::Uuid::nil(),
                agent_id: agent.agent_id.0,
                delegation_id: None,
                event_type: "policy_decision".into(),
                timestamp: chrono::Utc::now(),
                details: Some(details),
            });
            // Persist the decision through the Observability's single replay
            // sequence (R5: no second sequence source). The `sequence` field
            // here is a placeholder — the sink assigns the real one.
            obs.record_replay(runtime_observability::ReplayEvent {
                sequence: 0,
                event_type: "policy_decision".into(),
                task_id: uuid::Uuid::nil(),
                agent_id: agent.agent_id.0,
                result_summary: match reason {
                    Some(reason) => format!("{}: {} ({})", decision_str, action, reason),
                    None => format!("{}: {}", decision_str, action),
                },
                timestamp: chrono::Utc::now(),
            });
            obs.metric("policy_decision", 1.0, &[("decision", decision_str)]);
        }
    }

    pub fn check_with_caps(&self, agent: &runtime_auth::AgentIdentity, caps: &CapabilitySet, action: &str) -> Decision {
        let base = self.check(agent, action);
        match base {
            Decision::Allow => {
                let decision = match required_scope(action) {
                    Some(required) if caps.has_scoped(action, &required) => Decision::Allow,
                    Some(_) => Decision::Deny {
                        reason: format!("capability missing in agent CapabilitySet for {}", action),
                    },
                    None => Decision::Deny {
                        reason: format!("unclassified action '{}': default-deny", action),
                    },
                };
                // The base allow was already logged by `check`; when the
                // per-agent capability scope check escalates to a denial,
                // emit that denial too so lifecycle/replay/metric reflect it.
                if matches!(decision, Decision::Deny { .. }) {
                    self.log_decision(agent, action, &decision);
                }
                decision
            }
            other => other,
        }
    }
}

impl Default for PolicyEngine { fn default() -> Self { Self::new() } }

mod tests;
