use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use thiserror::Error;

use html5ever::driver::ParseOpts;
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::interface::{Attribute, ExpandedName, QualName};
use std::cell::OnceCell;

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
        parent: Option<Arc<RwLock<DomNode>>>,
    },
    DocumentType {
        name: String,
        public_id: String,
        system_id: String,
        parent: Option<Arc<RwLock<DomNode>>>,
    },
    Element {
        tag: String,
        attrs: HashMap<String, String>,
        children: Vec<Arc<RwLock<DomNode>>>,
        parent: Option<Arc<RwLock<DomNode>>>,
    },
    Text {
        content: String,
        parent: Option<Arc<RwLock<DomNode>>>,
    },
    Comment {
        content: String,
        parent: Option<Arc<RwLock<DomNode>>>,
    },
}

// Internal node representation used while building the tree via html5ever.
#[derive(Debug, Clone)]
enum InternalNode {
    Document,
    Element {
        tag: String,
        attrs: HashMap<String, String>,
    },
    Text(String),
    Comment(String),
    Doctype {
        name: String,
        public_id: String,
        system_id: String,
    },
    ProcessingInstruction {
        target: String,
        data: String,
    },
}

#[derive(Debug, Clone)]
struct InternalHandle {
    node: Rc<RefCell<InternalNode>>,
    children: Rc<RefCell<Vec<InternalHandle>>>,
    parent: Rc<RefCell<Option<InternalHandle>>>,
    // Cached QualName for elements, needed by TreeSink::elem_name.
    // Stored in OnceCell so we can return a reference with 'self lifetime.
    qual_name: Rc<OnceCell<QualName>>,
}

impl InternalHandle {
    fn new(node: InternalNode) -> Self {
        Self {
            node: Rc::new(RefCell::new(node)),
            children: Rc::new(RefCell::new(Vec::new())),
            parent: Rc::new(RefCell::new(None)),
            qual_name: Rc::new(OnceCell::new()),
        }
    }

    fn get_qual_name(&self) -> &QualName {
        self.qual_name.get_or_init(|| {
            let node = self.node.borrow();
            match &*node {
                InternalNode::Element { tag, .. } => {
                    QualName::new(None, "http://www.w3.org/1999/xhtml".into(), tag.clone().into())
                }
                _ => QualName::new(None, "http://www.w3.org/1999/xhtml".into(), "".into()),
            }
        })
    }
}

struct DomSink {
    document: InternalHandle,
    errors: Vec<String>,
}

impl DomSink {
    fn new() -> Self {
        Self {
            document: InternalHandle::new(InternalNode::Document),
            errors: Vec::new(),
        }
    }
}

fn merge_text_into_last(parent: &InternalHandle, text: StrTendril) {
    let parent_children = parent.children.borrow();
    if let Some(last) = parent_children.last() {
        let mut last_node = last.node.borrow_mut();
        if let InternalNode::Text(existing) = &mut *last_node {
            existing.push_str(&text);
            return;
        }
    }
    drop(parent_children);
    let text_handle = InternalHandle::new(InternalNode::Text(text.to_string()));
    *text_handle.parent.borrow_mut() = Some(parent.clone());
    parent.children.borrow_mut().push(text_handle);
}

impl TreeSink for DomSink {
    type Handle = InternalHandle;
    type Output = Self;

    fn finish(self) -> Self {
        self
    }

    fn parse_error(&mut self, msg: std::borrow::Cow<'static, str>) {
        self.errors.push(msg.to_string());
    }

    fn get_document(&mut self) -> Self::Handle {
        self.document.clone()
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> ExpandedName<'a> {
        target.get_qual_name().expanded()
    }

    fn create_element(
        &mut self,
        name: QualName,
        attrs: Vec<Attribute>,
        _flags: ElementFlags,
    ) -> Self::Handle {
        let mut attr_map: HashMap<String, String> = HashMap::new();
        for attr in attrs {
            attr_map.insert(attr.name.local.to_string(), attr.value.to_string());
        }
        let tag = name.local.to_string();
        let handle = InternalHandle::new(InternalNode::Element {
            tag,
            attrs: attr_map,
        });
        // Initialize the QualName cache.
        let _ = handle.qual_name.set(name);
        handle
    }

