//! G32 — Visual fallback adapter (Phase 6.3)
//!
//! Observable:
//! - registry preference order retained: HTTP > DOM > JS > MCP > Visual;
//! - visual-required actions select the `VisualAdapter`;
//! - a policy-denied visual execute never reaches the renderer backend;
//! - a missing renderer returns `AdapterResult::Unsupported` gracefully.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use runtime_auth::{AgentIdentity, HumanId};
use runtime_interaction::{
    AdapterDescriptor, AdapterKind, AdapterParams, AdapterRegistry, AdapterResult,
    InteractionAdapter, TaskInfo,
};
use runtime_observability::{Observability, TraceObservability};
use runtime_policy::{Capability, CapabilitySet, PolicyEngine, Scope};
use runtime_visual::{Renderer, VisualAdapter};

#[derive(Debug)]
struct MockAdapter {
    kind: AdapterKind,
}

#[async_trait]
impl InteractionAdapter for MockAdapter {
    fn descriptor(&self) -> AdapterDescriptor {
        AdapterDescriptor {
            kind: self.kind,
            handles: vec!["visual.render".into(), "screenshot".into()],
        }
    }

    async fn execute(
        &self,
        _agent: &AgentIdentity,
        _caps: &CapabilitySet,
        _info: &TaskInfo,
        _params: &AdapterParams,
    ) -> AdapterResult {
        AdapterResult::Success { response: format!("{:?}", self.kind), replay_sequence: 1 }
    }
}

fn identity() -> AgentIdentity {
    AgentIdentity::new(HumanId(uuid::Uuid::new_v4()))
}

fn policy_with(cap: &str) -> Arc<PolicyEngine> {
    Arc::new({
        let mut p = PolicyEngine::new();
        p.add_capability(cap);
        p
    })
}

fn caps_with(cap: &str) -> CapabilitySet {
    let mut caps = CapabilitySet::new();
    caps.grant(Capability::new(cap, Scope::All, None));
    caps
}

#[test]
fn registry_preference_order_retains_visual_last() {
    let order = AdapterKind::preference_order();
    assert_eq!(order[0], AdapterKind::Http);
    assert_eq!(order[1], AdapterKind::Dom);
    assert_eq!(order[2], AdapterKind::Js);
    assert_eq!(order[3], AdapterKind::Mcp);
    assert_eq!(order[4], AdapterKind::Visual);

    let obs: Arc<dyn Observability> = Arc::new(TraceObservability::without_replay());
    let policy = Arc::new(PolicyEngine::new());

    // All five kinds handle the same visual action; HTTP must win.
    let mut registry = AdapterRegistry::new();
    registry.register(Box::new(MockAdapter { kind: AdapterKind::Http }));
    registry.register(Box::new(MockAdapter { kind: AdapterKind::Dom }));
    registry.register(Box::new(MockAdapter { kind: AdapterKind::Js }));
    registry.register(Box::new(MockAdapter { kind: AdapterKind::Mcp }));
    registry.register(Box::new(VisualAdapter::new(obs, policy)));
    assert_eq!(
        registry.resolve("visual.render").unwrap().descriptor().kind,
        AdapterKind::Http,
        "HTTP must be preferred over the visual fallback"
    );
}

#[test]
fn visual_action_selects_visual_adapter() {
    let obs: Arc<dyn Observability> = Arc::new(TraceObservability::without_replay());
    let policy = Arc::new(PolicyEngine::new());
    let mut registry = AdapterRegistry::new();
    registry.register(Box::new(VisualAdapter::new(obs, policy)));

    let resolved = registry.resolve("visual.render").expect("visual adapter resolves");
    assert_eq!(resolved.descriptor().kind, AdapterKind::Visual);
    assert!(resolved.select("screenshot"));
    assert!(!resolved.select("http.get"));
    assert!(
        registry.resolve("http.get").is_none(),
        "visual fallback must not claim non-visual actions"
    );
}

#[derive(Debug)]
struct MockRenderer {
    called: Arc<AtomicBool>,
}

impl Renderer for MockRenderer {
    fn render(&self, _action: &str, _target: &str) -> Result<String, String> {
        self.called.store(true, Ordering::SeqCst);
        Ok("pixels".into())
    }
}

#[tokio::test]
async fn policy_denied_visual_execute_never_reaches_renderer() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let agent = identity();
        let info = TaskInfo::new(uuid::Uuid::new_v4(), agent.agent_id.0);
        let caps = CapabilitySet::new();
        let called = Arc::new(AtomicBool::new(false));
        let renderer = Arc::new(MockRenderer { called: called.clone() });
        let obs: Arc<dyn Observability> = Arc::new(TraceObservability::without_replay());
        let adapter = VisualAdapter::new(obs, policy_with("visual.render")).with_renderer(renderer);

        let params = AdapterParams::Visual {
            action: "visual.render".into(),
            target: "https://example.com".into(),
        };
        let result = adapter.execute(&agent, &caps, &info, &params).await;
        assert!(result.is_denied(), "expected Denied, got {:?}", result);
        assert!(
            !called.load(Ordering::SeqCst),
            "renderer must never be invoked when policy denies"
        );
    })
    .await
    .expect("policy-denied visual test must not hang");
}

#[tokio::test]
async fn unsupported_renderer_returns_unsupported() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let agent = identity();
        let info = TaskInfo::new(uuid::Uuid::new_v4(), agent.agent_id.0);
        let caps = caps_with("visual.render");
        let obs: Arc<dyn Observability> = Arc::new(TraceObservability::without_replay());
        let adapter = VisualAdapter::new(obs, policy_with("visual.render"));
        assert!(!adapter.has_renderer());

        let params = AdapterParams::Visual {
            action: "visual.render".into(),
            target: "https://example.com".into(),
        };
        let result = adapter.execute(&agent, &caps, &info, &params).await;
        assert!(
            result.is_unsupported(),
            "expected Unsupported when no renderer is configured, got {:?}",
            result
        );
    })
    .await
    .expect("unsupported renderer test must not hang");
}

#[tokio::test]
async fn allowed_visual_execute_renders() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let agent = identity();
        let info = TaskInfo::new(uuid::Uuid::new_v4(), agent.agent_id.0);
        let caps = caps_with("visual.render");
        let called = Arc::new(AtomicBool::new(false));
        let renderer = Arc::new(MockRenderer { called: called.clone() });
        let obs: Arc<dyn Observability> = Arc::new(TraceObservability::without_replay());
        let adapter = VisualAdapter::new(obs, policy_with("visual.render")).with_renderer(renderer);

        let params = AdapterParams::Visual {
            action: "visual.render".into(),
            target: "https://example.com".into(),
        };
        let result = adapter.execute(&agent, &caps, &info, &params).await;
        assert!(result.is_success(), "expected Success, got {:?}", result);
        assert!(called.load(Ordering::SeqCst), "renderer must be invoked when allowed");
    })
    .await
    .expect("allowed visual test must not hang");
}