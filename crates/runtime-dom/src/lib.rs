use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum DomError {
    #[error("parse error: {0}")]
    ParseError(String),
    #[error("selector error: {0}")]
    SelectorError(String),
    #[error("mutation error: {0}")]
    MutationError(String),
}

#[derive(Debug)]
pub enum DomNode {
    Document {
        children: Vec<Arc<RwLock<DomNode>>>,
    },
    Element {
        tag: String,
        attrs: HashMap<String, String>,
        children: Vec<Arc<RwLock<DomNode>>>,
    },
    Text(String),
    Comment(String),
}

pub struct HtmlParser;

impl HtmlParser {
    pub fn parse(html: &str) -> Result<Arc<RwLock<DomNode>>, DomError> {
        let mut children: Vec<Arc<RwLock<DomNode>>> = Vec::new();
        let html = html.trim();
        let mut remaining = html.to_string();

        while !remaining.is_empty() {
            if let Some(start) = remaining.find('<') {
                let before = remaining[..start].trim();
                if !before.is_empty() {
                    children.push(Arc::new(RwLock::new(DomNode::Text(before.to_string()))));
                }
                remaining = remaining[start..].to_string();
                if remaining.starts_with("<!--") {
                    if let Some(end) = remaining.find("-->") {
                        let content = remaining[4..end].to_string();
                        children.push(Arc::new(RwLock::new(DomNode::Comment(content))));
                        remaining = remaining[end + 3..].to_string();
                    } else {
                        return Err(DomError::ParseError("unclosed comment".into()));
                    }
                } else if remaining.starts_with("</") {
                    if let Some(end) = remaining.find('>') {
                        remaining = remaining[end + 1..].to_string();
                    } else {
                        return Err(DomError::ParseError("unclosed closing tag".into()));
                    }
                } else if remaining.starts_with('<') {
                    if let Some(end) = remaining.find('>') {
                        let tag_content = remaining[1..end].to_string();
                        let tag_name = tag_content
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .to_string();
                        let attrs = Self::parse_attrs(&tag_content);
                        let node = Arc::new(RwLock::new(DomNode::Element {
                            tag: tag_name,
                            attrs,
                            children: Vec::new(),
                        }));
                        children.push(node);
                        remaining = remaining[end + 1..].to_string();
                    } else {
                        return Err(DomError::ParseError("unclosed tag".into()));
                    }
                } else {
                    return Err(DomError::ParseError("unexpected <".into()));
                }
            } else {
                children.push(Arc::new(RwLock::new(DomNode::Text(remaining.to_string()))));
                break;
            }
        }

        let root = Arc::new(RwLock::new(DomNode::Document { children }));
        Ok(root)
    }

    fn parse_attrs(tag_content: &str) -> HashMap<String, String> {
        let mut attrs = HashMap::new();
        let mut rest = tag_content.to_string();
        if let Some(pos) = rest.find(' ') {
            rest = rest[pos + 1..].to_string();
        } else {
            return attrs;
        }
        let chars: Vec<char> = rest.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            while i < chars.len() && chars[i].is_whitespace() { i += 1; }
            if i >= chars.len() { break; }
            let mut key = String::new();
            while i < chars.len() && chars[i] != '=' && !chars[i].is_whitespace() {
                key.push(chars[i]); i += 1;
            }
            if i < chars.len() && chars[i] == '=' {
                i += 1;
                let mut val = String::new();
                if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                    let quote = chars[i];
                    i += 1;
                    while i < chars.len() && chars[i] != quote {
                        val.push(chars[i]); i += 1;
                    }
                    if i < chars.len() && chars[i] == quote { i += 1; }
                } else {
                    while i < chars.len() && !chars[i].is_whitespace() {
                        val.push(chars[i]); i += 1;
                    }
                }
                attrs.insert(key, val);
            } else {
                attrs.insert(key, String::new());
            }
        }
        attrs
    }
}

#[derive(Debug)]
pub struct DomTree {
    pub root: Arc<RwLock<DomNode>>,
}

impl DomTree {
    pub fn new(root: Arc<RwLock<DomNode>>) -> Self {
        Self { root }
    }

    pub fn query(&self, selector: &str) -> Option<Arc<RwLock<DomNode>>> {
        self.query_all(selector).into_iter().next()
    }

    pub fn query_all(&self, selector: &str) -> Vec<Arc<RwLock<DomNode>>> {
        let mut results = Vec::new();
        self.collect_matches(&self.root, selector, &mut results);
        results
    }

