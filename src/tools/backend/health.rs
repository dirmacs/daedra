//! Circuit-breaker state for a single backend.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

pub struct BackendHealth {
    consecutive_failures: AtomicU32,
    is_open: AtomicBool,
    last_failure: Mutex<std::time::Instant>,
    failure_threshold: u32,
    cooldown: Duration,
}

impl BackendHealth {
    /// Create a new circuit breaker that opens after `failure_threshold` consecutive failures and stays open for `cooldown` duration.
    pub fn new(failure_threshold: u32, cooldown: Duration) -> Self {
        Self {
            consecutive_failures: AtomicU32::new(0),
            is_open: AtomicBool::new(false),
            last_failure: Mutex::new(std::time::Instant::now()),
            failure_threshold,
            cooldown,
        }
    }

    /// Returns true when the backend may be queried (closed circuit or cooldown elapsed for probe).
    pub fn is_available(&self) -> bool {
        if !self.is_open.load(Ordering::Relaxed) {
            return true;
        }
        let last = self.last_failure.lock().expect("last_failure lock");
        last.elapsed() >= self.cooldown
    }

    /// Record a successful request — resets consecutive failure count and closes the circuit.
    pub fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
        self.is_open.store(false, Ordering::Relaxed);
    }

    /// Record a failed request — increments failure count and opens the circuit when threshold is reached.
    pub fn record_failure(&self) {
        let failures = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        *self.last_failure.lock().expect("last_failure lock") = std::time::Instant::now();
        if failures >= self.failure_threshold {
            self.is_open.store(true, Ordering::Relaxed);
        }
    }
}
