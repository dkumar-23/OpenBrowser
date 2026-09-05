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

/// Select text content from DOM nodes matching a CSS selector (simple + compound subset).
/// Supported subset: tag, #id, .class, div.foo, div#main,
/// [attr], [attr=value], ancestor descendant, parent > child.
fn select_from_node(root: &Arc<RwLock<DomNode>>, selector: &str) -> Vec<String> {
    use crate::DomTree;
    let mut results = Vec::new();
    let root_guard = root.read().unwrap();
    // Compound selectors: div.foo, div#main
    if !selector.starts_with('#') && !selector.starts_with('.')
        && !selector.starts_with('[') && (selector.contains('#') || selector.contains('.')) {
        compound_collect(&root_guard, selector, &mut results);
    } else if selector.starts_with('[') {
        attr_collect(&root_guard, selector, &mut results);
    } else if selector.contains('>') {
        let parts: Vec<&str> = selector.split('>').map(|s| s.trim()).collect();
        if parts.len() == 2 {
            direct_child_collect(&root_guard, parts[0], parts[1], &mut results);
        }
    } else if selector.contains(' ') {
        let parts: Vec<&str> = selector.split_whitespace().collect();
        if parts.len() == 2 {
            descendant_collect(&root_guard, parts[0], parts[1], &mut results);
        }
    } else if let Some(id) = selector.strip_prefix('#') {
        collect_by_id(&root_guard, id, &mut results);
    } else if let Some(cls) = selector.strip_prefix('.') {
        collect_by_class(&root_guard, cls, &mut results);
    } else {
        collect_by_tag(&root_guard, selector, &mut results);
    }
    let _ = DomTree::matches_simple; // re-export test helper
    results
}

fn compound_collect(node: &DomNode, sel: &str, results: &mut Vec<String>) {
    use crate::DomTree;
    if DomTree::matches_compound(node, sel) {
        collect_all_text(node, results);
    }
    if let DomNode::Element { children, .. } = node {
        for child in children {
            compound_collect(&child.read().unwrap(), sel, results);
        }
    }
}

fn attr_collect(node: &DomNode, sel: &str, results: &mut Vec<String>) {
    use crate::DomTree;
    if DomTree::matches_simple(node, sel) {
        collect_all_text(node, results);
    }
    if let DomNode::Element { children, .. } = node {
        for child in children {
            attr_collect(&child.read().unwrap(), sel, results);
        }
    }
}

fn direct_child_collect(node: &DomNode, parent_sel: &str, child_sel: &str, results: &mut Vec<String>) {
    use crate::DomTree;
    if let DomNode::Element { children, .. } = node {
        if DomTree::matches_simple(node, parent_sel) {
            for child in children {
                let c = child.read().unwrap();
                if DomTree::matches_simple(&c, child_sel) {
                    collect_all_text(&c, results);
                }
            }
        }
        for child in children {
            direct_child_collect(&child.read().unwrap(), parent_sel, child_sel, results);
        }
    }
}

fn descendant_collect(node: &DomNode, ancestor: &str, descendant: &str, results: &mut Vec<String>) {
    use crate::DomTree;
    if let DomNode::Element { children, .. } = node {
        if DomTree::matches_simple(node, ancestor) {
            for child in children {
                collect_descendants_matching(&child.read().unwrap(), descendant, results);
            }
        }
        for child in children {
            descendant_collect(&child.read().unwrap(), ancestor, descendant, results);
        }
    }
}

fn collect_descendants_matching(node: &DomNode, sel: &str, results: &mut Vec<String>) {
    use crate::DomTree;
    if DomTree::matches_simple(node, sel) {
        collect_all_text(node, results);
    }
    if let DomNode::Element { children, .. } = node {
        for child in children {
            collect_descendants_matching(&child.read().unwrap(), sel, results);
        }
    }
}

fn collect_by_id(node: &DomNode, id: &str, results: &mut Vec<String>) {
    if let DomNode::Element { attrs, children, .. } = node {
        if let Some(val) = attrs.get("id") {
            if val == id {
                collect_all_text(node, results);
            }
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

    fn matches_simple(node: &DomNode, sel: &str) -> bool {
        if let DomNode::Element { tag, attrs, .. } = node {
            if let Some(id) = sel.strip_prefix('#') {
                return attrs.get("id").map(|s| s == id).unwrap_or(false);
            }
            if let Some(cls) = sel.strip_prefix('.') {
                return attrs.get("class").map(|c| c.split_whitespace().any(|x| x == cls)).unwrap_or(false);
            }
            if sel.starts_with('[') {
                // [attr] or [attr=value]
                let inner = &sel[1..sel.len()-1];
                if let Some(eq_pos) = inner.find('=') {
                    let key = &inner[..eq_pos];
                    let val = inner[eq_pos+1..].trim_matches('"');
                    return attrs.get(key).map(|s| s == val).unwrap_or(false);
                } else {
                    return attrs.contains_key(inner);
                }
            }
            return tag == sel;
        }
        false
    }

    fn matches_compound(node: &DomNode, sel: &str) -> bool {
        // div.foo, div#main
        if let Some(pos) = sel.find(|c: char| c == '#' || c == '.') {
            let tag = &sel[..pos];
            let rest = &sel[pos..];
            if let DomNode::Element { tag: node_tag, attrs, .. } = node {
                if node_tag != tag { return false; }
                if let Some(id) = rest.strip_prefix('#') {
                    return attrs.get("id").map(|s| s == id).unwrap_or(false);
                }
                if let Some(cls) = rest.strip_prefix('.') {
                    return attrs.get("class").map(|c| c.split_whitespace().any(|x| x == cls)).unwrap_or(false);
                }
            }
        }
        false
    }

    fn compound_selection(node: &DomNode, sel: &str, results: &mut Vec<String>) {
        if matches_compound(node, sel) || matches_simple(node, sel) {
            collect_all_text(node, results);
        }
        if let DomNode::Element { children, .. } = node {
            for child in children {
                compound_selection(&child.read().unwrap(), sel, results);
            }
        }
    }

    fn descendant_selection(node: &DomNode, ancestor: &str, descendant: &str, results: &mut Vec<String>) {
        if let DomNode::Element { children, .. } = node {
            if matches_simple(node, ancestor) {
                for child in children {
                    collect_descendants_matching(&child.read().unwrap(), descendant, results);
                }
            }
            for child in children {
                descendant_selection(&child.read().unwrap(), ancestor, descendant, results);
            }
        }
    }

    fn collect_descendants_matching(node: &DomNode, sel: &str, results: &mut Vec<String>) {
        if matches_simple(node, sel) {
            collect_all_text(node, results);
        }
        if let DomNode::Element { children, .. } = node {
            for child in children {
                collect_descendants_matching(&child.read().unwrap(), sel, results);
            }
        }
    }

    fn direct_child_selection(node: &DomNode, parent_sel: &str, child_sel: &str, results: &mut Vec<String>) {
        if let DomNode::Element { children, .. } = node {
            if matches_simple(node, parent_sel) {
                for child in children {
                    let c = child.read().unwrap();
                    if matches_simple(&c, child_sel) {
                        collect_all_text(&c, results);
                    }
                }
            }
            for child in children {
                direct_child_selection(&child.read().unwrap(), parent_sel, child_sel, results);
            }
        }
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
