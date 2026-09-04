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

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn form_new() {
        let f = FormData::new("http://x.com".into(), Method::Post, HashMap::new());
        assert_eq!(f.enctype, "application/x-www-form-urlencoded");
    }
}
