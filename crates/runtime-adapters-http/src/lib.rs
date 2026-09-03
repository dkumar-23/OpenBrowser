use runtime_observability::{TraceContext, Observability};
use runtime_auth::AgentIdentity;
use runtime_policy::PolicyEngine;

pub struct HttpAdapter {
    client: runtime_network::HttpClient,
    observability: std::sync::Arc<dyn Observability>,
    policy: std::sync::Arc<PolicyEngine>,
}

impl HttpAdapter {
    pub fn new(observability: std::sync::Arc<dyn Observability>, policy: std::sync::Arc<PolicyEngine>) -> Self {
        Self { client: runtime_network::HttpClient::new(), observability, policy }
    }
    pub async fn execute(&self, agent: &AgentIdentity, action: &str, url: &str) -> String {
        // CF-1 FIX: enforce policy BEFORE any network call
        let decision = self.policy.check(agent, action);
        match decision {
            runtime_policy::Decision::Deny { reason } => {
                self.observability.log_structured(
                    runtime_observability::LogLevel::Warn,
                    "http_adapter_denied",
                    &TraceContext::new(uuid::Uuid::nil(), None),
                    &[("action", action), ("reason", &reason)],
                );
                return format!("DENIED: {}", reason);
            }
            runtime_policy::Decision::Allow => {},
        }
        let res = self.client.get(url).await.unwrap_or_default();
        self.observability.log_structured(
            runtime_observability::LogLevel::Info,
            "http_adapter_executed",
            &TraceContext::new(uuid::Uuid::nil(), None),
            &[("action", action), ("url", url)],
        );
        res
    }
}
