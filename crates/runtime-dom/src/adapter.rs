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

/// Select text content from DOM nodes matching a CSS selector (simple support).
/// Supports: tagname, #id, .class (multi-class via whitespace split).
fn select_from_node(root: &Arc<RwLock<DomNode>>, selector: &str) -> Vec<String> {
    let mut results = Vec::new();
    let root_guard = root.read().unwrap();
    if let Some(id) = selector.strip_prefix('#') {
        collect_by_id(&root_guard, id, &mut results);
    } else if let Some(cls) = selector.strip_prefix('.') {
        collect_by_class(&root_guard, cls, &mut results);
    } else {
        collect_by_tag(&root_guard, selector, &mut results);
    }
    results
}

fn collect_by_id(node: &DomNode, id: &str, results: &mut Vec<String>) {
    if let DomNode::Element { attrs, children, .. } = node {
        if attrs.get("id").map(|s| s.as_str()) == Some(id) {
            collect_all_text(node, results);
        }
    }
    if let DomNode::Element { children, .. } = node {
        for child in children {
            collect_by_id(&child.read().unwrap(), id, results);
        }
    }
}

fn collect_by_class(node: &DomNode, class: &str, results: &mut Vec<String>) {
    if let DomNode::Element { attrs, children, .. } = node {
        if attrs.get("class").map(|c| c.split_whitespace().any(|s| s == class)).unwrap_or(false) {
            collect_all_text(node, results);
        }
    }
    if let DomNode::Element { children, .. } = node {
        for child in children {
            collect_by_class(&child.read().unwrap(), class, results);
        }
    }
}

fn collect_by_tag(node: &DomNode, tag: &str, results: &mut Vec<String>) {
    if let DomNode::Element { tag: node_tag, children, .. } = node {
        if node_tag.eq_ignore_ascii_case(tag) {
            collect_all_text(node, results);
        }
    }
    if let DomNode::Element { children, .. } = node {
        for child in children {
            collect_by_tag(&child.read().unwrap(), tag, results);
        }
    }
}

fn collect_all_text(node: &DomNode, results: &mut Vec<String>) {
    match node {
        DomNode::Text(text) => {
            let t = text.trim();
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
