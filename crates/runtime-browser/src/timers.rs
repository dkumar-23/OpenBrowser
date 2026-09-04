use std::time::Duration;
use tokio::time::{sleep, Duration as TD};


#[derive(Debug, Clone)]
pub struct TimerHandle { cancelled: std::sync::Arc<std::sync::atomic::AtomicBool> }
impl TimerHandle { pub fn cancel(&self) { self.cancelled.store(true, std::sync::atomic::Ordering::Relaxed); } }

#[derive(Debug, Clone)]
pub struct IntervalHandle { cancelled: std::sync::Arc<std::sync::atomic::AtomicBool> }
impl IntervalHandle { pub fn cancel(&self) { self.cancelled.store(true, std::sync::atomic::Ordering::Relaxed); } }

pub fn set_timeout<F>(delay: Duration, callback: F) -> TimerHandle
where F: FnOnce() + Send + 'static
{
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let c = cancelled.clone();
    tokio::spawn(async move {
        sleep(TD::from(delay)).await;
        if !c.load(std::sync::atomic::Ordering::Relaxed) { callback(); }
    });
    TimerHandle { cancelled }
}

pub fn set_interval<F>(period: Duration, callback: F) -> IntervalHandle
where F: Fn() + Send + Sync + 'static
{
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let c = cancelled.clone();
    tokio::spawn(async move {
        loop {
            sleep(TD::from(period)).await;
            if c.load(std::sync::atomic::Ordering::Relaxed) { break; }
            callback();
        }
    });
    IntervalHandle { cancelled }
}