    fn create_comment(&mut self, text: StrTendril) -> Self::Handle {
        InternalHandle::new(InternalNode::Comment(text.to_string()))
    }

    fn create_pi(&mut self, target: StrTendril, data: StrTendril) -> Self::Handle {
        InternalHandle::new(InternalNode::ProcessingInstruction {
            target: target.to_string(),
            data: data.to_string(),
        })
    }

    fn append(&mut self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        match child {
            NodeOrText::AppendNode(node) => {
                *node.parent.borrow_mut() = Some(parent.clone());
                parent.children.borrow_mut().push(node);
            }
            NodeOrText::AppendText(text) => {
                merge_text_into_last(parent, text);
            }
        }
    }

    fn append_based_on_parent_node(
        &mut self,
        element: &Self::Handle,
        prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        if element.parent.borrow().is_some() {
            self.append(prev_element, child);
        } else {
            self.append(element, child);
        }
    }

    fn append_doctype_to_document(
        &mut self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        let dt = InternalHandle::new(InternalNode::Doctype {
            name: name.to_string(),
            public_id: public_id.to_string(),
            system_id: system_id.to_string(),
        });
        *dt.parent.borrow_mut() = Some(self.document.clone());
        self.document.children.borrow_mut().push(dt);
    }

    fn get_template_contents(&mut self, target: &Self::Handle) -> Self::Handle {
        // For minimal HTML we don't track a separate template contents fragment;
        // the template element itself holds the children, which is sufficient
        // for the tests and adapter code.
        target.clone()
    }

    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        Rc::ptr_eq(&x.node, &y.node)
    }

    fn set_quirks_mode(&mut self, _mode: QuirksMode) {}

    fn append_before_sibling(
        &mut self,
        sibling: &Self::Handle,
        new_node: NodeOrText<Self::Handle>,
    ) {
        let parent_opt = sibling.parent.borrow().clone();
        let parent = match parent_opt {
            Some(p) => p,
            None => return,
        };
        match new_node {
            NodeOrText::AppendNode(node) => {
                *node.parent.borrow_mut() = Some(parent.clone());
                let mut sibs = parent.children.borrow_mut();
                if let Some(pos) = sibs.iter().position(|h| Rc::ptr_eq(&h.node, &sibling.node)) {
                    sibs.insert(pos, node);
                } else {
                    sibs.push(node);
                }
            }
            NodeOrText::AppendText(text) => {
                let text_handle = InternalHandle::new(InternalNode::Text(text.to_string()));
                *text_handle.parent.borrow_mut() = Some(parent.clone());
                let mut sibs = parent.children.borrow_mut();
                if let Some(pos) = sibs.iter().position(|h| Rc::ptr_eq(&h.node, &sibling.node)) {
                    sibs.insert(pos, text_handle);
                } else {
                    sibs.push(text_handle);
                }
            }
        }
    }

    fn add_attrs_if_missing(&mut self, target: &Self::Handle, attrs: Vec<Attribute>) {
        let mut node = target.node.borrow_mut();
        if let InternalNode::Element { attrs: existing, .. } = &mut *node {
            for attr in attrs {
                existing
                    .entry(attr.name.local.to_string())
                    .or_insert(attr.value.to_string());
            }
        }
    }

    fn remove_from_parent(&mut self, target: &Self::Handle) {
        let parent_opt = target.parent.borrow().clone();
        if let Some(parent) = parent_opt {
            parent
                .children
                .borrow_mut()
                .retain(|h| !Rc::ptr_eq(&h.node, &target.node));
            *target.parent.borrow_mut() = None;
        }
    }

    fn reparent_children(&mut self, node: &Self::Handle, new_parent: &Self::Handle) {
        let children: Vec<InternalHandle> = node.children.borrow_mut().drain(..).collect();
        for child in &children {
            *child.parent.borrow_mut() = Some(new_parent.clone());
        }
        new_parent.children.borrow_mut().extend(children);
    }
}

