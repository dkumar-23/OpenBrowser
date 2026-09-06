#[cfg(test)]
mod tests {
    
    use std::sync::Arc;
    use runtime_observability::{TraceContext, LogLevel, ReplayEvent, Observability, TraceObservability};
    use uuid::Uuid;

    #[test]
    fn trace_context_has_all_ids() {
        let agent = Uuid::new_v4();
        let ctx = TraceContext::new(agent, None);
        assert_eq!(ctx.agent_id, agent);
        assert_ne!(ctx.task_id, ctx.request_id);
        assert!(ctx.delegation_id.is_none());
    }

    #[test]
    fn trace_context_with_delegation() {
        let agent = Uuid::new_v4();
        let delegation = Uuid::new_v4();
        let ctx = TraceContext::new(agent, Some(delegation));
        assert_eq!(ctx.delegation_id, Some(delegation));
    }

    #[test]
    fn replay_event_serializes() {
        let e = ReplayEvent {
            sequence: 1,
            event_type: "policy_check".to_string(),
            task_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            result_summary: "allow".to_string(),
            timestamp: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"sequence\":1"));
    }

    #[test]
    fn observability_is_object_safe() {
        let obs: Arc<dyn Observability> = Arc::new(TraceObservability::default());
        let ctx = TraceContext::new(Uuid::new_v4(), None);
        obs.log_structured(LogLevel::Info, "test", &ctx, &[]);
        obs.trace_span("test_span", &ctx);
        obs.metric("test_metric", 1.0, &[]);
        let _ = obs.record_replay(ReplayEvent {
            sequence: 0,
            event_type: "test".to_string(),
            task_id: ctx.task_id,
            agent_id: ctx.agent_id,
            result_summary: "ok".to_string(),
            timestamp: chrono::Utc::now(),
        });
    }
}
