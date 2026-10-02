//! G26 — Sandbox Kill Mechanisms Exercised Directly (Priority P0)
//!
//! Observable: the `runtime-sandbox` primitives that back resource kill
//! enforcement fire on their own, independent of the scheduler. The
//! `Watchdog` cancels its token when the wall budget elapses and stays armed
//! for an unlimited (zero) budget; `OOMContainment` cancels its token when the
//! polled memory usage exceeds the cap and stays armed for an unlimited
//! (zero) cap.

use std::time::Duration;

use runtime_sandbox::{OOMContainment, Watchdog};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn watchdog_cancels_token_on_wall_budget_elapse() {
    let token = CancellationToken::new();
    let watchdog = Watchdog::arm(20, token.clone());
    tokio::time::timeout(Duration::from_secs(1), token.cancelled())
        .await
        .expect("watchdog must cancel the watched token within its budget");
    drop(watchdog);
}

#[tokio::test]
async fn watchdog_disarm_prevents_fire() {
    let token = CancellationToken::new();
    let watchdog = Watchdog::arm(10, token.clone());
    watchdog.disarm();
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(!token.is_cancelled(), "disarmed watchdog must not fire");
}

#[tokio::test]
async fn watchdog_zero_budget_is_unlimited() {
    let token = CancellationToken::new();
    let watchdog = Watchdog::arm(0, token.clone());
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(!token.is_cancelled(), "zero wall budget means unlimited");
    drop(watchdog);
}

#[tokio::test]
async fn oom_containment_cancels_token_when_memory_exceeds_cap() {
    let token = CancellationToken::new();
    let containment = OOMContainment::arm(
        100,
        Duration::from_millis(5),
        token.clone(),
        || 1_000,
    );
    tokio::time::timeout(Duration::from_secs(1), token.cancelled())
        .await
        .expect("OOM containment must cancel the token when memory exceeds the cap");
    drop(containment);
}

#[tokio::test]
async fn oom_containment_zero_cap_is_unlimited() {
    let token = CancellationToken::new();
    let containment = OOMContainment::arm(
        0,
        Duration::from_millis(5),
        token.clone(),
        || 1_000_000,
    );
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(!token.is_cancelled(), "zero memory cap means unlimited");
    drop(containment);
}
