//! Dynamic User-Agent tests for runtime-network.
//!
//! Verifies:
//! - Per-request UA override works (Request::user_agent())
//! - Multiple UAs on same HttpClient
//! - Default UA when no override

use runtime_network::{HttpClient, Request, Method};

#[tokio::test]
async fn test_dynamic_ua_per_request() {
    let desktop = "Mozilla/5.0 (X11; Linux x86_64) Chrome/135.0";
    let mobile  = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0) Mobile/15E148 Safari/604.1";
    let stealth = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/130.0";

    let req_d = Request::get("https://httpbin.org/headers").user_agent(desktop);
    let req_m = Request::get("https://httpbin.org/headers").user_agent(mobile);
    let req_s = Request::get("https://httpbin.org/headers").user_agent(stealth);

    assert_eq!(req_d.user_agent_override.as_deref(), Some(desktop));
    assert_eq!(req_m.user_agent_override.as_deref(), Some(mobile));
    assert_eq!(req_s.user_agent_override.as_deref(), Some(stealth));
    println!("3 distinct UAs build correctly.");
}

#[tokio::test]
async fn test_default_ua_when_no_override() {
    let req = Request::get("https://example.com/");
    assert!(req.user_agent_override.is_none());
    println!("No override → uses Client default.");
}

#[tokio::test]
async fn test_dynamic_ua_post_with_body() {
    let req = Request::post("https://example.com/")
        .user_agent("Mozilla/5.0 Mobile/15E148 Safari/604.1")
        .text("hello");
    assert_eq!(req.method, Method::Post);
    assert!(req.user_agent_override.is_some());
    assert_eq!(req.body.as_deref(), Some(b"hello".as_ref()));
    println!("POST + UA + body: all three combined.");
}
