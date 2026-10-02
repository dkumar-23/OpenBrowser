// runtime-visual: visual fallback adapter implementing InteractionAdapter.
//
// Per context.md §8 (rendering optional) and §11 (unified interaction API):
// the visual path is a fallback only, selected when a task explicitly requires
// rendering. Policy is enforced BEFORE any renderer is touched (R1/R2), and a
// missing renderer fails gracefully with `AdapterResult::Unsupported`.

use async_trait::async_trait;
use std::sync::Arc;

use runtime_auth::AgentIdentity;
use runtime_interaction::{
    AdapterDescriptor, AdapterKind, AdapterParams, AdapterResult, InteractionAdapter, TaskInfo,
};
use runtime_observability::{Observability, ReplayEvent};
use runtime_policy::{CapabilitySet, Decision, PolicyEngine};

/// Explicit visual-required actions handled by the fallback adapter.
pub const VISUAL_ACTIONS: [&str; 2] = ["visual.render", "screenshot"];

/// Optional rendering backend. Implementations are injected; the adapter never
/// depends on a concrete renderer.
pub trait Renderer: Send + Sync + std::fmt::Debug {
    /// Render or screenshot `target` for `action`. Returns the backend output.
    fn render(&self, action: &str, target: &str) -> Result<String, String>;
}

/// Visual fallback adapter. Lowest-preference adapter in the registry.
#[derive(Debug)]
pub struct VisualAdapter {
    observability: Arc<dyn Observability>,
    policy: Arc<PolicyEngine>,
    renderer: Option<Arc<dyn Renderer>>,
}

impl VisualAdapter {
    /// Construct without a renderer. Visual execution returns `Unsupported`.
    pub fn new(observability: Arc<dyn Observability>, policy: Arc<PolicyEngine>) -> Self {
        Self { observability, policy, renderer: None }
    }

    /// Attach a rendering backend.
    pub fn with_renderer(mut self, renderer: Arc<dyn Renderer>) -> Self {
        self.renderer = Some(renderer);
        self
    }

    /// Whether a renderer is configured.
    pub fn has_renderer(&self) -> bool {
        self.renderer.is_some()
    }
}

fn capability_for(action: &str) -> &str {
    if action == "screenshot" {
        "screenshot"
    } else {
        "visual.render"
    }
}

