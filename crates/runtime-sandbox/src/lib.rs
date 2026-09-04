
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
pub struct Watchdog { /* stub for crash/watchdog */ }

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
        // max_memory_bytes
        if self.usage.memory_bytes > self.quota.max_memory_bytes {
            return false;
        }
        // max_cpu_ms
        if self.usage.cpu_ms > self.quota.max_cpu_ms {
            return false;
        }
        // max_wall_ms
        if self.usage.wall_ms > self.quota.max_wall_ms {
            return false;
        }
        // max_network_bytes
        if self.usage.network_bytes > self.quota.max_network_bytes {
            return false;
        }
        // max_requests
        if self.usage.requests > self.quota.max_requests {
            return false;
        }
        true
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
}
