//! Runtime HTTP networking layer.
//!
//! Provides a [`HttpClient`] built on top of [`reqwest::Client`] that supports
//! TLS, cookie storage, redirect policies, response compression (gzip / brotli),
//! and tracing / observability integration via [`runtime_observability::TraceContext`].
//!
//! The client is designed to be cheap to clone (it wraps a `reqwest::Client`
//! which is internally an `Arc`), so the public API is `Send + Sync` and can be
//! shared between tasks.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context as _};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::redirect;
use serde::{Deserialize, Serialize};
use tracing::{info_span, Instrument};

use runtime_observability::TraceContext;

/// Default upper bound on redirect hops — matches typical browser behaviour.
pub const DEFAULT_MAX_REDIRECTS: usize = 10;

/// Default request timeout for a single HTTP call.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Builder for [`HttpClient`].
///
/// Use [`Client::builder`] as the entry point.  All options are optional; a
/// builder with no modifications produces a sane default client (TLS via
/// rustls, cookie jar enabled, ≤10 redirect hops, gzip + brotli, 30s timeout).
#[derive(Debug, Clone)]
pub struct ClientBuilder {
    cookie_store: bool,
    max_redirects: usize,
    gzip: bool,
    brotli: bool,
    timeout: Duration,
    user_agent: Option<String>,
    default_headers: HeaderMap,
    accept_invalid_certs: bool,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            cookie_store: true,
            max_redirects: DEFAULT_MAX_REDIRECTS,
            gzip: true,
            brotli: true,
            timeout: DEFAULT_TIMEOUT,
            user_agent: Some("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36".to_string()),
            default_headers: HeaderMap::new(),
            accept_invalid_certs: false,
        }
    }
}

impl ClientBuilder {
    /// Enable / disable the persistent cookie store.
    pub fn cookie_store(mut self, enable: bool) -> Self {
        self.cookie_store = enable;
        self
    }

    /// Cap the number of redirect hops.  `0` disables redirects entirely.
    pub fn max_redirects(mut self, n: usize) -> Self {
        self.max_redirects = n;
        self
    }

    /// Enable / disable gzip response decoding.
    pub fn gzip(mut self, enable: bool) -> Self {
        self.gzip = enable;
        self
    }

    /// Enable / disable brotli response decoding.
    pub fn brotli(mut self, enable: bool) -> Self {
        self.brotli = enable;
        self
    }

    /// Set the per-request timeout.
    pub fn timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }

    /// Override the default `User-Agent` header.  Pass `None` to drop it.
    pub fn user_agent(mut self, ua: Option<String>) -> Self {
        self.user_agent = ua;
        self
    }

    /// Add a default header applied to every request.
    pub fn default_header(mut self, name: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_ref().as_bytes()),
            HeaderValue::from_str(value.as_ref()),
        ) {
            self.default_headers.insert(n, v);
        }
        self
    }

    /// Configure TLS.  When `true`, server certificates are not validated
    /// (intended for development / testing only).
    pub fn accept_invalid_certs(mut self, accept: bool) -> Self {
        self.accept_invalid_certs = accept;
        self
    }

    /// Finalise the configuration and build a [`HttpClient`].
    pub fn build(self) -> anyhow::Result<HttpClient> {
        let mut builder = reqwest::Client::builder()
            .cookie_store(self.cookie_store)
            .gzip(self.gzip)
            .brotli(self.brotli)
            .timeout(self.timeout)
            .danger_accept_invalid_certs(self.accept_invalid_certs)
            .redirect(if self.max_redirects == 0 {
                redirect::Policy::none()
            } else {
                redirect::Policy::limited(self.max_redirects)
            });

        if let Some(ua) = self.user_agent {
            builder = builder.user_agent(ua);
        }

        if !self.default_headers.is_empty() {
            builder = builder.default_headers(self.default_headers);
        }

        let client = builder
            .build()
            .context("failed to build reqwest::Client")?;

        Ok(HttpClient {
            inner: Arc::new(client),
        })
    }
}