#[async_trait]
impl InteractionAdapter for VisualAdapter {
    fn descriptor(&self) -> AdapterDescriptor {
        AdapterDescriptor {
            kind: AdapterKind::Visual,
            handles: VISUAL_ACTIONS.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Only visual-required actions are selected by the fallback.
    fn select(&self, action: &str) -> bool {
        VISUAL_ACTIONS.contains(&action)
    }

    async fn execute(
        &self,
        agent: &AgentIdentity,
        caps: &CapabilitySet,
        info: &TaskInfo,
        params: &AdapterParams,
    ) -> AdapterResult {
        let (action, target) = match params {
            AdapterParams::Visual { action, target } => (action.clone(), target.clone()),
            _ => {
                return AdapterResult::Error {
                    message: format!("VisualAdapter expects AdapterParams::Visual, got {:?}", params),
                    replay_sequence: 0,
                }
            }
        };

        // Policy enforcement happens BEFORE any renderer is touched (R1/R2).
        let capability = capability_for(&action);
        match self.policy.check_with_caps(agent, caps, capability) {
            Decision::Deny { reason } => {
                let event = ReplayEvent {
                    sequence: 0,
                    event_type: "capability_denied".into(),
                    task_id: info.task_id,
                    agent_id: agent.agent_id.0,
                    result_summary: reason.clone(),
                    timestamp: chrono::Utc::now(),
                };
                let seq = self.observability.record_replay(event);
                self.observability
                    .metric("visual_policy_denied", 1.0, &[("capability", capability)]);
                AdapterResult::Denied { reason, replay_sequence: seq }
            }
            Decision::Allow => match &self.renderer {
                None => {
                    let message =
                        format!("visual renderer not configured for action '{}'", action);
                    let event = ReplayEvent {
                        sequence: 0,
                        event_type: "visual_unsupported".into(),
                        task_id: info.task_id,
                        agent_id: agent.agent_id.0,
                        result_summary: message.clone(),
                        timestamp: chrono::Utc::now(),
                    };
                    let seq = self.observability.record_replay(event);
                    self.observability
                        .metric("visual_unsupported", 1.0, &[("action", &action)]);
                    AdapterResult::Unsupported { message, replay_sequence: seq }
                }
                Some(renderer) => match renderer.render(&action, &target) {
                    Ok(output) => {
                        let event = ReplayEvent {
                            sequence: 0,
                            event_type: "visual_rendered".into(),
                            task_id: info.task_id,
                            agent_id: agent.agent_id.0,
                            result_summary: format!("{} -> {} bytes", action, output.len()),
                            timestamp: chrono::Utc::now(),
                        };
                        let seq = self.observability.record_replay(event);
                        self.observability
                            .metric("visual_rendered", 1.0, &[("action", &action)]);
                        AdapterResult::Success { response: output, replay_sequence: seq }
                    }
                    Err(message) => {
                        let event = ReplayEvent {
                            sequence: 0,
                            event_type: "visual_error".into(),
                            task_id: info.task_id,
                            agent_id: agent.agent_id.0,
                            result_summary: message.clone(),
                            timestamp: chrono::Utc::now(),
                        };
                        let seq = self.observability.record_replay(event);
                        self.observability.metric("visual_error", 1.0, &[]);
                        AdapterResult::Error { message, replay_sequence: seq }
                    }
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_auth::HumanId;
    use runtime_policy::{Capability, Scope};

    #[derive(Debug)]
    struct CountingRenderer {
        calls: std::sync::atomic::AtomicUsize,
    }

    impl Renderer for CountingRenderer {
        fn render(&self, action: &str, target: &str) -> Result<String, String> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(format!("{}:{}", action, target))
        }
    }

    fn identity() -> AgentIdentity {
        AgentIdentity::new(HumanId(uuid::Uuid::new_v4()))
    }

    #[test]
    fn test_visual_adapter_selects_only_visual_actions() {
        let obs = Arc::new(runtime_observability::TraceObservability::without_replay());
        let policy = Arc::new(PolicyEngine::new());
        let adapter = VisualAdapter::new(obs, policy);
        assert!(adapter.select("visual.render"));
        assert!(adapter.select("screenshot"));
        assert!(!adapter.select("http.get"));
        assert!(!adapter.select("extract_page"));
    }

    #[tokio::test]
    async fn test_visual_adapter_without_renderer_is_unsupported() {
        let agent = identity();
        let info = TaskInfo::new(uuid::Uuid::new_v4(), agent.agent_id.0);
        let mut caps = CapabilitySet::new();
        caps.grant(Capability::new("visual.render", Scope::All, None));
        let obs = Arc::new(runtime_observability::TraceObservability::without_replay());
        let policy = Arc::new({
            let mut p = PolicyEngine::new();
            p.add_capability("visual.render");
            p
        });
        let adapter = VisualAdapter::new(obs, policy);
        let params = AdapterParams::Visual { action: "visual.render".into(), target: "https://x".into() };
        let result = adapter.execute(&agent, &caps, &info, &params).await;
        assert!(result.is_unsupported(), "expected Unsupported, got {:?}", result);
    }

    #[tokio::test]
    async fn test_visual_adapter_policy_denied_never_renders() {
        let agent = identity();
        let info = TaskInfo::new(uuid::Uuid::new_v4(), agent.agent_id.0);
        let caps = CapabilitySet::new();
        let renderer = Arc::new(CountingRenderer { calls: std::sync::atomic::AtomicUsize::new(0) });
        let obs = Arc::new(runtime_observability::TraceObservability::without_replay());
        let policy = Arc::new({
            let mut p = PolicyEngine::new();
            p.add_capability("visual.render");
            p
        });
        let adapter = VisualAdapter::new(obs, policy).with_renderer(renderer.clone());
        let params = AdapterParams::Visual { action: "visual.render".into(), target: "https://x".into() };
        let result = adapter.execute(&agent, &caps, &info, &params).await;
        assert!(result.is_denied(), "expected Denied, got {:?}", result);
        assert_eq!(
            renderer.calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "renderer must not be called when policy denies"
        );
    }

    #[tokio::test]
    async fn test_visual_adapter_renders_when_allowed() {
        let agent = identity();
        let info = TaskInfo::new(uuid::Uuid::new_v4(), agent.agent_id.0);
        let mut caps = CapabilitySet::new();
        caps.grant(Capability::new("visual.render", Scope::All, None));
        let renderer = Arc::new(CountingRenderer { calls: std::sync::atomic::AtomicUsize::new(0) });
        let obs = Arc::new(runtime_observability::TraceObservability::without_replay());
        let policy = Arc::new({
            let mut p = PolicyEngine::new();
            p.add_capability("visual.render");
            p
        });
        let adapter = VisualAdapter::new(obs, policy).with_renderer(renderer.clone());
        let params = AdapterParams::Visual { action: "visual.render".into(), target: "https://x".into() };
        let result = adapter.execute(&agent, &caps, &info, &params).await;
        assert!(result.is_success(), "expected Success, got {:?}", result);
        assert_eq!(renderer.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}