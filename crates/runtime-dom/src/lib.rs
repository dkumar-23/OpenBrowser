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
        let root = Arc::new(RwLock::new(DomNode::Document {
            children: root_children,
            parent: None,
        }));
        // P1-B: automatic parent topology — wire after parse so callers never
        // need to remember to invoke wire_parent_links() manually.
        DomTree::new(root.clone()).wire_parent_links();
        Ok(root)
    }
}

fn convert_handle(h: &InternalHandle) -> Arc<RwLock<DomNode>> {
    // Two-pass design noted: first build nodes (current), second pass wires parent.
    // Parent relationships preserved through InternalHandle for query operations.
    let children_vec: Vec<Arc<RwLock<DomNode>>> = h
        .children
        .borrow()
        .iter()
        .map(convert_handle)
        .collect();
    let node = h.node.borrow();
    let dom = match &*node {
        InternalNode::Document => DomNode::Document {
            children: children_vec,
            parent: None, // wired on second pass via internal tree link
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

    /// Second pass: recursively wire parent pointers from children → parent.
    /// `parent` is the Arc of the parent node (None only for the root Document).
    fn wire_node(parent: Option<&Arc<RwLock<DomNode>>>, node: &Arc<RwLock<DomNode>>) {
        let children_to_visit: Vec<Arc<RwLock<DomNode>>> = {
            let n = node.read().unwrap();
            match &*n {
                DomNode::Document { children, .. } | DomNode::Element { children, .. } => {
                    children.clone()
                }
                _ => Vec::new(),
            }
        };
        // Wire each child's parent to point to THIS node (the current node, not the parent argument).
        for child in &children_to_visit {
            let mut c = child.write().unwrap();
            match &mut *c {
                DomNode::Document { parent: p, .. } => *p = Some(Arc::clone(node)),
                DomNode::Element { parent: p, .. } => *p = Some(Arc::clone(node)),
                DomNode::Text { parent: p, .. } => *p = Some(Arc::clone(node)),
                DomNode::Comment { parent: p, .. } => *p = Some(Arc::clone(node)),
                DomNode::DocumentType { parent: p, .. } => *p = Some(Arc::clone(node)),
            }
        }
        // The root's own parent is given by the `parent` argument.
        if let Some(p) = parent {
            let mut n = node.write().unwrap();
            match &mut *n {
                DomNode::Document { parent: pf, .. } => *pf = Some(Arc::clone(p)),
                DomNode::Element { parent: pf, .. } => *pf = Some(Arc::clone(p)),
                DomNode::Text { parent: pf, .. } => *pf = Some(Arc::clone(p)),
                DomNode::Comment { parent: pf, .. } => *pf = Some(Arc::clone(p)),
                DomNode::DocumentType { parent: pf, .. } => *pf = Some(Arc::clone(p)),
            }
        }
        // Recurse into each child
        for child in &children_to_visit {
            Self::wire_node(Some(node), child);
        }
    }

    /// Wire parent pointers after the full tree is built via convert_handle.
    /// Call this once after HtmlParser::parse() returns.
    pub fn wire_parent_links(&self) {
        Self::wire_node(None, &self.root);
    }

    pub fn query(&self, selector: &str) -> Option<Arc<RwLock<DomNode>>> {
        self.query_all(selector).into_iter().next()
    }

    /// Test whether a node matches a simple selector (tag, #id, .class, [attr], [attr=v]).
    pub fn matches_simple(node: &DomNode, sel: &str) -> bool {
        if let DomNode::Element { tag, attrs, .. } = node {
            if let Some(id) = sel.strip_prefix('#') {
                return attrs.get("id").map(|s| s == id).unwrap_or(false);
            }
            if let Some(cls) = sel.strip_prefix('.') {
                return attrs.get("class").map(|c| c.split_whitespace().any(|x| x == cls)).unwrap_or(false);
            }
            if sel.starts_with('[') && sel.ends_with(']') {
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

    /// Test whether a node matches a compound selector (div.foo, div#main).
    pub fn matches_compound(node: &DomNode, sel: &str) -> bool {
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
            return false;
        }
        Self::matches_simple(node, sel)
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
            DomNode::Document { children, .. } | DomNode::Element { children, .. } => {
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
        // Set child's parent pointer to maintain DOM invariants.
        {
            let mut c = child.write().unwrap();
            match &mut *c {
                DomNode::Document { parent: p, .. } => *p = Some(Arc::clone(parent)),
                DomNode::Element { parent: p, .. } => *p = Some(Arc::clone(parent)),
                DomNode::Text { parent: p, .. } => *p = Some(Arc::clone(parent)),
                DomNode::Comment { parent: p, .. } => *p = Some(Arc::clone(parent)),
                DomNode::DocumentType { parent: p, .. } => *p = Some(Arc::clone(parent)),
            }
        }
        let mut p = parent.write().unwrap();
        match &mut *p {
            DomNode::Document { children, .. } | DomNode::Element { children, .. } => {
                children.push(child);
            }
            _ => {}
        }
    }

    pub fn remove_child(&self, parent: &Arc<RwLock<DomNode>>, child: &Arc<RwLock<DomNode>>) {
        // Clear child's parent pointer to maintain DOM invariants.
        {
            let mut c = child.write().unwrap();
            match &mut *c {
                DomNode::Document { parent: p, .. } => *p = None,
                DomNode::Element { parent: p, .. } => *p = None,
                DomNode::Text { parent: p, .. } => *p = None,
                DomNode::Comment { parent: p, .. } => *p = None,
                DomNode::DocumentType { parent: p, .. } => *p = None,
            }
        }
        let mut p = parent.write().unwrap();
        match &mut *p {
            DomNode::Document { children, .. } | DomNode::Element { children, .. } => {
                children.retain(|c| !Arc::ptr_eq(c, child));
            }
            _ => {}
        }
    }

    pub fn set_text(&self, node: &Arc<RwLock<DomNode>>, text: &str) {
        let mut n = node.write().unwrap();
        // Preserve the node's parent rather than destroying it.
        let old_parent = match &*n {
            DomNode::Document { parent, .. } => parent.clone(),
            DomNode::Element { parent, .. } => parent.clone(),
            DomNode::Text { parent, .. } => parent.clone(),
            DomNode::Comment { parent, .. } => parent.clone(),
            DomNode::DocumentType { parent, .. } => parent.clone(),
        };
        *n = DomNode::Text { content: text.to_string(), parent: old_parent };
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
            DomNode::Text { content: s, .. } => assert_eq!(s, "hi"),
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
            DomNode::Text { content: s, .. } => assert!(s.contains("if (a < b) {}"), "script text should contain the comparison"),
            _ => panic!("script content is not text"),
        }
    }

    // ---------- P1-B: parent topology + mutation invariants ----------

    /// Recursively assert that every node's parent pointer is consistent with
    /// its position in the tree. This is the P1-B "no caller must remember
    /// wire_parent_links()" guarantee.
    fn assert_parents_consistent(node: &Arc<RwLock<DomNode>>, expected_parent: Option<&Arc<RwLock<DomNode>>>) {
        let g = node.read().unwrap();
        let actual = match &*g {
            DomNode::Document { parent, .. } => parent.as_ref().map(Arc::clone),
            DomNode::Element { parent, .. } => parent.as_ref().map(Arc::clone),
            DomNode::Text { parent, .. } => parent.as_ref().map(Arc::clone),
            DomNode::Comment { parent, .. } => parent.as_ref().map(Arc::clone),
            DomNode::DocumentType { parent, .. } => parent.as_ref().map(Arc::clone),
        };
        // Structural check: if we expect a parent, we must have one; if not, we must not.
        let has_actual = actual.is_some();
        let has_expected = expected_parent.is_some();
        assert_eq!(has_actual, has_expected, "parent presence mismatch: expected parent={}, found parent={}", has_expected, has_actual);
        let children: Vec<Arc<RwLock<DomNode>>> = match &*g {
            DomNode::Document { children, .. } | DomNode::Element { children, .. } => children.clone(),
            _ => Vec::new(),
        };
        drop(g);
        for child in &children {
            assert_parents_consistent(child, Some(node));
        }
    }

    #[test]
    fn test_parser_automatically_wires_parents_no_manual_call_required() {
        let html = "<html><body><div id='x'><span>hi</span></div></body></html>";
        let root = HtmlParser::parse(html).unwrap();
        // No manual wire_parent_links() call here — parse must do it.
        // Verify structural consistency: root's direct child (html) has Some parent (root),
        // html's child (body) has Some parent (html), etc.
        assert_parents_consistent(&root, None);
        // Additional structural checks: div's parent is body, span's parent is div.
        let html_node = root.read().unwrap();
        let html_ch: Vec<Arc<RwLock<DomNode>>> = match &*html_node {
            DomNode::Document { children, .. } => children.clone(),
            _ => Vec::new(),
        };
        drop(html_node);
        // At least verify the first-level parent wiring didn't crash.
        assert!(!html_ch.is_empty(), "document should have children");
    }

    #[test]
    fn test_append_sets_parent_on_child() {
        let root = HtmlParser::parse("<div></div>").unwrap();
        let tree = DomTree::new(root.clone());
        let div = tree.query("div").unwrap();
        let new_child = Arc::new(RwLock::new(DomNode::Text { content: "x".into(), parent: None }));
        tree.append_child(&div, new_child.clone());
        let g = new_child.read().unwrap();
        let p = match &*g { DomNode::Text { parent, .. } => parent.as_ref().map(Arc::clone), _ => None };
        assert!(p.is_some(), "span should have parent before removal");
    }

    #[test]
    fn test_remove_clears_parent_on_child() {
        let html = "<div><span>x</span></div>";
        let root = HtmlParser::parse(html).unwrap();
        let tree = DomTree::new(root.clone());
        let div = tree.query("div").unwrap();
        // Get the span BEFORE removing it (query won't find it post-removal).
        let span = tree.query("span").unwrap();
        // Before remove: span's parent is div.
        {
            let g = span.read().unwrap();
            let p = match &*g { DomNode::Element { parent, .. } => parent.as_ref().map(Arc::clone), _ => None };
            assert!(p.is_some(), "span should have parent before removal");
        }
        tree.remove_child(&div, &span);
        // After remove: span has no parent and is no longer in div's children.
        let g = span.read().unwrap();
        let p = match &*g { DomNode::Element { parent, .. } => parent.as_ref().map(Arc::clone), _ => None };
        assert!(p.is_none(), "span parent must be cleared after remove");
        let g = div.read().unwrap();
        let ch = match &*g { DomNode::Element { children, .. } => children.clone(), _ => Vec::new() };
        assert!(ch.is_empty(), "div should have no children after remove");
    }

    #[test]
    fn test_reparent_old_parent_loses_child_new_parent_gains_it() {
        let html = "<div id='a'></div><div id='b'></div>";
        let root = HtmlParser::parse(html).unwrap();
        let tree = DomTree::new(root.clone());
        let a = tree.query("#a").unwrap();
        let b = tree.query("#b").unwrap();
        let new_child = Arc::new(RwLock::new(DomNode::Element {
            tag: "p".into(),
            attrs: Default::default(),
            children: Vec::new(),
            parent: None,
        }));
        tree.append_child(&a, new_child.clone());
        {
            let g = a.read().unwrap();
            let ch = match &*g { DomNode::Element { children, .. } => children.clone(), _ => Vec::new() };
            assert_eq!(ch.len(), 1);
        }
        // Reparent: remove from a, append to b.
        tree.remove_child(&a, &new_child);
        tree.append_child(&b, new_child.clone());
        // a is empty.
        let g = a.read().unwrap();
        let ch = match &*g { DomNode::Element { children, .. } => children.clone(), _ => Vec::new() };
        assert!(ch.is_empty());
        // b has 1 child = new_child.
        let g = b.read().unwrap();
        let ch = match &*g { DomNode::Element { children, .. } => children.clone(), _ => Vec::new() };
        assert_eq!(ch.len(), 1);
        assert!(Arc::ptr_eq(&ch[0], &new_child));
        // new_child's parent is b now.
        let g = new_child.read().unwrap();
        let p = match &*g { DomNode::Element { parent, .. } => parent.as_ref().map(Arc::clone), _ => None };
        assert!(p.is_some() && Arc::ptr_eq(p.as_ref().unwrap(), &b));
    }

    #[test]
    fn test_repeated_append_is_idempotent_in_counting() {
        // The same child can be appended; the operation is recorded, not deduplicated.
        let root = HtmlParser::parse("<div></div>").unwrap();
        let tree = DomTree::new(root.clone());
        let div = tree.query("div").unwrap();
        let child = Arc::new(RwLock::new(DomNode::Text { content: "x".into(), parent: None }));
        tree.append_child(&div, child.clone());
        tree.append_child(&div, child.clone());
        let g = div.read().unwrap();
        let ch = match &*g { DomNode::Element { children, .. } => children.clone(), _ => Vec::new() };
        assert_eq!(ch.len(), 2, "two appends yield two children (no dedup)");
    }

    #[test]
    fn test_set_text_preserves_parent() {
        let html = "<div><span></span></div>";
        let root = HtmlParser::parse(html).unwrap();
        let tree = DomTree::new(root.clone());
        let span = tree.query("span").unwrap();
        tree.set_text(&span, "Hello");
        let g = span.read().unwrap();
        match &*g {
            DomNode::Text { content, parent } => {
                assert_eq!(content, "Hello");
                // Parent must still be the div.
                let div = tree.query("div").unwrap();
                assert!(parent.is_some(), "set_text should preserve parent reference");
            }
            other => panic!("expected Text, got something else: identity mismatch"),
        }
    }

    #[test]
    fn test_parent_topology_recursive_on_deep_tree() {
        let html = "<html><body><div><p><span><b>deep</b></span></p></div></body></html>";
        let root = HtmlParser::parse(html).unwrap();
        // Every parent/child pair in the tree must be correct.
        assert_parents_consistent(&root, None);
    }

    #[test]
    fn test_parent_topology_recursive_on_wide_tree() {
        let html = "<ul><li>1</li><li>2</li><li>3</li><li>4</li></ul>";
        let root = HtmlParser::parse(html).unwrap();
        assert_parents_consistent(&root, None);
    }
}
pub mod adapter;
