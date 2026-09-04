//! Phase 2.3: runtime-browser — minimal working crate for navigation/forms/fetch/timers.

mod navigate;
mod forms;
mod fetch;
mod timers;

pub use forms::{FormData, FormSubmitOptions};
pub use fetch::FetchRequest;
pub use timers::{TimerHandle, IntervalHandle};

use thiserror::Error;
use runtime_network::HttpClient;

#[derive(Error, Debug)]
pub enum BrowserError {
    #[error("navigation failed: {0}")]
    Navigation(String),
    #[error("form submission failed: {0}")]
    FormSubmission(String),
    #[error("fetch failed: {0}")]
    Fetch(String),
}

#[derive(Debug, Clone, Default)]
pub struct NavigationPolicy {
    pub allowed_schemes: Vec<String>,
    pub block_external: bool,
    pub max_redirects: Option<usize>,
}

#[derive(Debug, Clone)]
pub enum NavigationEvent {
    Started { url: String },
    Redirected { from: String, to: String },
    Loaded { url: String, status: u16 },
    Error { url: String, message: String },
}

#[derive(Debug, Clone)]
pub struct ObservedCookie {
    pub name: String, pub value: String,
    pub domain: Option<String>, pub path: Option<String>,
    pub secure: bool, pub http_only: bool,
}

#[derive(Debug)]
pub struct NavigationResult {
    pub status: u16,
    pub final_url: String,
    pub cookies: Vec<ObservedCookie>,
    pub dom: runtime_dom::DomTree,
}

#[derive(Debug, Clone)]
pub struct Browser {
    pub client: HttpClient,
}

impl Browser {
    pub fn new() -> anyhow::Result<Self> { Ok(Self { client: HttpClient::new()? }) }
    pub fn with_client(client: HttpClient) -> Self { Self { client } }
    pub async fn navigate(&self, url: &str) -> Result<NavigationResult, BrowserError> {
        navigate::do_navigate(&self.client, url).await
    }
    pub fn set_timeout<F>(&self, delay: std::time::Duration, callback: F) -> TimerHandle
    where F: FnOnce() + Send + 'static,
    { timers::set_timeout(delay, callback) }
    pub fn set_interval<F>(&self, period: std::time::Duration, callback: F) -> IntervalHandle
    where F: Fn() + Send + Sync + 'static,
    { timers::set_interval(period, callback) }
    pub fn http_client(&self) -> &HttpClient { &self.client }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn browser_new() { assert!(Browser::new().is_ok()); }
    #[test] fn browser_clone() { let b = Browser::new().unwrap(); let _ = b.clone(); }
}
