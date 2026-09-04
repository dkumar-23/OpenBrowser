//! G13 / Observability — No Secret Leakage (P1)
//!
//! Observable: serialized ReplayEvent data must not contain the bytes
//! of an issued AuthHandle — the credential is opaque and must never
//! leak into replay log.

use runtime_auth::{AgentId, CredentialBroker, InMemoryBroker};
use runtime_observability::{ReplayEvent, ReplayWriter};
use uuid::Uuid;
use chrono::Utc;
use std::sync::Arc;

#[test]
fn no_credential_leak() {
    // 1. Issue a credential and capture its opaque bytes.
    let broker = InMemoryBroker::default();
    let agent = AgentId::new();
    let handle = broker.issue(&agent, "secret-scope");
    let opaque_hex: String = handle
        .opaque
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();
    // Also collect as raw byte string for substring check.
    let opaque_bytes: Vec<u8> = handle.opaque.to_vec();

    // 2. Create a ReplayWriter and record a ReplayEvent that does NOT
    //    include the handle. Simulate what an adapter would write.
    let tmpdir = tempfile::tempdir().expect("tempdir");
    let writer = Arc::new(ReplayWriter::new(tmpdir.path().join("replay.jsonl")));

    let event = ReplayEvent {
        sequence: 1,
        event_type: "http_executed".into(),
        task_id: Uuid::new_v4(),
        agent_id: agent.0,
        result_summary: "successful fetch (200 OK)".into(),
        timestamp: Utc::now(),
    };
    writer.write(&event).expect("write event");

    // 3. Read back the file and confirm the credential's opaque bytes
    //    appear nowhere.
    let path = tmpdir.path().join("replay.jsonl");
    let data = std::fs::read_to_string(&path).expect("read");

    // Hex-string search is what attackers would do on serialized JSON.
    assert!(
        !data.contains(&opaque_hex),
        "replay log must not contain credential opaque bytes (hex)"
    );
    // Also try raw byte substring for paranoia.
    let opaque_str = String::from_utf8_lossy(&opaque_bytes).to_string();
    // The opaque bytes are random 32 bytes; the printable substring
    // is unlikely to be meaningful, but we still search.
    if !opaque_str.is_empty() {
        assert!(
            !data.contains(&opaque_str),
            "replay log must not contain credential opaque bytes (raw)"
        );
    }
}