pub struct HtmlParser;

impl HtmlParser {
    pub fn parse(html: &str) -> Result<Arc<RwLock<DomNode>>, DomError> {
        let sink = DomSink::new();
        let parser = html5ever::parse_document(sink, ParseOpts::default());
        let sink = parser.one(html);
        let root_children: Vec<Arc<RwLock<DomNode>>> = sink
            .document
            .children
            .borrow()
            .iter()
            .map(convert_handle)
            .collect();
        Ok(Arc::new(RwLock::new(DomNode::Document {
            children: root_children,
        })))
    }
}

fn convert_handle(h: &InternalHandle) -> Arc<RwLock<DomNode>> {
    let children_vec: Vec<Arc<RwLock<DomNode>>> = h
        .children
        .borrow()
        .iter()
        .map(convert_handle)
        .collect();
    let node = h.node.borrow();
    let parent_arc: Option<Arc<RwLock<DomNode>>> = h.parent.borrow().as_ref().and_then(|p| {
        Some(convert_handle(p)) // simplified; full parent wiring needs two-pass
    });
    let dom = match &*node {
        InternalNode::Document => DomNode::Document {
            children: children_vec,
            parent: None,
        },
        InternalNode::Element { tag, attrs } => DomNode::Element {
            tag: tag.clone(),
            attrs: attrs.clone(),
            children: children_vec,
            parent: None,
        },
        InternalNode::Text(s) => DomNode::Text { content: s.clone(), parent: None },
        InternalNode::Comment(s) => DomNode::Comment { content: s.clone(), parent: None },
        InternalNode::Doctype { name, public_id, system_id } => DomNode::DocumentType {
            name: name.clone(),
            public_id: public_id.clone(),
            system_id: system_id.clone(),
            parent: None,
        },
        InternalNode::ProcessingInstruction { .. } => DomNode::Comment { content: String::new(), parent: None },
    };
    Arc::new(RwLock::new(dom))
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

    #[test]
    fn test_parse_nested_html_produces_tree() {
        let html = "<div><p>hi</p></div>";
        let root = HtmlParser::parse(html).unwrap();
        let tree = DomTree::new(root);

        // Find div
        let div = tree.query("div").expect("div should be present");
        // div should have exactly one child, which is a p
        let div_children = {
            let g = div.read().unwrap();
            match &*g {
                DomNode::Element { children, .. } => children.clone(),
                _ => panic!("div is not an element"),
            }
        };
        assert_eq!(div_children.len(), 1, "div should have exactly one child");

        // p should have exactly one child, a text node with "hi"
        let p_node = div_children.into_iter().next().unwrap();
        let p_children = {
            let g = p_node.read().unwrap();
            match &*g {
                DomNode::Element { tag, children, .. } => {
                    assert_eq!(tag, "p", "child should be a p tag");
                    children.clone()
                }
                _ => panic!("div child is not an element"),
            }
        };
        assert_eq!(p_children.len(), 1, "p should have exactly one child");
        let text_node = p_children.into_iter().next().unwrap();
        let g = text_node.read().unwrap();
        match &*g {
            DomNode::Text(s) => assert_eq!(s, "hi"),
            _ => panic!("expected text node 'hi'"),
        }
    }

    #[test]
    fn test_parse_script_preserves_content() {
        let html = "<script>if (a < b) {}</script>";
        let root = HtmlParser::parse(html).unwrap();
        let tree = DomTree::new(root);
        let script = tree.query("script").expect("script tag should exist");
        let script_children = {
            let g = script.read().unwrap();
            match &*g {
                DomNode::Element { children, .. } => children.clone(),
                _ => panic!("script is not an element"),
            }
        };
        assert!(!script_children.is_empty(), "script should have preserved content");
        let text_node = script_children.into_iter().next().unwrap();
        let g = text_node.read().unwrap();
        match &*g {
            DomNode::Text(s) => assert!(s.contains("if (a < b) {}"), "script text should contain the comparison"),
            _ => panic!("script content is not text"),
        }
    }
}
pub mod adapter;
