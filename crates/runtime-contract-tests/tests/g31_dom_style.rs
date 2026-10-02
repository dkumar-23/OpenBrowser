//! G31 — Minimal CSS/Layout pipeline (Phase 6.2)
//!
//! Observable: inline declarations parse, stylesheet rules apply and inline
//! overrides them, and a minimal box model is derived for elements with
//! explicit dimensions. Data/layout only — no rendering.

use std::sync::Arc;

use runtime_dom::style::{compute_layout, parse_inline_style, StyleEngine};
use runtime_dom::{DomNode, DomTree, HtmlParser};

#[test]
fn inline_style_parses_property_subset() {
    let style = parse_inline_style(
        "color: red; background-color: #eee; display: inline-block; width: 120px; height: 40px",
    );
    assert_eq!(style.color.as_deref(), Some("red"));
    assert_eq!(style.background_color.as_deref(), Some("#eee"));
    assert_eq!(style.display.as_deref(), Some("inline-block"));
    assert_eq!(style.width, Some(120.0));
    assert_eq!(style.height, Some(40.0));
}

#[test]
fn stylesheet_rule_applies_and_inline_overrides() {
    let html = "<html><head><style>.card { color: blue; width: 80px; }</style></head>\
                <body><div class='card' style='color: green; height: 20px'>x</div></body></html>";
    let root = HtmlParser::parse(html).expect("parse");
    let engine = StyleEngine::from_document(&root);
    assert!(!engine.rules.is_empty(), "stylesheet must yield rules");

    let tree = DomTree::new(Arc::clone(&root));
    let card = tree.query("div").expect("card element");
    let style = engine.compute_for(&root, &card);
    assert_eq!(style.color.as_deref(), Some("green"), "inline overrides stylesheet");
    assert_eq!(style.width, Some(80.0), "stylesheet width applies");
    assert_eq!(style.height, Some(20.0), "inline height applies");
}

#[test]
fn box_model_computes_for_explicit_dimensions() {
    let style = parse_inline_style("width: 100px; height: 50px; padding: 10px; margin: 5px");
    let layout = compute_layout(&style).expect("explicit dimensions produce a box");
    assert_eq!(layout.content_width, 100.0);
    assert_eq!(layout.content_height, 50.0);
    assert_eq!(layout.border_box_width(), 120.0);
    assert_eq!(layout.border_box_height(), 70.0);
    assert_eq!(layout.margin_box_width(), 130.0);
    assert_eq!(layout.margin_box_height(), 80.0);
}

#[test]
fn box_model_absent_without_explicit_dimensions() {
    let html = "<html><body><p style='color: red; padding: 4px'>text</p></body></html>";
    let root = HtmlParser::parse(html).expect("parse");
    let tree = DomTree::new(Arc::clone(&root));
    let p = tree.query("p").expect("p element");
    let style = StyleEngine::new().compute_for(&root, &p);
    assert!(compute_layout(&style).is_none());

    // Sanity: the element is still present and typed.
    assert!(matches!(&*p.read().unwrap(), DomNode::Element { tag, .. } if tag == "p"));
}