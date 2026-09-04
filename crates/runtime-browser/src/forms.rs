//! Form handling stubs — Phase 2.3 minimal.
use std::collections::HashMap;
use std::time::Duration;
use runtime_network::{HttpClient, Method};


#[derive(Debug, Clone)]
pub struct FormData {
    pub action: String,
    pub method: Method,
    pub fields: HashMap<String, String>,
    pub enctype: String,
}

impl FormData {
    pub fn new(action: String, method: Method, fields: HashMap<String, String>) -> Self {
        Self { action, method, fields, enctype: "application/x-www-form-urlencoded".into() }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FormSubmitOptions {
    pub timeout: Option<Duration>,
}

pub(super) async fn do_submit_form(
    client: &HttpClient,
    form: &FormData,
    _options: FormSubmitOptions,
) -> Result<runtime_network::Response, super::BrowserError> {
    use runtime_network::Request;
    let mut fields_str = form.fields.iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>().join("&");
    if fields_str.is_empty() { fields_str = String::new(); }
    let req = Request {
        method: form.method.clone(),
        url: form.action.clone(),
        headers: vec![
            ("Content-Type".to_string(), "application/x-www-form-urlencoded".to_string()),
            ("Accept".to_string(), "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".to_string()),
            ("Accept-Encoding".to_string(), "gzip, deflate, br".to_string()),
            ("Accept-Language".to_string(), "en-US,en;q=0.9".to_string()),
            ("Referer".to_string(), "https://www.google.com/".to_string()),
        ],
        body: Some(fields_str.into_bytes()),
        content_type: Some("application/x-www-form-urlencoded".into()),
        timeout: _options.timeout,
        user_agent_override: Some("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36".into()),
    };
    client.execute(req).await
        .map_err(|e| super::BrowserError::FormSubmission(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn form_new() {
        let f = FormData::new("http://x.com".into(), Method::Post, HashMap::new());
        assert_eq!(f.enctype, "application/x-www-form-urlencoded");
    }
}
