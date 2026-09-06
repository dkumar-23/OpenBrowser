// runtime-dom/src/adapter.rs: DOM adapter implementing InteractionAdapter (Phase 3.1)
//
// Per context.md §11: agent should not need to understand mechanism.
// This adapter provides HTML parsing + DOM selection through the unified interaction API.

use async_trait::async_trait;
use runtime_interaction::{
    InteractionAdapter, AdapterDescriptor, AdapterKind, AdapterParams, AdapterResult, TaskInfo,
};
use runtime_auth::AgentIdentity;
use runtime_policy::{PolicyEngine, CapabilitySet};
use runtime_observability::{Observability, ReplayEvent};
use std::sync::{Arc, RwLock};
use serde_json;

use crate::DomTree;

use crate::{HtmlParser, DomNode};

/// DOM adapter — parses HTML and selects elements via the InteractionAdapter trait.
#[derive(Debug)]
pub struct DomAdapter {
    observability: Arc<dyn Observability>,
    policy: Arc<PolicyEngine>,
}

impl DomAdapter {
    pub fn new(observability: Arc<dyn Observability>, policy: Arc<PolicyEngine>) -> Self {
        Self { observability, policy }
    }
}

#[async_trait]
impl InteractionAdapter for DomAdapter {
    fn descriptor(&self) -> AdapterDescriptor {
        AdapterDescriptor {
            kind: AdapterKind::Dom,
            handles: vec![
                "extract_page".into(),
                "dom.parse".into(),
                "dom.select".into(),
            ],
        }
    }

    async fn execute(
        &self,
        agent: &AgentIdentity,
        caps: &CapabilitySet,
        info: &TaskInfo,
        params: &AdapterParams,
    ) -> AdapterResult {
        let (html, selector) = match params {
            AdapterParams::Dom { html, selector } => (html.clone(), selector.clone()),
            _ => return AdapterResult::Error {
                message: format!("DomAdapter expects AdapterParams::Dom, got {:?}", params),
                replay_sequence: 0,
            },
        };

        // Policy enforcement (CF-1 + CF-2)
        let decision = self.policy.check_with_caps(agent, caps, "extract_page");
        match decision {
            runtime_policy::Decision::Deny { reason } => {
                let event = ReplayEvent {
                    sequence: 0,
                    event_type: "capability_denied".into(),
                    task_id: info.task_id,
                    agent_id: agent.agent_id.0,
                    result_summary: reason.clone(),
                    timestamp: chrono::Utc::now(),
                };
                let seq = self.observability.record_replay(event);
                self.observability.metric("dom_policy_denied", 1.0, &[("capability", "extract_page")]);
                AdapterResult::Denied { reason, replay_sequence: seq }
            }
            runtime_policy::Decision::Allow => {
                // Parse HTML and select
                match HtmlParser::parse(&html) {
                    Ok(root) => {
                        let selected = select_from_node(&root, &selector);
                        let json_result = serde_json::json!({
                            "parsed": true,
                            "selector": selector,
                            "elements_found": selected.len(),
                            "sample": selected.iter().take(5).cloned().collect::<Vec<_>>(),
                        });
                        let event = ReplayEvent {
                            sequence: 0,
                            event_type: "dom_parsed".into(),
                            task_id: info.task_id,
                            agent_id: agent.agent_id.0,
                            result_summary: format!("{} elements selected", selected.len()),
                            timestamp: chrono::Utc::now(),
                        };
                        let seq = self.observability.record_replay(event);
                        self.observability.metric("dom_parsed", 1.0, &[("selector", &selector)]);
                        AdapterResult::Success {
                            response: serde_json::to_string(&json_result).unwrap_or_default(),
                            replay_sequence: seq,
                        }
                    }
                    Err(parse_err) => {
                        let msg = format!("parse error: {}", parse_err);
                        let event = ReplayEvent {
                            sequence: 0,
                            event_type: "dom_error".into(),
                            task_id: info.task_id,
                            agent_id: agent.agent_id.0,
                            result_summary: msg.clone(),
                            timestamp: chrono::Utc::now(),
                        };
                        let seq = self.observability.record_replay(event);
                        self.observability.metric("dom_error", 1.0, &[]);
                        AdapterResult::Error { message: msg, replay_sequence: seq }
                    }
                }
            }
        }
    }
}

