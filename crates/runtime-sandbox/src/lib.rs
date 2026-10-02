use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Resource limits. A quota field of 0 means the corresponding limit is
/// **unlimited** for every dimension: `WorkerGuard::enforce` skips a
/// zero-valued quota field, and the active mechanisms below (`Watchdog`,
/// `OOMContainment`) treat 0 as "unlimited" and disarm themselves. Nonzero
/// limits are enforced as ceiling values (usage equal to the limit passes;
/// usage above it breaches).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourceQuota {
    pub max_memory_bytes: u64,
    pub max_cpu_ms: u64,
    pub max_wall_ms: u64,
    pub max_network_bytes: u64,
    pub max_requests: u32,
}

/// Current resource usage counters tracked per-worker for enforcement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourceUsage {
    pub memory_bytes: u64,
    pub cpu_ms: u64,
    pub wall_ms: u64,
    pub network_bytes: u64,
    pub requests: u32,
}

#[derive(Clone, Debug)]
pub struct WorkerGuard {
    pub quota: ResourceQuota,
    pub usage: ResourceUsage,
}

impl WorkerGuard {
    pub fn new(quota: ResourceQuota) -> Self {
        Self {
            quota,
            usage: ResourceUsage::default(),
        }
    }

    /// Apply usage delta (saturating) to the tracked counters.
    pub fn add_usage(&mut self, delta: ResourceUsage) {
        self.usage.memory_bytes = self.usage.memory_bytes.saturating_add(delta.memory_bytes);
        self.usage.cpu_ms = self.usage.cpu_ms.saturating_add(delta.cpu_ms);
        self.usage.wall_ms = self.usage.wall_ms.saturating_add(delta.wall_ms);
        self.usage.network_bytes = self.usage.network_bytes.saturating_add(delta.network_bytes);
        self.usage.requests = self.usage.requests.saturating_add(delta.requests);
    }

    /// Enforce quota: return false if any measured usage exceeds its limit.
    pub fn enforce(&self) -> bool {
        self.breached_dimension().is_none()
    }

    /// The first quota dimension currently breached, if any. A zero-valued
    /// quota field is unlimited and is skipped.
    pub fn breached_dimension(&self) -> Option<&'static str> {
        if self.quota.max_memory_bytes != 0 && self.usage.memory_bytes > self.quota.max_memory_bytes { return Some("memory"); }
        if self.quota.max_cpu_ms != 0 && self.usage.cpu_ms > self.quota.max_cpu_ms { return Some("cpu"); }
        if self.quota.max_wall_ms != 0 && self.usage.wall_ms > self.quota.max_wall_ms { return Some("wall"); }
        if self.quota.max_network_bytes != 0 && self.usage.network_bytes > self.quota.max_network_bytes { return Some("network"); }
        if self.quota.max_requests != 0 && self.usage.requests > self.quota.max_requests { return Some("requests"); }
        None
    }
}

/// Active wall-clock watchdog: a spawned tokio task that fires a
/// CancellationToken when the wall budget elapses. The watched worker is
/// identified by the token it was armed with. `max_wall_ms == 0` means
/// unlimited — the watchdog disarms itself immediately.
#[derive(Debug)]
pub struct Watchdog {
    handle: tokio::task::JoinHandle<()>,
    disarm: CancellationToken,
}

impl Watchdog {
    /// Arm a watchdog for `max_wall_ms`: when the budget elapses, `token`
    /// is cancelled (hard-cancel path for the running worker). Dropping the
    /// returned guard disarms the watchdog without firing.
    pub fn arm(max_wall_ms: u64, token: CancellationToken) -> Self {
        let disarm = CancellationToken::new();
        let disarmer = disarm.clone();
        let watched = token.clone();
        let handle = if max_wall_ms == 0 {
            disarmer.cancel();
            tokio::spawn(async {})
        } else {
            tokio::spawn(async move {
                tokio::select! {
                    _ = disarmer.cancelled() => {}
                    _ = tokio::time::sleep(Duration::from_millis(max_wall_ms)) => {
                        watched.cancel();
                    }
                }
            })
        };
        Self { handle, disarm }
    }

    /// Disarm the watchdog without firing it (call on normal completion).
    pub fn disarm(self) {
        self.disarm.cancel();
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.disarm.cancel();
        self.handle.abort();
    }
}

/// OOM containment: a spawned tokio task that polls a usage provider and
/// hard-cancels a token when the memory cap is breached. `cap == 0` means
/// unlimited — containment disarms itself immediately.
#[derive(Debug)]
pub struct OOMContainment {
    handle: tokio::task::JoinHandle<()>,
    disarm: CancellationToken,
}

impl OOMContainment {
    /// Arm containment: every `tick`, `usage` is polled for the current
    /// memory footprint; if it exceeds `max_memory_bytes`, `token` is
    /// cancelled. Dropping the returned guard disarms the containment.
    pub fn arm<F>(max_memory_bytes: u64, tick: Duration, token: CancellationToken, usage: F) -> Self
    where
        F: Fn() -> u64 + Send + Sync + 'static,
    {
        let disarm = CancellationToken::new();
        let disarmer = disarm.clone();
        let watched = token.clone();
        let handle = if max_memory_bytes == 0 {
            disarmer.cancel();
            tokio::spawn(async {})
        } else {
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(tick);
                ticker.tick().await;
                loop {
                    ticker.tick().await;
                    if disarmer.is_cancelled() {
                        break;
                    }
                    if usage() > max_memory_bytes {
                        watched.cancel();
                        break;
                    }
                }
            })
        };
        Self { handle, disarm }
    }

    /// Disarm the containment without firing it (call on normal completion).
    pub fn disarm(self) {
        self.disarm.cancel();
    }
}

