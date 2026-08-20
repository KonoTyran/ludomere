use std::{
    sync::{Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};

struct State {
    bytes_per_second: Option<u64>,
    tokens: f64,
    updated_at: Instant,
}

pub struct BandwidthLimiter {
    state: Mutex<State>,
    changed: Condvar,
}

impl BandwidthLimiter {
    fn new(bytes_per_second: Option<u64>) -> Self {
        Self {
            state: Mutex::new(State {
                bytes_per_second,
                tokens: bytes_per_second.unwrap_or_default() as f64,
                updated_at: Instant::now(),
            }),
            changed: Condvar::new(),
        }
    }

    pub fn set_limit(&self, bytes_per_second: Option<u64>) {
        let mut state = self.state.lock().unwrap();
        state.bytes_per_second = bytes_per_second.filter(|limit| *limit > 0);
        state.tokens = state.bytes_per_second.unwrap_or_default() as f64;
        state.updated_at = Instant::now();
        self.changed.notify_all();
    }

    pub fn acquire(&self, bytes: u64, cancelled: impl Fn() -> bool) -> bool {
        let mut remaining = bytes as f64;
        let mut state = self.state.lock().unwrap();
        while remaining > 0.0 {
            if cancelled() {
                return false;
            }
            let Some(limit) = state.bytes_per_second else {
                return true;
            };
            let now = Instant::now();
            state.tokens = (state.tokens
                + now.duration_since(state.updated_at).as_secs_f64() * limit as f64)
                .min(limit as f64);
            state.updated_at = now;
            let consumed = remaining.min(state.tokens);
            remaining -= consumed;
            state.tokens -= consumed;
            if remaining > 0.0 {
                let wait =
                    Duration::from_secs_f64((remaining.min(limit as f64) / limit as f64).min(0.1));
                let result = self.changed.wait_timeout(state, wait).unwrap();
                state = result.0;
            }
        }
        true
    }
}

fn global() -> &'static BandwidthLimiter {
    static LIMITER: OnceLock<BandwidthLimiter> = OnceLock::new();
    LIMITER.get_or_init(|| BandwidthLimiter::new(None))
}

pub fn set_limit(bytes_per_second: Option<u64>) {
    global().set_limit(bytes_per_second);
}

pub(crate) fn acquire(bytes: u64, cancelled: impl Fn() -> bool) -> bool {
    global().acquire(bytes, cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn switching_to_unlimited_wakes_waiters() {
        let limiter = Arc::new(BandwidthLimiter::new(Some(1)));
        assert!(limiter.acquire(1, || false));
        let waiting = limiter.clone();
        let worker = std::thread::spawn(move || waiting.acquire(10, || false));
        std::thread::sleep(Duration::from_millis(20));
        limiter.set_limit(None);
        assert!(worker.join().unwrap());
    }

    #[test]
    fn cancellation_interrupts_a_throttled_transfer() {
        let limiter = BandwidthLimiter::new(Some(1));
        assert!(limiter.acquire(1, || false));
        assert!(!limiter.acquire(10, || true));
    }

    #[test]
    fn enforces_the_shared_rate_after_the_initial_bucket() {
        let limiter = BandwidthLimiter::new(Some(10_000));
        assert!(limiter.acquire(10_000, || false));
        let started = Instant::now();
        assert!(limiter.acquire(2_000, || false));
        assert!(started.elapsed() >= Duration::from_millis(150));
    }
}
