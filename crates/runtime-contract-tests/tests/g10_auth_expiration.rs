//! G9 — Auth Expiration (P1)
//!
//! Observable: handle whose `expires_at` is in the past must fail
//! validation. We exercise this via the `Capability::is_expired` path
//! which is the contract for time-based capability expiry.

use runtime_policy::{Capability, Scope};

#[test]
fn auth_expiration_works() {
    // Past-expiration capability is treated as missing.
    let mut cap = Capability::new("test_scope", Scope::All, Some(-3600));
    // Force expiration: even if ttl=-3600 works, set explicit past.
    cap.expiration = Some(chrono::Utc::now() - chrono::Duration::seconds(1));
    assert!(cap.is_expired(), "capability with past expiration must be expired");
    assert!(!is_cap_effective(&cap), "expired capability must not be effective");
}

fn is_cap_effective(cap: &Capability) -> bool {
    !cap.is_expired()
}
