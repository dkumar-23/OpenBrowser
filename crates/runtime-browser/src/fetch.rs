//! Fetch wrapper — Phase 2.3.
use runtime_network::{HttpClient, Method, Request};

#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub method: Method,
    pub url: String,
    pub body: Option<Vec<u8>>,
    pub user_agent_override: Option<String>,
}

impl FetchRequest {
    pub fn get(url: String) -> Self {
        Self { method: Method::Get, url, body: None, user_agent_override: None }
    }
    pub fn post(url: String, body: Vec<u8>) -> Self {
        Self { method: Method::Post, url, body: Some(body), user_agent_override: None }
    }
    pub fn with_ua(mut self, ua: String) -> Self {
        self.user_agent_override = Some(ua);
        self
    }
}

pub(super) async fn do_fetch(client: &HttpClient, req: FetchRequest) -> anyhow::Result<runtime_network::Response> {
    let r = Request {
        method: req.method,
        url: req.url,
        headers: vec![],
        body: req.body,
        content_type: None,
        timeout: None,
        user_agent_override: req.user_agent_override,
    };
    client.execute(r).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn fetch_get() { let _ = FetchRequest::get("http://x.com".into()); }
    #[test] fn fetch_post() { let _ = FetchRequest::post("http://x.com".into(), vec![]); }
    #[test] fn fetch_ua() {
        let r = FetchRequest::get("http://x.com".into()).with_ua("CustomAgent/1.0".into());
        assert!(r.user_agent_override.is_some());
    }
}