/// Async HTTP client with TLS, cookies, redirects, and compression.
#[derive(Debug, Clone)]
pub struct HttpClient {
    inner: Arc<reqwest::Client>,
}

impl Default for HttpClient {
    fn default() -> Self {
        ClientBuilder::default()
            .build()
            .expect("default HttpClient configuration must be valid")
    }
}

impl HttpClient {
    /// Create a new [`ClientBuilder`].
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// Build a default [`HttpClient`] with sensible defaults.
    pub fn new() -> anyhow::Result<Self> {
        ClientBuilder::default().build()
    }

    /// Convenience: GET `url` and return the body as a UTF-8 `String`.
    pub async fn get(&self, url: &str) -> anyhow::Result<String> {
        let req = Request::get(url);
        let resp = self.execute_with_context(req, None).await?;
        resp.text()
    }

    /// Execute a fully-built [`Request`] and return a structured [`Response`].
    ///
    /// This is the primary entry point for general HTTP use.  The caller
    /// controls method, URL, headers, and body; the client only adds its
    /// default headers, timeout, and observability.
    ///
    /// A new [`TraceContext`] is generated automatically for observability.
    pub async fn execute(&self, req: Request) -> anyhow::Result<Response> {
        use runtime_observability::TraceContext;
        use uuid::Uuid;
        let ctx = TraceContext::new(
            Uuid::new_v4(),
            None,
        );
        self.execute_with_context(req, Some(ctx)).await
    }

    /// Execute a request while propagating an explicit [`TraceContext`].
    ///
    /// All events emitted by this call are tagged with the supplied context.
    pub async fn execute_with_trace(
        &self,
        req: Request,
        ctx: TraceContext,
    ) -> anyhow::Result<Response> {
        self.execute_with_context(req, Some(ctx)).await
    }

    /// G12 fix: execute with typed network error mapping.
    pub async fn execute_typed(&self, req: Request) -> Result<Response, HttpError> {
        let url = req.url.clone();
        match self.execute(req).await {
            Ok(resp) => Ok(resp),
            Err(err) => {
                // Best-effort mapping: if the error was produced by reqwest,
                // it will typically be a reqwest::Error wrapped in anyhow.
                // We attempt to downcast; if impossible we fall back to Other.
                let mapped = if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
                    HttpError::from_reqwest(reqwest_err, &url)
                } else {
                    HttpError::Other(err.to_string())
                };
                Err(mapped)
            }
        }
    }

    async fn execute_with_context(
        &self,
        req: Request,
        ctx: Option<TraceContext>,
    ) -> anyhow::Result<Response> {
        let method = req.method.clone();
        let url = req.url.clone();
        let span = info_span!(
            "http_request",
            method = %method.as_str(),
            url = %url,
            task_id = tracing::field::Empty,
            agent_id = tracing::field::Empty,
            request_id = tracing::field::Empty,
        );

        if let Some(c) = &ctx {
            span.record("task_id", tracing::field::display(c.task_id));
            span.record("agent_id", tracing::field::display(c.agent_id));
            span.record("request_id", tracing::field::display(c.request_id));
        }

        async move {
            tracing::info!(method = %method.as_str(), url = %url, ua = ?req.user_agent_override, "http request start");

            let mut builder = self.inner.request(reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET), &url);
            if let Some(body) = req.body {
                builder = builder.body(body);
            }
            for (name, value) in req.headers.iter() {
                if let (Ok(n), Ok(v)) = (
                    HeaderName::from_bytes(name.as_bytes()),
                    HeaderValue::from_str(value),
                ) {
                    builder = builder.header(n, v);
                }
            }
            if let Some(ct) = &req.content_type {
                builder = builder.header(CONTENT_TYPE, ct);
            }
            if let Some(timeout) = req.timeout {
                builder = builder.timeout(timeout);
            }
            // Dynamic User-Agent override per request (agentic: desktop/mobile/stealth)
            if let Some(ua) = &req.user_agent_override {
                builder = builder.header("User-Agent", ua.as_str());
            }

            let response = builder
                .send()
                .await
                .with_context(|| format!("HTTP request to {url} failed"))?;

            let status = response.status();
            let url_final = response.url().to_string();
            let mut headers = Vec::with_capacity(response.headers().len());
            for (name, value) in response.headers().iter() {
                let v = value
                    .to_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| String::from_utf8_lossy(value.as_bytes()).into_owned());
                headers.push((name.as_str().to_string(), v));
            }
            let body = response
                .bytes()
                .await
                .with_context(|| format!("failed to read response body from {url_final}"))?;

            tracing::info!(
                method = %method.as_str(),
                url = %url,
                status = status.as_u16(),
                body_len = body.len(),
                "http request complete"
            );

            Ok(Response {
                status: status.as_u16(),
                url: url_final,
                headers,
                body,
            })
        }
        .instrument(span)
        .await
    }
}