impl Drop for OOMContainment {
    fn drop(&mut self) {
        self.disarm.cancel();
        self.handle.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforce_passes_under_quota() {
        let guard = WorkerGuard::new(ResourceQuota {
            max_memory_bytes: 1024,
            max_cpu_ms: 100,
            max_wall_ms: 200,
            max_network_bytes: 100,
            max_requests: 5,
        });
        assert!(guard.enforce());
    }

    #[test]
    fn enforce_fails_memory_exceeded() {
        let mut guard = WorkerGuard::new(ResourceQuota {
            max_memory_bytes: 100,
            max_cpu_ms: 100,
            max_wall_ms: 200,
            max_network_bytes: 100,
            max_requests: 5,
        });
        guard.add_usage(ResourceUsage { memory_bytes: 150, ..Default::default() });
        assert!(!guard.enforce());
    }

    #[test]
    fn enforce_fails_cpu_exceeded() {
        let mut guard = WorkerGuard::new(ResourceQuota {
            max_memory_bytes: 1024,
            max_cpu_ms: 50,
            max_wall_ms: 200,
            max_network_bytes: 100,
            max_requests: 5,
        });
        guard.add_usage(ResourceUsage { cpu_ms: 60, ..Default::default() });
        assert!(!guard.enforce());
    }

    #[test]
    fn enforce_fails_wall_exceeded() {
        let mut guard = WorkerGuard::new(ResourceQuota {
            max_memory_bytes: 1024,
            max_cpu_ms: 100,
            max_wall_ms: 10,
            max_network_bytes: 100,
            max_requests: 5,
        });
        guard.add_usage(ResourceUsage { wall_ms: 15, ..Default::default() });
        assert!(!guard.enforce());
    }

    #[test]
    fn enforce_fails_network_exceeded() {
        let mut guard = WorkerGuard::new(ResourceQuota {
            max_memory_bytes: 1024,
            max_cpu_ms: 100,
            max_wall_ms: 200,
            max_network_bytes: 10,
            max_requests: 5,
        });
        guard.add_usage(ResourceUsage { network_bytes: 20, ..Default::default() });
        assert!(!guard.enforce());
    }

    #[test]
    fn enforce_fails_requests_exceeded() {
        let mut guard = WorkerGuard::new(ResourceQuota {
            max_memory_bytes: 1024,
            max_cpu_ms: 100,
            max_wall_ms: 200,
            max_network_bytes: 100,
            max_requests: 2,
        });
        guard.add_usage(ResourceUsage { requests: 3, ..Default::default() });
        assert!(!guard.enforce());
    }

    #[test]
    fn enforce_at_limit_is_ok() {
        let mut guard = WorkerGuard::new(ResourceQuota {
            max_memory_bytes: 100,
            max_cpu_ms: 100,
            max_wall_ms: 100,
            max_network_bytes: 100,
            max_requests: 100,
        });
        guard.add_usage(ResourceUsage {
            memory_bytes: 100,
            cpu_ms: 100,
            wall_ms: 100,
            network_bytes: 100,
            requests: 100,
        });
        assert!(guard.enforce());
    }

    #[tokio::test]
    async fn watchdog_fires_on_wall_breach() {
        let token = tokio_util::sync::CancellationToken::new();
        let wd = Watchdog::arm(10, token.clone());
        tokio::time::timeout(Duration::from_millis(500), token.cancelled())
            .await
            .expect("watchdog must fire within its budget");
        drop(wd);
    }

    #[tokio::test]
    async fn watchdog_disarm_prevents_fire() {
        let token = tokio_util::sync::CancellationToken::new();
        let wd = Watchdog::arm(10, token.clone());
        wd.disarm();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!token.is_cancelled(), "disarmed watchdog must not fire");
    }

    #[tokio::test]
    async fn watchdog_zero_budget_is_unlimited() {
        let token = tokio_util::sync::CancellationToken::new();
        let wd = Watchdog::arm(0, token.clone());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!token.is_cancelled(), "zero wall budget means unlimited");
        drop(wd);
    }

    #[tokio::test]
    async fn oom_containment_fires_on_cap_breach() {
        let token = tokio_util::sync::CancellationToken::new();
        let oom = OOMContainment::arm(100, Duration::from_millis(5), token.clone(), || 1_000);
        tokio::time::timeout(Duration::from_millis(500), token.cancelled())
            .await
            .expect("containment must fire when memory exceeds cap");
        drop(oom);
    }

    #[tokio::test]
    async fn oom_containment_passes_under_cap() {
        let token = tokio_util::sync::CancellationToken::new();
        let oom = OOMContainment::arm(1_000, Duration::from_millis(5), token.clone(), || 100);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!token.is_cancelled(), "usage under the cap must not fire containment");
        oom.disarm();
    }

    #[tokio::test]
    async fn oom_containment_zero_cap_is_unlimited() {
        let token = tokio_util::sync::CancellationToken::new();
        let oom = OOMContainment::arm(0, Duration::from_millis(5), token.clone(), || 1_000_000);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!token.is_cancelled(), "zero memory cap means unlimited");
        drop(oom);
    }

    #[test]
    fn enforce_passes_with_zero_quota_and_zero_usage() {
        let guard = WorkerGuard::new(ResourceQuota::default());
        assert!(guard.enforce());
    }
}
