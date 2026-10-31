//! Rate limiters: one aggregate limiter and one per backend name.

use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

pub(super) struct BackendRateLimiters {
    knowledge: DefaultKeyedRateLimiter<String>,
}

impl BackendRateLimiters {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            knowledge: Self::knowledge_limiter(),
        })
    }

    /// Moderate default keyed limiter: 1 req / 2s sustained, burst 3.
    pub(super) fn default_limiter() -> DefaultKeyedRateLimiter<String> {
        RateLimiter::dashmap(
            Quota::with_period(Duration::from_secs(2))
                .expect("2s period is valid")
                .allow_burst(NonZeroU32::new(3).unwrap()),
        )
    }

    /// Knowledge backends: 2 req / s sustained, burst 2.
    fn knowledge_limiter() -> DefaultKeyedRateLimiter<String> {
        RateLimiter::dashmap(Quota::per_second(NonZeroU32::new(2).unwrap()))
    }

    pub(super) async fn until_ready(
        &self,
        name: &str,
        scraper_default: &DefaultKeyedRateLimiter<String>,
    ) {
        let key = name.to_string();
        match name {
            // Scraper backends use the moderate default keyed limiter on SearchProvider.
            "bing" | "duckduckgo" => scraper_default.until_key_ready(&key).await,
            _ => self.knowledge.until_key_ready(&key).await,
        }
    }
}