    fn collect_matches(
        &self,
        node: &Arc<RwLock<DomNode>>,
        selector: &str,
        results: &mut Vec<Arc<RwLock<DomNode>>>,
    ) {
        let node_ref = node.read().unwrap();
        if Self::matches_selector(&node_ref, selector) {
            results.push(node.clone());
        }
        let children: Vec<Arc<RwLock<DomNode>>> = match &*node_ref {
            DomNode::Document { children } | DomNode::Element { children, .. } => {
                children.clone()
            }
            _ => Vec::new(),
        };
        drop(node_ref);
        for child in children {
            self.collect_matches(&child, selector, results);
        }
    }

    fn matches_selector(node: &DomNode, selector: &str) -> bool {
        match node {
            DomNode::Element { tag, attrs, .. } => {
                if selector == tag {
                    return true;
                }
                if let Some(id) = selector.strip_prefix('#') {
                    return attrs.get("id").map_or(false, |v| v == id);
                }
                if let Some(cls) = selector.strip_prefix('.') {
                    return attrs.get("class").map_or(false, |v| {
                        v.split_whitespace().any(|c| c == cls)
                    });
                }
                false
            }
            _ => false,
        }
    }

    pub fn append_child(&self, parent: &Arc<RwLock<DomNode>>, child: Arc<RwLock<DomNode>>) {
        let mut p = parent.write().unwrap();
        match &mut *p {
            DomNode::Document { children } | DomNode::Element { children, .. } => {
                children.push(child);
            }
            _ => {}
        }
    }

    pub fn remove_child(&self, parent: &Arc<RwLock<DomNode>>, child: &Arc<RwLock<DomNode>>) {
        let mut p = parent.write().unwrap();
        match &mut *p {
            DomNode::Document { children } | DomNode::Element { children, .. } => {
                children.retain(|c| !Arc::ptr_eq(c, child));
            }
            _ => {}
        }
    }

    pub fn set_text(&self, node: &Arc<RwLock<DomNode>>, text: &str) {
        let mut n = node.write().unwrap();
        *n = DomNode::Text(text.to_string());
    }
}

pub struct EventEmitter {
    listeners: Arc<RwLock<HashMap<String, Vec<Box<dyn Fn(&str) + Send + Sync>>>>>,
}

impl EventEmitter {
    pub fn new() -> Self {
        Self {
            listeners: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn on<F>(&self, event: &str, callback: F)
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        let mut map = self.listeners.write().unwrap();
        map.entry(event.to_string())
            .or_default()
            .push(Box::new(callback));
    }

    pub fn emit(&self, event: &str, data: &str) {
        let map = self.listeners.read().unwrap();
        if let Some(cbs) = map.get(event) {
            for cb in cbs {
                cb(data);
            }
        }
    }
}

impl Default for EventEmitter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;

    #[test]
    fn test_parse_simple_tag() {
        let html = "<div id=\"main\" class=\"box\"></div>";
        let root = HtmlParser::parse(html).unwrap();
        let tree = DomTree::new(root);
        let found = tree.query("div");
        assert!(found.is_some());
    }

    #[test]
    fn test_parse_text_and_comment() {
        let html = "Hello <!-- note -->";
        let root = HtmlParser::parse(html).unwrap();
        let tree = DomTree::new(root);
        assert!(tree.query_all("div").is_empty());
    }

    #[test]
    fn test_selector_by_id() {
        let html = "<span id=\"x\"></span>";
        let root = HtmlParser::parse(html).unwrap();
        let tree = DomTree::new(root);
        assert!(tree.query("#x").is_some());
    }

    #[test]
    fn test_selector_by_class() {
        let html = "<p class=\"foo bar\"></p>";
        let root = HtmlParser::parse(html).unwrap();
        let tree = DomTree::new(root);
        assert!(tree.query(".foo").is_some());
        assert!(tree.query(".bar").is_some());
        assert!(tree.query(".baz").is_none());
    }

    #[test]
    fn test_event_emitter() {
        let emitter = EventEmitter::new();
        let called = Arc::new(AtomicBool::new(false));
        let c = called.clone();
        emitter.on("click", move |_| {
            c.store(true, Ordering::SeqCst);
        });
        emitter.emit("click", "{}");
        assert!(called.load(Ordering::SeqCst));
    }
}
pub mod adapter;
