use async_trait::async_trait;
use runtime_observability::{Observability, ReplayEvent};
use runtime_auth::AgentIdentity;
use runtime_policy::{PolicyEngine, CapabilitySet};
use runtime_interaction::{
    InteractionAdapter, AdapterDescriptor, AdapterKind, AdapterParams, AdapterResult, TaskInfo
};

pub struct HttpAdapter {
    client: runtime_network::HttpClient,
    observability: std::sync::Arc<dyn Observability>,
    policy: std::sync::Arc<PolicyEngine>,
}

impl HttpAdapter {
    pub fn new(observability: std::sync::Arc<dyn Observability>, policy: std::sync::Arc<PolicyEngine>) -> Self {
        Self { client: runtime_network::HttpClient::new(), observability, policy }
    }
}

#[async_trait]
impl InteractionAdapter for HttpAdapter {
    fn descriptor(&self) -> AdapterDescriptor {
        AdapterDescriptor { kind: AdapterKind::Http, handles: vec!["http.get".into()] }
    }

    async fn execute(
        &self,
        agent: &AgentIdentity,
        caps: &CapabilitySet,
        ctx: &TaskInfo,
        params: &AdapterParams,
    ) -> AdapterResult {
        let url = match params {
            AdapterParams::Http { url, .. } => url.clone(),
        };
        // Policy check with capability set (CF-1 + CF-2)
        let decision = self.policy.check_with_caps(agent, caps, "http.get");
        match decision {
            runtime_policy::Decision::Deny { reason } => {
                let event = ReplayEvent {
                    sequence: 0,
                    event_type: "policy_denied".into(),
                    task_id: ctx.task_id,
                    agent_id: agent.agent_id.0,
                    result_summary: reason.clone(),
                    timestamp: chrono::Utc::now(),
                };
                let replay_seq = self.observability.record_replay(event);
                self.observability.metric("policy_denied", 1.0, &[("action", "http.get")]);
                AdapterResult::Denied { reason, replay_sequence: replay_seq }
            }
            runtime_policy::Decision::Allow => {
                let res = self.client.get(&url).await.unwrap_or_default();
                let event = ReplayEvent {
                    sequence: 0,
                    event_type: "http_executed".into(),
                    task_id: ctx.task_id,
                    agent_id: agent.agent_id.0,
                    result_summary: "success".into(),
                    timestamp: chrono::Utc::now(),
                };
                let replay_seq = self.observability.record_replay(event);
                self.observability.metric("http_executed", 1.0, &[("action", "http.get")]);
                AdapterResult::Success { response: res, replay_sequence: replay_seq }
            }
        }
    }
}

mod tests;
