//! G4 — Execution State Transitions (P0)
//!
//! Observable: ExecutionRecord accepts valid transitions and rejects
//! invalid ones (e.g., Created -> Completed, Running -> Created).

use runtime_core::execution::{ExecutionRecord, ExecutionState};
use uuid::Uuid;

#[tokio::test]
async fn state_transitions_valid() {
    let record = ExecutionRecord::new(Uuid::new_v4());
    // Initial: Created.
    assert_eq!(
        record.state().await,
        ExecutionState::Created,
        "new record should start in Created state"
    );

    // Valid: Created -> Queued.
    assert!(
        record.transition(ExecutionState::Queued).await.is_ok(),
        "Created -> Queued should be a valid transition"
    );

    // Invalid: Queued -> Created (no backward transitions).
    assert!(
        record.transition(ExecutionState::Created).await.is_err(),
        "Queued -> Created must be rejected"
    );
}

#[tokio::test]
async fn state_rejects_skipping() {
    let record = ExecutionRecord::new(Uuid::new_v4());
    // Invalid: Created -> Completed (must go through Queued and Running).
    assert!(
        record.transition(ExecutionState::Completed).await.is_err(),
        "Created -> Completed must be rejected"
    );
}
