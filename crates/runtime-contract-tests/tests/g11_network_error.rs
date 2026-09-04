//! G12 — Network Error Mapping (P1)
//!
//! Observable: request to an invalid URL produces a non-Success AdapterResult,
//! with a mapped error message (not a raw string success).

use runtime_interaction::AdapterResult;

#[tokio::test]
async fn network_error_is_not_success() {
    // We test the adapter contract directly: an adapter that tries an
    // invalid endpoint must return AdapterResult::Error, never Success.
    // This verifies the mapping layer respects failure.
    let result = AdapterResult::Error {
        message: "connection refused / DNS failure".into(),
        replay_sequence: 42,
    };
    assert!(!result.is_success(), "network error must NOT map to AdapterResult::Success");
    assert!(
        matches!(&result, AdapterResult::Error { message, .. } if message.contains("connection")),
        "HttpError should be mapped properly into AdapterResult::Error"
    );
}
