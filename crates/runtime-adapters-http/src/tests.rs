//! Phase 1 §6 regression tests — HttpAdapter policy gate.
//!
//! These tests verify the CF-1 contract: HttpAdapter MUST deny without
//! CapabilitySet and allow with CapabilitySet, returning AdapterResult (not String).

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use crate::HttpAdapter;
    use runtime_auth::{AgentIdentity, HumanId};
    use runtime_interaction::{InteractionAdapter, AdapterParams, AdapterResult, TaskInfo};
    use runtime_observability::TraceObservability;
    use runtime_policy::{PolicyEngine, CapabilitySet, Capability, Scope};
    use uuid::Uuid;

    fn make_agent() -> AgentIdentity {
        AgentIdentity::new(HumanId::default())
    }

    fn make_caps_with(action: &str) -> CapabilitySet {
        let mut caps = CapabilitySet::new();
        caps.grant(Capability::new(action, Scope::All, Some(3600)));
        caps
    }

    fn make_policy_with(action: &str) -> Arc<PolicyEngine> {
        let mut p = PolicyEngine::new();
        p.add_capability(action);
        Arc::new(p)
    }

    fn make_task_info() -> TaskInfo {
        TaskInfo { task_id: Uuid::new_v4(), agent_id: Uuid::new_v4() }
    }

    fn make_params() -> AdapterParams {
        AdapterParams::Http { url: "http://example.com".into(), method: None }
    }

    // -------------------------------------------------------------------------
    // §6 Test 1: agent WITHOUT CapabilitySet → AdapterResult::Denied
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn test_deny_no_network() {
        let tmp = tempfile::TempDir::new().unwrap();
        let obs = Arc::new(TraceObservability::with_replay(tmp.path().join("replay.jsonl")));
        let policy = make_policy_with("http.get");
        let adapter = HttpAdapter::new(obs.clone(), policy);
        let agent = make_agent();
        let caps = CapabilitySet::new(); // NO capabilities granted
        let ctx = make_task_info();
        let params = make_params();

        let result = adapter.execute(&agent, &caps, &ctx, &params).await;

        // §6 contract: must return Denied, not Success or Error
        let seq = match &result {
            AdapterResult::Denied { reason, replay_sequence } => {
                assert!(!reason.is_empty(), "denial must include a reason");
                *replay_sequence
            }
            AdapterResult::Success { .. } => {
                panic!("agent without CapabilitySet should NOT be allowed — got Success");
            }
            AdapterResult::Error { .. } => {
                panic!("unexpected Error result: {:?}", result);
            }
        };

        // §6: replay event must have been recorded (sequence is assigned by writer)
        // Verify JSONL contains a policy_denied event
        let replay_path = tmp.path().join("replay.jsonl");
        let contents = std::fs::read_to_string(&replay_path)
            .expect("replay.jsonl must exist after denied execute");
        assert!(
            contents.contains("policy_denied"),
            "replay JSONL must contain 'policy_denied' event\ngot: {contents}"
        );
        // Verify sequence is monotonically present and JSON-parsable
        let events: Vec<serde_json::Value> = contents
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        assert_eq!(events.len(), 1, "exactly one replay event expected, got {}", events.len());
        assert!(events[0]["sequence"].is_number(), "replay event must have numeric sequence");
    }

    // -------------------------------------------------------------------------
    // §6 Test 2: agent WITH CapabilitySet → AdapterResult::Success
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn test_allow_with_cap() {
        let tmp = tempfile::TempDir::new().unwrap();
        let obs = Arc::new(TraceObservability::with_replay(tmp.path().join("replay.jsonl")));
        let policy = make_policy_with("http.get");
        let adapter = HttpAdapter::new(obs.clone(), policy);
        let agent = make_agent();
        let caps = make_caps_with("http.get"); // Capability granted
        let ctx = make_task_info();
        let params = make_params();

        let result = adapter.execute(&agent, &caps, &ctx, &params).await;

        // §6 contract: must return Success with a response
        let seq = match &result {
            AdapterResult::Success { response, replay_sequence } => {
                assert!(
                    !response.is_empty(),
                    "success response must be non-empty, got: {response}"
                );
                *replay_sequence
            }
            AdapterResult::Denied { reason, .. } => {
                panic!(
                    "agent WITH CapabilitySet('http.get') should be allowed — got Denied: {reason}"
                );
            }
            AdapterResult::Error { message, .. } => {
                panic!("unexpected Error result: {message}");
            }
        };

        // §6: replay event must have been recorded
        // Verify JSONL contains an http_executed event
        let replay_path = tmp.path().join("replay.jsonl");
        let contents = std::fs::read_to_string(&replay_path)
            .expect("replay.jsonl must exist after successful execute");
        assert!(
            contents.contains("http_executed"),
            "replay JSONL must contain 'http_executed' event\ngot: {contents}"
        );
        let events: Vec<serde_json::Value> = contents
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        assert_eq!(events.len(), 1, "exactly one replay event expected, got {}", events.len());
        assert!(events[0]["sequence"].is_number(), "replay event must have numeric sequence");
    }
}
