//! Phase 1 §6 regression tests — ReplayWriter JSONL monotonic sequence.
//!
//! These tests verify the CF-3 contract: ReplayWriter MUST write JSONL with
//! strictly increasing sequence numbers, and record_replay must return the
//! assigned sequence.

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use uuid::Uuid;

    // -------------------------------------------------------------------------
    // §6 Test: TraceObservability (which wraps ReplayWriter) emits monotonic
    // increasing sequence in JSONL via record_replay
    // -------------------------------------------------------------------------
    #[test]
    fn test_replay_jsonl_monotonic_and_metric() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("replay.jsonl");
        let obs: TraceObservability = TraceObservability::with_replay(path.clone());

        // Record two events and capture the returned sequences
        let seq1 = obs.record_replay(ReplayEvent {
            sequence: 0, // ignored; observability assigns its own via writer
            event_type: "event_a".into(),
            task_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            result_summary: "summary_a".into(),
            timestamp: chrono::Utc::now(),
        });

        let seq2 = obs.record_replay(ReplayEvent {
            sequence: 0,
            event_type: "event_b".into(),
            task_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            result_summary: "summary_b".into(),
            timestamp: chrono::Utc::now(),
        });

        // §6 / CF-3 contract: sequence must be strictly increasing
        assert!(
            seq2 > seq1,
            "record_replay must return increasing sequence: seq1={seq1}, seq2={seq2}"
        );

        // Read back the JSONL file and verify stored sequences
        let contents = std::fs::read_to_string(&path)
            .expect("replay.jsonl must exist after record_replay");

        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "expected 2 JSONL lines, got {}", lines.len());

        let e1: ReplayEvent = serde_json::from_str(lines[0])
            .expect("first line must be valid JSON ReplayEvent");
        let e2: ReplayEvent = serde_json::from_str(lines[1])
            .expect("second line must be valid JSON ReplayEvent");

        assert_eq!(
            e1.sequence, seq1,
            "stored sequence[0] must match returned seq1"
        );
        assert_eq!(
            e2.sequence, seq2,
            "stored sequence[1] must match returned seq2"
        );
        assert!(
            e2.sequence > e1.sequence,
            "stored sequences must be monotonically increasing"
        );

        // Verify event_type is preserved
        assert_eq!(e1.event_type, "event_a");
        assert_eq!(e2.event_type, "event_b");
    }
}
