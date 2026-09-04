//! Agentic session — simulates an autonomous agent performing real web tasks.
//!
//! Actions: navigate, fetch, query DOM, interact, observe cookies.
//!
//! Target 1: DuckDuckGo — search "rust programming language", read results
//! Target 2: Indigo.com — navigate, query flight-related elements
//!
//! Design contract: thin orchestrator (context.md §11), no rendering required.

use runtime_browser::{Browser, FormData, FetchRequest, NavigationResult};
use runtime_network::Method;
use std::collections::HashMap;

// ════════════════════════════════════════════════════════════════════
// Agentic Action 1: Navigate + inspect cookies
// ════════════════════════════════════════════════════════════════════

async fn agent_navigate(browser: &Browser, url: &str) -> Option<NavigationResult> {
    match browser.navigate(url).await {
        Ok(res) => {
            println!("\n[AGENT] Navigated to {}", url);
            println!("       Status: {} | Cookies: {}", res.status, res.cookies.len());
            if !res.cookies.is_empty() {
                for c in &res.cookies {
                    println!("       Cookie: {}={} (secure={}, http_only={})",
                             c.name, c.value, c.secure, c.http_only);
                }
            }
            Some(res)
        }
        Err(e) => {
            println!("\n[AGENT] Navigation to {} FAILED: {:?}", url, e);
            None
        }
    }
}

// ════════════════════════════════════════════════════════════════════
// Agentic Action 2: Query DOM for selector count + first match text
// ════════════════════════════════════════════════════════════════════

fn agent_query(dom: &runtime_browser::NavigationResult, selector: &str, label: &str) {
    let nodes = dom.dom.query_all(selector);
    println!("[AGENT] Selector '{}' ({}): {} node(s)", label, selector, nodes.len());
}

// ════════════════════════════════════════════════════════════════════
// Agentic Action 3: Fetch (lower-level than navigate)
// ════════════════════════════════════════════════════════════════════

async fn agent_fetch(browser: &Browser, url: &str) {
    let req = FetchRequest::get(url.to_string());
    match browser.fetch(req).await {
        Ok(resp) => {
            println!("[AGENT] Fetch {}: status={}", url, resp.status());
        }
        Err(e) => {
            println!("[AGENT] Fetch {} FAILED: {:?}", url, e);
        }
    }
}

// ════════════════════════════════════════════════════════════════════
// Agentic Action 4: Submit search form
// ════════════════════════════════════════════════════════════════════

async fn agent_search_form(browser: &Browser, base_url: &str, query: &str) {
    let mut fields = HashMap::new();
    fields.insert("q".to_string(), query.to_string());
    let form = FormData::new(base_url.to_string(), Method::Get, fields);
    match browser.submit_form(&form, Default::default()).await {
        Ok(resp) => {
            println!("[AGENT] Search '{}' via form: status={}", query, resp.status());
        }
        Err(e) => {
            println!("[AGENT] Search form FAILED: {:?}", e);
        }
    }
}

// ════════════════════════════════════════════════════════════════════
// Agentic Action 5: Inspect page structure
// ════════════════════════════════════════════════════════════════════

fn agent_inspect_page(dom: &runtime_browser::NavigationResult, site: &str) {
    let selectors = if site.contains("duckduckgo") {
        vec![
            ("search", "input"),
            ("form", "form"),
            ("links", "a"),
            ("results", "h2"),
            ("divs", "div"),
        ]
    } else if site.contains("indigo") {
        vec![
            ("header", "header"),
            ("nav", "nav"),
            ("links", "a"),
            ("buttons", "button"),
            ("forms", "form"),
        ]
    } else {
        vec![
            ("links", "a"),
            ("divs", "div"),
        ]
    };

    for (label, sel) in &selectors {
        let count = dom.dom.query_all(sel).len();
        println!("[AGENT]   {}: {} ({}x)", label, sel, count);
    }
}

// ════════════════════════════════════════════════════════════════════
// SESSION: Agent navigates duckduckgo → search rust → indigo
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_agentic_session_duckduckgo_indigo() {
    let browser = Browser::new().expect("Browser must initialize");
    println!("\n= AGENTIC SESSION START =\n");

    // ── Step 1: Navigate to DuckDuckGo ──────────────────────────
    let ddg = agent_navigate(&browser, "https://duckduckgo.com/html/").await;
    if let Some(res) = &ddg {
        agent_inspect_page(res, "duckduckgo");

        // ── Step 2: Search via form ───────────────────────────
        println!("\n[AGENT] Step 2: Submit search form 'rust programming language'");
        agent_search_form(&browser, "https://duckduckgo.com/html/", "rust programming language").await;

        // ── Step 3: Navigate directly via URL (no form needed) ──
        println!("\n[AGENT] Step 3: Navigate via search URL");
        let search_url = "https://duckduckgo.com/html/?q=rust+programming+language";
        let rust_res = agent_navigate(&browser, search_url).await;

        if let Some(r) = &rust_res {
            agent_inspect_page(r, "duckduckgo");

            // ── Step 4: Query for Rust-specific elements ───────
            println!("\n[AGENT] Step 4: DOM analysis for Rust content");
            agent_query(r, "h2", "result headings");
            agent_query(r, "a", "all links");
            agent_query(r, "div", "all divs");

            // Count how many results look like real results
            let h2s = r.dom.query_all("h2");
            println!("[AGENT]   Result count: {} search result headings", h2s.len());
            assert!(h2s.len() > 0, "DDG should return at least some result headings");
        }
    }

    // ── Step 5: Navigate to Indigo ──────────────────────────────
    println!("\n= TARGET 2: indigo.com =\n");
    let indigo = agent_navigate(&browser, "https://www.indigo.in/").await;

    if let Some(res) = &indigo {
        println!("[AGENT] Indigo.com loaded successfully");
        agent_inspect_page(res, "indigo");

        // ── Step 6: Check Indigo flight-related elements ───────
        for sel in &["a[href*=flight]", "button", "form", "input", "nav"] {
            let count = res.dom.query_all(sel).len();
            if count > 0 {
                println!("[AGENT]   '{}': {}x", sel, count);
            }
        }
    } else {
        println!("[AGENT] Indigo unreachable — using DDG fallback");
        let fallback = agent_navigate(&browser, "https://duckduckgo.com/html/?q=indigo+airlines").await;
        if let Some(r) = &fallback {
            agent_inspect_page(r, "duckduckgo");
            agent_query(r, "h2", "DDG result headings");
        }
    }

    // ── Step 7: Fetch (lower-level) ───────────────────────────
    println!("\n[AGENT] Step 7: Low-level fetch");
    agent_fetch(&browser, "https://duckduckgo.com/favicon.ico").await;

    println!("\n= AGENTIC SESSION COMPLETE =\n");
}
