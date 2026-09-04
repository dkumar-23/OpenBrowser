//! Fetch wrapper — Phase 2.3 minimal.
use runtime_network::{HttpClient, Method, Request};


#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub method: Method,
    pub url: String,
    pub body: Option<Vec<u8>>,
}

impl FetchRequest {
    pub fn get(url: String) -> Self { Self { method: Method::Get, url, body: None } }
    pub fn post(url: String, body: Vec<u8>) -> Self { Self { method: Method::Post, url, body: Some(body) } }
}

pub(super) async fn do_fetch(client: &HttpClient, req: FetchRequest) -> anyhow::Result<runtime_network::Response> {
    let r = Request { method: req.method, url: req.url, headers: vec![], body: req.body, content_type: None, timeout: None };
    client.execute(r).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn fetch_get() { let _ = FetchRequest::get("http://x.com".into()); }
    #[test] fn fetch_post() { let _ = FetchRequest::post("http://x.com".into(), vec![]); }
}
