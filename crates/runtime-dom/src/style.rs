// runtime-dom/src/style.rs: minimal CSS + layout subsystem (Phase 6.2).
//
// Data/layout only — no pixels, no renderer. Parses a small property subset
// (`color`, `background-color`, `display`, `width`, `height`, `margin`,
// `padding`), computes a cascaded `ComputedStyle` (inline overrides
// stylesheet) reusing the existing `DomTree` selector engine, and derives a
// minimal `LayoutBox` for elements with explicit dimensions.

use std::sync::{Arc, RwLock};

use crate::{DomNode, DomTree};

/// Four-sided edge values (margin or padding), in pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EdgeSizes {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// Cascaded style for a single element. `None` means "not declared".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComputedStyle {
    pub color: Option<String>,
    pub background_color: Option<String>,
    pub display: Option<String>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub margin: EdgeSizes,
    pub padding: EdgeSizes,
}

/// A single stylesheet rule: selector plus its declarations.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleRule {
    pub selector: String,
    pub declarations: Vec<(String, String)>,
}

/// Minimal box model for an element with explicit dimensions.
///
/// `content_*` are the declared width/height; `border_box_*` add padding and
/// `margin_box_*` add margin. No border width is modelled.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutBox {
    pub content_width: f32,
    pub content_height: f32,
    pub padding: EdgeSizes,
    pub margin: EdgeSizes,
}

impl LayoutBox {
    pub fn border_box_width(&self) -> f32 {
        self.content_width + self.padding.left + self.padding.right
    }
    pub fn border_box_height(&self) -> f32 {
        self.content_height + self.padding.top + self.padding.bottom
    }
    pub fn margin_box_width(&self) -> f32 {
        self.border_box_width() + self.margin.left + self.margin.right
    }
    pub fn margin_box_height(&self) -> f32 {
        self.border_box_height() + self.margin.top + self.margin.bottom
    }
}

