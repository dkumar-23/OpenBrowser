//! G25 — Semantic Action Scope Classification (Priority P0)
//!
//! Observable: write-class semantic actions (`purchase`, `submit_form`,
//! `authenticate`, `mcp.invoke`, `schedule`, and the mutating HTTP verbs)
//! require Write-or-better capabilities; `Scope::Read` never authorizes them.
//! Read-class actions (`http.get`, `search_web`, `extract_page`) accept a
//! `Scope::Read` capability. An allow-listed but unclassified action is
//! default-denied.

use runtime_auth::{AgentIdentity, HumanId};
use runtime_policy::{Capability, CapabilitySet, Decision, PolicyEngine, Scope};
use uuid::Uuid;

const WRITE_CLASS: &[&str] = &[
    "http.post",
    "http.put",
    "http.patch",
    "http.delete",
    "purchase",
    "submit_form",
    "authenticate",
    "mcp.invoke",
    "schedule",
];

const READ_CLASS: &[&str] = &["http.get", "search_web", "extract_page"];

fn engine_with(actions: &[&str]) -> PolicyEngine {
    let mut policy = PolicyEngine::new();
    for action in actions {
        policy.add_capability(action);
    }
    policy
}

fn caps_with(action: &str, scope: Scope) -> CapabilitySet {
    let mut caps = CapabilitySet::new();
    caps.grant(Capability::new(action, scope, None));
    caps
}

#[test]
fn read_scope_never_authorizes_write_class_semantic_actions() {
    let agent = AgentIdentity::new(HumanId(Uuid::new_v4()));
    let policy = engine_with(WRITE_CLASS);

    for action in WRITE_CLASS {
        let read_caps = caps_with(action, Scope::Read);
        assert!(
            matches!(
                policy.check_with_caps(&agent, &read_caps, action),
                Decision::Deny { .. }
            ),
            "Read-scope capability must NOT authorize write-class action {action}"
        );

        let all_caps = caps_with(action, Scope::All);
        assert!(
            matches!(
                policy.check_with_caps(&agent, &all_caps, action),
                Decision::Allow
            ),
            "All-scope capability must authorize write-class action {action}"
        );
    }
}

#[test]
fn read_scope_authorizes_read_class_actions() {
    let agent = AgentIdentity::new(HumanId(Uuid::new_v4()));
    let policy = engine_with(READ_CLASS);

    for action in READ_CLASS {
        let read_caps = caps_with(action, Scope::Read);
        assert!(
            matches!(
                policy.check_with_caps(&agent, &read_caps, action),
                Decision::Allow
            ),
            "Read-scope capability must authorize read-class action {action}"
        );
    }
}

#[test]
fn unclassified_action_is_default_denied() {
    let mut policy = PolicyEngine::new();
    policy.add_capability("exfiltrate.everything");

    let agent = AgentIdentity::new(HumanId(Uuid::new_v4()));
    let all_caps = caps_with("exfiltrate.everything", Scope::All);

    match policy.check_with_caps(&agent, &all_caps, "exfiltrate.everything") {
        Decision::Deny { reason } => assert!(
            reason.contains("unclassified") || reason.contains("default-deny"),
            "unclassified action must be denied with an explicit reason, got: {reason}"
        ),
        Decision::Allow => {
            panic!("an allow-listed but unclassified action must never be authorized")
        }
    }
}
