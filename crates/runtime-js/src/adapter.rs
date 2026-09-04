// runtime-js/src/adapter.rs: JS adapter implementing InteractionAdapter (Phase 3.1)
//
// Per context.md §11: agent should not need to understand mechanism.
// This adapter provides JavaScript execution through the unified interaction API.

use async_trait::async_trait;
use runtime_interaction::{
    InteractionAdapter, AdapterDescriptor, AdapterKind, AdapterParams, AdapterResult, TaskInfo,
};
use runtime_auth::AgentIdentity;
use runtime_policy::{PolicyEngine, CapabilitySet};
use runtime_observability::{Observability, ReplayEvent};
use std::sync::Arc;

/// JS adapter — executes JavaScript source through the InteractionAdapter trait.
#[derive(Debug)]
pub struct JsAdapter {
    observability: Arc<dyn Observability>,
    policy: Arc<PolicyEngine>,
}

impl JsAdapter {
    pub fn new(observability: Arc<dyn Observability>, policy: Arc<PolicyEngine>) -> Self {
        Self { observability, policy }
    }
}

#[async_trait]
impl InteractionAdapter for JsAdapter {
    fn descriptor(&self) -> AdapterDescriptor {
        AdapterDescriptor {
            kind: AdapterKind::Js,
            handles: vec![
                "schedule".into(),
                "js.execute".into(),
                "js.compile".into(),
            ],
        }
    }

    async fn execute(
        &self,
        agent: &AgentIdentity,
        caps: &CapabilitySet,
        info: &TaskInfo,
        params: &AdapterParams,
    ) -> AdapterResult {
        let source = match params {
            AdapterParams::Js { source } => source.clone(),
            _ => return AdapterResult::Error {
                message: format!("JsAdapter expects AdapterParams::Js, got {:?}", params),
                replay_sequence: 0,
            },
        };

        // Policy enforcement (CF-1 + CF-2)
        let decision = self.policy.check_with_caps(agent, caps, "schedule");
        match decision {
            runtime_policy::Decision::Deny { reason } => {
                let event = ReplayEvent {
                    sequence: 0,
                    event_type: "capability_denied".into(),
                    task_id: info.task_id,
                    agent_id: agent.agent_id.0,
                    result_summary: reason.clone(),
                    timestamp: chrono::Utc::now(),
                };
                let seq = self.observability.record_replay(event);
                self.observability.metric("js_policy_denied", 1.0, &[("capability", "schedule")]);
                AdapterResult::Denied { reason, replay_sequence: seq }
            }
            runtime_policy::Decision::Allow => {
                // In a real implementation, this would compile + execute via V8/Boa.
                // For Phase 3, we return a simulated execution result.
                let event = ReplayEvent {
                    sequence: 0,
                    event_type: "js_executed".into(),
                    task_id: info.task_id,
                    agent_id: agent.agent_id.0,
                    result_summary: format!("executed {} chars of JS source", source.len()),
                    timestamp: chrono::Utc::now(),
                };
                let seq = self.observability.record_replay(event);
                self.observability.metric("js_executed", 1.0, &[]);
                AdapterResult::Success {
                    response: format!("{{\"status\":\"ok\",\"source_length\":{},\"note\":\"JS adapter executed; real engine requires V8/Boa integration\"}}", source.len()),
                    replay_sequence: seq,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_auth::{AgentIdentity, HumanId};
    use runtime_policy::CapabilitySet;

    fn make_identity() -> AgentIdentity {
        AgentIdentity::new(HumanId(uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn test_js_adapter_policy_denied() {
        let agent = make_identity();
        let info = runtime_interaction::TaskInfo::new(uuid::Uuid::new_v4(), agent.agent_id.0);
        let caps = CapabilitySet::new();
        let obs = Arc::new(runtime_observability::TraceObservability::without_replay());
        let policy = Arc::new({
            let mut p = runtime_policy::PolicyEngine::new();
            p.add_capability("schedule");
            p
        });
        let adapter = JsAdapter::new(obs, policy);
        let params = AdapterParams::Js { source: "console.log('hello')".into() };
        let result = adapter.execute(&agent, &caps, &info, &params).await;
        assert!(matches!(result, AdapterResult::Denied { .. }));
    }

    #[tokio::test]
    async fn test_js_adapter_policy_allowed() {
        let agent = make_identity();
        let info = runtime_interaction::TaskInfo::new(uuid::Uuid::new_v4(), agent.agent_id.0);
        let mut caps = CapabilitySet::new();
        caps.grant(runtime_policy::Capability::new("schedule", runtime_policy::Scope::Read, None));
        let obs = Arc::new(runtime_observability::TraceObservability::without_replay());
        let policy = Arc::new({
            let mut p = runtime_policy::PolicyEngine::new();
            p.add_capability("schedule");
            p
        });
        let adapter = JsAdapter::new(obs, policy);
        let params = AdapterParams::Js { source: "setTimeout(()=>{}, 100)".into() };
        let result = adapter.execute(&agent, &caps, &info, &params).await;
        assert!(matches!(result, AdapterResult::Success { .. }));
    }

    #[test]
    fn test_js_adapter_descriptor() {
        let obs = Arc::new(runtime_observability::TraceObservability::without_replay());
        let policy = Arc::new(runtime_policy::PolicyEngine::new());
        let adapter = JsAdapter::new(obs, policy);
        let desc = adapter.descriptor();
        assert_eq!(desc.kind, AdapterKind::Js);
        assert!(adapter.handles("schedule"));
        assert!(!adapter.handles("http.get"));
    }
}