/// Strip `/* ... */` comments from a CSS string.
fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(c2) = chars.next() {
                if c2 == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parse a `;`-separated declaration list into `(property, value)` pairs.
/// Property names are lowercased; a trailing `!important` is stripped.
pub fn parse_declarations(input: &str) -> Vec<(String, String)> {
    input
        .split(';')
        .filter_map(|decl| {
            let decl = decl.trim();
            if decl.is_empty() {
                return None;
            }
            let (prop, value) = decl.split_once(':')?;
            let prop = prop.trim().to_ascii_lowercase();
            let value = value.trim();
            let value = value
                .strip_suffix("!important")
                .map(str::trim)
                .unwrap_or(value);
            if prop.is_empty() || value.is_empty() {
                None
            } else {
                Some((prop, value.to_string()))
            }
        })
        .collect()
}

/// Parse a CSS length. Supports bare numbers and `px`; other units return `None`.
fn parse_length(value: &str) -> Option<f32> {
    let value = value.trim();
    let numeric = value.strip_suffix("px").unwrap_or(value).trim();
    numeric.parse::<f32>().ok()
}

/// Apply a CSS `margin`/`padding` shorthand (1–4 whitespace-separated values).
fn apply_edge_shorthand(edges: &mut EdgeSizes, value: &str) {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let parsed: Option<Vec<f32>> = parts.iter().map(|p| parse_length(p)).collect();
    let parsed = match parsed {
        Some(p) if !p.is_empty() && p.len() <= 4 => p,
        _ => return,
    };
    match parsed.as_slice() {
        [all] => {
            edges.top = *all;
            edges.right = *all;
            edges.bottom = *all;
            edges.left = *all;
        }
        [vertical, horizontal] => {
            edges.top = *vertical;
            edges.bottom = *vertical;
            edges.right = *horizontal;
            edges.left = *horizontal;
        }
        [top, horizontal, bottom] => {
            edges.top = *top;
            edges.right = *horizontal;
            edges.bottom = *bottom;
            edges.left = *horizontal;
        }
        [top, right, bottom, left] => {
            edges.top = *top;
            edges.right = *right;
            edges.bottom = *bottom;
            edges.left = *left;
        }
        _ => {}
    }
}

/// Apply declarations onto a `ComputedStyle`. Later declarations win.
pub fn apply_declarations(style: &mut ComputedStyle, decls: &[(String, String)]) {
    for (prop, value) in decls {
        match prop.as_str() {
            "color" => style.color = Some(value.clone()),
            "background-color" => style.background_color = Some(value.clone()),
            "display" => style.display = Some(value.clone()),
            "width" => {
                if let Some(v) = parse_length(value) {
                    style.width = Some(v);
                }
            }
            "height" => {
                if let Some(v) = parse_length(value) {
                    style.height = Some(v);
                }
            }
            "margin" => apply_edge_shorthand(&mut style.margin, value),
            "padding" => apply_edge_shorthand(&mut style.padding, value),
            "margin-top" => {
                if let Some(v) = parse_length(value) {
                    style.margin.top = v;
                }
            }
            "margin-right" => {
                if let Some(v) = parse_length(value) {
                    style.margin.right = v;
                }
            }
            "margin-bottom" => {
                if let Some(v) = parse_length(value) {
                    style.margin.bottom = v;
                }
            }
            "margin-left" => {
                if let Some(v) = parse_length(value) {
                    style.margin.left = v;
                }
            }
            "padding-top" => {
                if let Some(v) = parse_length(value) {
                    style.padding.top = v;
                }
            }
            "padding-right" => {
                if let Some(v) = parse_length(value) {
                    style.padding.right = v;
                }
            }
            "padding-bottom" => {
                if let Some(v) = parse_length(value) {
                    style.padding.bottom = v;
                }
            }
            "padding-left" => {
                if let Some(v) = parse_length(value) {
                    style.padding.left = v;
                }
            }
            _ => {}
        }
    }
}

/// Parse an inline `style="..."` attribute into a `ComputedStyle`.
pub fn parse_inline_style(input: &str) -> ComputedStyle {
    let mut style = ComputedStyle::default();
    apply_declarations(&mut style, &parse_declarations(input));
    style
}

/// Parse a stylesheet into rules. Each comma-separated selector becomes its
/// own rule sharing the same declarations.
pub fn parse_stylesheet(css: &str) -> Vec<StyleRule> {
    let css = strip_css_comments(css);
    let mut rules = Vec::new();
    for block in css.split('}') {
        let Some(open) = block.find('{') else {
            continue;
        };
        let selectors = &block[..open];
        let declarations = parse_declarations(&block[open + 1..]);
        if declarations.is_empty() {
            continue;
        }
        for selector in selectors.split(',') {
            let selector = selector.trim();
            if !selector.is_empty() {
                rules.push(StyleRule {
                    selector: selector.to_string(),
                    declarations: declarations.clone(),
                });
            }
        }
    }
    rules
}

fn collect_elements(node: &Arc<RwLock<DomNode>>, out: &mut Vec<Arc<RwLock<DomNode>>>) {
    let (is_element, children): (bool, Vec<Arc<RwLock<DomNode>>>) = {
        let guard = node.read().unwrap();
        match &*guard {
            DomNode::Document { children, .. } => (false, children.clone()),
            DomNode::Element { children, .. } => (true, children.clone()),
            _ => (false, Vec::new()),
        }
    };
    if is_element {
        out.push(Arc::clone(node));
    }
    for child in children {
        collect_elements(&child, out);
    }
}

fn collect_text(node: &DomNode, out: &mut String) {
    match node {
        DomNode::Text { content, .. } => out.push_str(content),
        DomNode::Element { children, .. } => {
            for child in children {
                collect_text(&child.read().unwrap(), out);
            }
        }
        _ => {}
    }
}

/// Minimal style engine: a list of rules plus cascade/layout helpers.
#[derive(Clone, Debug, Default)]
pub struct StyleEngine {
    pub rules: Vec<StyleRule>,
}

impl StyleEngine {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn from_css(css: &str) -> Self {
        Self { rules: parse_stylesheet(css) }
    }

    /// Build an engine from the `<style>` blocks found in a parsed document.
    pub fn from_document(root: &Arc<RwLock<DomNode>>) -> Self {
        let tree = DomTree::new(Arc::clone(root));
        let mut css = String::new();
        for style_node in tree.query_all("style") {
            collect_text(&style_node.read().unwrap(), &mut css);
            css.push('\n');
        }
        Self::from_css(&css)
    }

    /// Compute the cascaded style for a single node. Stylesheet rules apply in
    /// source order; the inline `style` attribute overrides them.
    pub fn compute_for(
        &self,
        root: &Arc<RwLock<DomNode>>,
        node: &Arc<RwLock<DomNode>>,
    ) -> ComputedStyle {
        let mut style = ComputedStyle::default();
        let tree = DomTree::new(Arc::clone(root));
        for rule in &self.rules {
            let matched = tree
                .query_all(&rule.selector)
                .iter()
                .any(|n| Arc::ptr_eq(n, node));
            if matched {
                apply_declarations(&mut style, &rule.declarations);
            }
        }
        let inline = {
            let guard = node.read().unwrap();
            match &*guard {
                DomNode::Element { attrs, .. } => attrs.get("style").cloned(),
                _ => None,
            }
        };
        if let Some(inline) = inline {
            apply_declarations(&mut style, &parse_declarations(&inline));
        }
        style
    }

    /// Compute cascaded styles for every element in the tree.
    pub fn compute_all(
        &self,
        root: &Arc<RwLock<DomNode>>,
    ) -> Vec<(Arc<RwLock<DomNode>>, ComputedStyle)> {
        let mut elements = Vec::new();
        collect_elements(root, &mut elements);
        elements
            .into_iter()
            .map(|node| {
                let style = self.compute_for(root, &node);
                (node, style)
            })
            .collect()
    }
}

/// Compute a minimal box model for a style with explicit width and height.
/// Returns `None` when either dimension is unspecified (e.g. `auto`).
pub fn compute_layout(style: &ComputedStyle) -> Option<LayoutBox> {
    let width = style.width?;
    let height = style.height?;
    Some(LayoutBox {
        content_width: width,
        content_height: height,
        padding: style.padding.clone(),
        margin: style.margin.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HtmlParser;

    #[test]
    fn test_parse_inline_style_properties() {
        let style = parse_inline_style(
            "color: red; background-color: #fff; display: flex; width: 100px; height: 50px",
        );
        assert_eq!(style.color.as_deref(), Some("red"));
        assert_eq!(style.background_color.as_deref(), Some("#fff"));
        assert_eq!(style.display.as_deref(), Some("flex"));
        assert_eq!(style.width, Some(100.0));
        assert_eq!(style.height, Some(50.0));
    }

    #[test]
    fn test_parse_inline_style_edge_shorthands() {
        let style = parse_inline_style("padding: 5px 10px; margin: 1px 2px 3px 4px");
        assert_eq!(style.padding.top, 5.0);
        assert_eq!(style.padding.bottom, 5.0);
        assert_eq!(style.padding.left, 10.0);
        assert_eq!(style.padding.right, 10.0);
        assert_eq!(style.margin.top, 1.0);
        assert_eq!(style.margin.right, 2.0);
        assert_eq!(style.margin.bottom, 3.0);
        assert_eq!(style.margin.left, 4.0);
    }

    #[test]
    fn test_stylesheet_rule_applies_and_inline_overrides() {
        let html = "<html><head><style>.box { color: blue; width: 50px; }</style></head>\
                    <body><div class='box' style='color: green'>x</div></body></html>";
        let root = HtmlParser::parse(html).unwrap();
        let engine = StyleEngine::from_document(&root);
        assert!(!engine.rules.is_empty(), "stylesheet rule must be parsed");

        let tree = DomTree::new(Arc::clone(&root));
        let div = tree.query("div").expect("div");
        let style = engine.compute_for(&root, &div);
        assert_eq!(style.color.as_deref(), Some("green"), "inline must override stylesheet");
        assert_eq!(style.width, Some(50.0), "stylesheet width must apply");
    }

    #[test]
    fn test_stylesheet_selector_reuses_engine_tag_id_class() {
        let css = "div { color: red; } #main { color: green; } .big { color: blue; }";
        let html = "<html><body><div id='main' class='big'>x</div></body></html>";
        let root = HtmlParser::parse(html).unwrap();
        let engine = StyleEngine::from_css(css);
        let tree = DomTree::new(Arc::clone(&root));
        let div = tree.query("div").unwrap();
        let style = engine.compute_for(&root, &div);
        // Later rule in source order wins for equal specificity in this minimal
        // cascade: `.big` (blue) is applied last.
        assert_eq!(style.color.as_deref(), Some("blue"));
    }

    #[test]
    fn test_box_model_explicit_dimensions() {
        let style = parse_inline_style("width: 100px; height: 50px; padding: 10px; margin: 5px");
        let layout = compute_layout(&style).expect("explicit dimensions must produce a box");
        assert_eq!(layout.content_width, 100.0);
        assert_eq!(layout.content_height, 50.0);
        assert_eq!(layout.border_box_width(), 120.0);
        assert_eq!(layout.border_box_height(), 70.0);
        assert_eq!(layout.margin_box_width(), 130.0);
        assert_eq!(layout.margin_box_height(), 80.0);
    }

    #[test]
    fn test_box_model_requires_explicit_dimensions() {
        let style = parse_inline_style("color: red; padding: 4px");
        assert!(compute_layout(&style).is_none());
    }

    #[test]
    fn test_compute_all_elements() {
        let html = "<html><body><div style='width: 10px; height: 20px'></div><span></span></body></html>";
        let root = HtmlParser::parse(html).unwrap();
        let engine = StyleEngine::new();
        let styles = engine.compute_all(&root);
        assert!(styles.len() >= 3, "html/body/div/span elements expected");
        let div_style = styles
            .iter()
            .find(|(n, _)| matches!(&*n.read().unwrap(), DomNode::Element { tag, .. } if tag == "div"))
            .map(|(_, s)| s.clone())
            .expect("div style");
        assert_eq!(div_style.width, Some(10.0));
        assert_eq!(div_style.height, Some(20.0));
    }
}