/// Select text content from DOM nodes matching a CSS selector.
/// Delegates matching to the single selector engine in `DomTree::query_all`.
/// Supported subset: tag, #id, .class, compound (div.foo, div#main),
/// [attr], [attr=value], "ancestor descendant", "parent > child".
fn select_from_node(root: &Arc<RwLock<DomNode>>, selector: &str) -> Vec<String> {
    let mut results = Vec::new();
    for node in DomTree::new(Arc::clone(root)).query_all(selector) {
        collect_all_text(&node.read().unwrap(), &mut results);
    }
    results
}
fn collect_all_text(node: &DomNode, results: &mut Vec<String>) {
    match node {
        DomNode::Text { content, .. } => {
            let t = content.trim();
            if !t.is_empty() { results.push(t.to_string()); }
        }
        DomNode::Element { children, .. } => {
            for child in children {
                collect_all_text(&child.read().unwrap(), results);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_auth::{AgentIdentity, HumanId};
    use runtime_policy::CapabilitySet;

    fn make_identity() -> AgentIdentity {
        AgentIdentity::new(HumanId(uuid::Uuid::new_v4()))
    }

    #[test]
    fn test_selector_compound() {
        let html = "<html><body><div id='main' class='x'><p class='x'>Hello</p></div></body></html>";
        let root = HtmlParser::parse(html).unwrap();
        assert!(!select_from_node(&root, "div#main").is_empty());
        assert!(!select_from_node(&root, "div.x").is_empty());
        assert!(!select_from_node(&root, "p.x").is_empty());
    }

    #[test]
    fn test_selector_attribute() {
        let html = r#"<html><body><div data-id="42"><span>x</span></div></body></html>"#;
        let root = HtmlParser::parse(html).unwrap();
        assert!(!select_from_node(&root, "[data-id]").is_empty());
        assert!(!select_from_node(&root, r#"[data-id="42"]"#).is_empty());
    }

    #[test]
    fn test_selector_descendant() {
        let html = "<html><body><div><span>Hello</span></div></body></html>";
        let root = HtmlParser::parse(html).unwrap();
        assert!(!select_from_node(&root, "div span").is_empty());
    }

    #[test]
    fn test_selector_child() {
        let html = "<html><body><div><span>Direct</span></div></body></html>";
        let root = HtmlParser::parse(html).unwrap();
        assert!(!select_from_node(&root, "div > span").is_empty());
    }

    #[test]
    fn test_selector_attr_value_edge_cases() {
        // Attribute values containing selector-significant characters.
        let html = r#"<html><body><div data-x="a.b">dot</div><div data-y="a b">space</div></body></html>"#;
        let root = HtmlParser::parse(html).unwrap();
        // Value with a dot: pinned — must match exactly, not be split.
        assert_eq!(select_from_node(&root, r#"[data-x="a.b"]"#), vec!["dot".to_string()]);
        // Whitespace inside the brackets around attr/value.
        assert_eq!(select_from_node(&root, r#"[ data-x = "a.b" ]"#), vec!["dot".to_string()]);
    }

    #[test]
    fn test_append_child_rejects_cycles() {
        // A malformed structure (cycle via append_child) must never be
        // created: append_child rejects appends that would make a node's
        // parent point into its own subtree.
        let html = "<html><body><div id='a'><span id='b'>x</span></div></body></html>";
        let root = HtmlParser::parse(html).unwrap();
        let tree = DomTree::new(Arc::clone(&root));
        let div = tree.query("div").unwrap();
        let span = tree.query("span").unwrap();
        // Rejected: div is an ancestor of span, so appending it under span
        // would create a children/parent cycle.
        assert!(!tree.append_child(&span, Arc::clone(&div)), "cycle append must be rejected");
        // Rejected: appending a node under itself.
        assert!(!tree.append_child(&div, Arc::clone(&div)), "self append must be rejected");
        // Tree remains well-formed and queryable.
        assert_eq!(tree.query_all("body span").len(), 1);
    }

    #[tokio::test]
    async fn test_dom_adapter_policy_denied() {
        let agent = make_identity();
        let info = runtime_interaction::TaskInfo::new(uuid::Uuid::new_v4(), agent.agent_id.0);
        let caps = CapabilitySet::new();
        let obs = Arc::new(runtime_observability::TraceObservability::without_replay());
        let policy = Arc::new({
            let mut p = runtime_policy::PolicyEngine::new();
            p.add_capability("extract_page");
            p
        });
        let adapter = DomAdapter::new(obs, policy);
        let params = AdapterParams::Dom {
            html: "<html><body><p>Hello</p></body></html>".into(),
            selector: "p".into(),
        };
        let result = adapter.execute(&agent, &caps, &info, &params).await;
        assert!(matches!(result, AdapterResult::Denied { .. }));
    }

    #[tokio::test]
    async fn test_dom_adapter_policy_allowed() {
        let agent = make_identity();
        let info = runtime_interaction::TaskInfo::new(uuid::Uuid::new_v4(), agent.agent_id.0);
        let mut caps = CapabilitySet::new();
        caps.grant(runtime_policy::Capability::new("extract_page", runtime_policy::Scope::Read, None));
        let obs = Arc::new(runtime_observability::TraceObservability::without_replay());
        let policy = Arc::new({
            let mut p = runtime_policy::PolicyEngine::new();
            p.add_capability("extract_page");
            p
        });
        let adapter = DomAdapter::new(obs, policy);
        let params = AdapterParams::Dom {
            html: "<html><body><p id='main'>Hello World</p></body></html>".into(),
            selector: "#main".into(),
        };
        let result = adapter.execute(&agent, &caps, &info, &params).await;
        assert!(result.is_success());
    }

    #[tokio::test]
    async fn test_dom_adapter_unexpected_params() {
        let agent = make_identity();
        let info = runtime_interaction::TaskInfo::new(uuid::Uuid::new_v4(), agent.agent_id.0);
        let caps = CapabilitySet::new();
        let obs = Arc::new(runtime_observability::TraceObservability::without_replay());
        let policy = Arc::new(runtime_policy::PolicyEngine::new());
        let adapter = DomAdapter::new(obs, policy);
        let params = AdapterParams::Http {  url: "https://example.com".into(), method: None, body: None, headers: Default::default() };
        let result = adapter.execute(&agent, &caps, &info, &params).await;
        assert!(matches!(result, AdapterResult::Error { .. }));
    }

    #[test]
    fn test_dom_adapter_descriptor() {
        let obs = Arc::new(runtime_observability::TraceObservability::without_replay());
        let policy = Arc::new(runtime_policy::PolicyEngine::new());
        let adapter = DomAdapter::new(obs, policy);
        let desc = adapter.descriptor();
        assert_eq!(desc.kind, AdapterKind::Dom);
        assert!(adapter.handles("extract_page"));
        assert!(adapter.handles("dom.parse"));
        assert!(!adapter.handles("http.get"));
    }
}
