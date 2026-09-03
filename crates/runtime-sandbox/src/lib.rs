use std::time::Duration;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourceQuota {
    pub max_memory_bytes: u64,
    pub max_cpu_ms: u64,
    pub max_wall_ms: u64,
    pub max_network_bytes: u64,
    pub max_requests: u32,
}

#[derive(Clone, Debug)]
pub struct Watchdog { /* stub for crash/watchdog */ }

#[derive(Clone, Debug)]
pub struct WorkerGuard {
    pub quota: ResourceQuota,
}

impl WorkerGuard {
    pub fn enforce(&self) -> bool { true /* stub: real enforcement measures quotas */ }
}