/// HTTP method, deliberately a small enum (not the giant `http::Method`) so
/// it serialises cleanly and stays `Clone + PartialEq`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Patch,
    Options,
    Other(String),
}

impl Method {
    pub fn as_str(&self) -> &str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Head => "HEAD",
            Method::Patch => "PATCH",
            Method::Options => "OPTIONS",
            Method::Other(s) => s.as_str(),
        }
    }
}

impl From<&str> for Method {
    fn from(s: &str) -> Self {
        match s.to_ascii_uppercase().as_str() {
            "GET" => Method::Get,
            "POST" => Method::Post,
            "PUT" => Method::Put,
            "DELETE" => Method::Delete,
            "HEAD" => Method::Head,
            "PATCH" => Method::Patch,
            "OPTIONS" => Method::Options,
            other => Method::Other(other.to_string()),
        }
    }
}

/// A high-level request description that can be turned into a `reqwest`
/// request by [`HttpClient::execute`].
#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub content_type: Option<String>,
    pub timeout: Option<Duration>,
    pub user_agent_override: Option<String>,
}

impl Request {
    /// Construct a GET request.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            content_type: None,
            timeout: None,
            user_agent_override: None,
        }
    }

    /// Construct a POST request.
    pub fn post(url: impl Into<String>) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            content_type: None,
            timeout: None,
            user_agent_override: None,
        }
    }

    /// Attach a header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Attach a string body (sets `content_type` if not already set).
    pub fn text(mut self, body: impl Into<String>) -> Self {
        let body = body.into();
        if self.content_type.is_none() {
            self.content_type = Some("text/plain; charset=utf-8".to_string());
        }
        self.body = Some(body.into_bytes());
        self
    }

    /// Attach a JSON body.
    pub fn json<T: Serialize>(mut self, value: &T) -> anyhow::Result<Self> {
        let bytes = serde_json::to_vec(value)
            .map_err(|e| anyhow!("failed to serialise JSON body: {e}"))?;
        self.content_type = Some("application/json".to_string());
        self.body = Some(bytes);
        Ok(self)
    }

    /// Override the per-request timeout.
    pub fn timeout(mut self, t: Duration) -> Self {
        self.timeout = Some(t);
        self
    }

    /// Per-request User-Agent override.
    /// Lets the agent choose browser identity (desktop/mobile/stealth) per task.
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent_override = Some(ua.into());
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("DNS resolution failed for {0}")]
    DnsFailed(String),
    #[error("TLS handshake failed: {0}")]
    TlsFailed(String),
    #[error("Connection refused at {0}")]
    ConnectionRefused(String),
    #[error("Request timed out after {0:?}")]
    Timeout(std::time::Duration),
    #[error("Request was cancelled")]
    Cancelled,
    #[error("Body read error: {0}")]
    BodyReadError(String),
    #[error("Other transport error: {0}")]
    Other(String),
}

impl HttpError {
    pub fn from_reqwest(err: &reqwest::Error, url: &str) -> Self {
        if err.is_timeout() { Self::Timeout(std::time::Duration::from_secs(30)) }
        else if err.is_connect() { Self::ConnectionRefused(url.to_string()) }
        else if err.is_decode() { Self::BodyReadError(err.to_string()) }
        else if err.is_redirect() { Self::Other(format!("redirect error: {}", err)) }
        else { Self::Other(err.to_string()) }
    }
}

/// Structured HTTP response suitable for replay and logging.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: bytes::Bytes,
}

impl Response {
    /// HTTP status code.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// True if `2xx`.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Body as raw bytes.
    pub fn bytes(&self) -> bytes::Bytes {
        self.body.clone()
    }

    /// Body decoded as UTF-8.
    pub fn text(&self) -> anyhow::Result<String> {
        String::from_utf8(self.body.to_vec())
            .map_err(|e| anyhow!("response body is not valid UTF-8: {e}"))
    }

    /// Body parsed as JSON.
    pub fn json<T: for<'de> Deserialize<'de>>(&self) -> anyhow::Result<T> {
        serde_json::from_slice(&self.body)
            .map_err(|e| anyhow!("failed to deserialise JSON response: {e}"))
    }

    /// Look up a header by name (case-insensitive).  Returns the first match.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

// `bytes::Bytes` is the type returned by `reqwest::Response::bytes`.  We
// re-export it here so downstream crates do not need an extra dependency.
pub use bytes;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_produces_valid_client() {
        let client = HttpClient::builder()
            .cookie_store(true)
            .max_redirects(5)
            .gzip(true)
            .brotli(true)
            .build();
        assert!(client.is_ok());
    }

    #[test]
    fn builder_zero_redirects_disables_redirects() {
        // Just exercise the API surface — reqwest's redirect policy is private.
        let client = HttpClient::builder()
            .max_redirects(0)
            .build()
            .expect("client with redirects disabled");
        // Default `Request::get` should still build.
        let _ = Request::get("http://example.invalid/");
        drop(client);
    }

    #[test]
    fn request_fluent_api() {
        let req = Request::post("https://example.test/api")
            .header("X-Trace", "abc")
            .text("hello")
            .timeout(Duration::from_secs(5));
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.body.as_deref(), Some(b"hello".as_ref()));
        assert_eq!(req.content_type.as_deref(), Some("text/plain; charset=utf-8"));
        assert_eq!(req.timeout, Some(Duration::from_secs(5)));
        assert_eq!(req.headers, vec![("X-Trace".to_string(), "abc".to_string())]);
    }

    #[test]
    fn request_json_body_sets_content_type() {
        let req = Request::post("https://example.test/api")
            .json(&serde_json::json!({"k": 1}))
            .expect("json body");
        assert_eq!(req.content_type.as_deref(), Some("application/json"));
        let body = req.body.expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("parse");
        assert_eq!(parsed["k"], 1);
    }

    #[test]
    fn method_from_str() {
        assert_eq!(Method::from("get"), Method::Get);
        assert_eq!(Method::from("POST"), Method::Post);
        assert_eq!(Method::from("weird"), Method::Other("WEIRD".to_string()));
    }

    #[test]
    fn response_helpers() {
        let resp = Response {
            status: 200,
            url: "http://x".into(),
            headers: vec![("Content-Type".into(), "text/plain".into())],
            body: bytes::Bytes::from_static(b"hi"),
        };
        assert!(resp.is_success());
        assert_eq!(resp.text().unwrap(), "hi");
        assert_eq!(resp.header("content-type"), Some("text/plain"));
    }

    #[tokio::test]
    async fn execute_real_http_with_mockito() {
        let mut server = mockito::Server::new_async().await;
        server.mock("GET", "/hello")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("world")
            .create_async()
            .await;
        let client = HttpClient::new().expect("client");
        let resp = client.execute(Request::get(server.url() + "/hello")).await.expect("execute");
        assert!(resp.is_success());
        assert_eq!(resp.text().unwrap(), "world");
    }

    #[tokio::test]
    async fn get_convenience_returns_body() {
        let mut server = mockito::Server::new_async().await;
        server.mock("GET", "/ping")
            .with_status(200)
            .with_body("pong")
            .create_async()
            .await;
        let client = HttpClient::new().expect("client");
        let body = client.get(&(server.url() + "/ping")).await.expect("get");
        assert_eq!(body, "pong");
    }
}
