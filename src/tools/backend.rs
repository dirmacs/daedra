//! Search backend trait and multi-backend provider.
//!
//! Daedra supports multiple search backends with automatic fallback:
//! - Bing HTML scraping (default, no API key needed)
//! - Serper.dev (Google results via API, needs SERPER_API_KEY)
//! - Tavily (AI-optimized search, needs TAVILY_API_KEY)
//! - DuckDuckGo HTML scraping (blocked from datacenter IPs, fallback only)

use crate::types::{DaedraError, DaedraResult, SearchArgs, SearchResponse};
use async_trait::async_trait;
use backoff::backoff::Backoff;
use backoff::ExponentialBackoff;
use governor::{DefaultDirectRateLimiter, DefaultKeyedRateLimiter, Quota, RateLimiter};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{info, warn};

/// In-memory circuit breaker for a single backend.
/// After 3 consecutive failures, opens the circuit (marks backend unhealthy).
/// After 30s cooldown, allows one probe request. If it succeeds, closes the circuit.
#[derive(Debug)]
pub struct BackendHealth {
    consecutive_failures: AtomicU32,
    is_open: AtomicBool,
    last_failure: Mutex<std::time::Instant>,
    failure_threshold: u32,
    cooldown: Duration,
}

impl BackendHealth {
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

    pub fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
        self.is_open.store(false, Ordering::Relaxed);
    }

    pub fn record_failure(&self) {
        let failures = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        *self.last_failure.lock().expect("last_failure lock") = std::time::Instant::now();
        if failures >= self.failure_threshold {
            self.is_open.store(true, Ordering::Relaxed);
        }
    }
}

/// Per-backend rate limits keyed by backend name (category-specific quotas).
struct BackendRateLimiters {
    api: DefaultKeyedRateLimiter<String>,
    knowledge: DefaultKeyedRateLimiter<String>,
}

impl BackendRateLimiters {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            api: Self::api_limiter(),
            knowledge: Self::knowledge_limiter(),
        })
    }

    /// Moderate default keyed limiter: 1 req / 2s sustained, burst 3.
    fn default_limiter() -> DefaultKeyedRateLimiter<String> {
        RateLimiter::dashmap(
            Quota::with_period(Duration::from_secs(2))
                .expect("2s period is valid")
                .allow_burst(NonZeroU32::new(3).unwrap()),
        )
    }


    /// API backends (serper, tavily): 2 req / s sustained, burst 2.
    fn api_limiter() -> DefaultKeyedRateLimiter<String> {
        RateLimiter::dashmap(Quota::per_second(NonZeroU32::new(2).unwrap()))
    }

    /// Knowledge backends: 2 req / s sustained, burst 2.
    fn knowledge_limiter() -> DefaultKeyedRateLimiter<String> {
        RateLimiter::dashmap(Quota::per_second(NonZeroU32::new(2).unwrap()))
    }

    async fn until_ready(
        &self,
        name: &str,
        scraper_default: &DefaultKeyedRateLimiter<String>,
    ) {
        let key = name.to_string();
        match name {
            // Scraper backends use the moderate default keyed limiter on SearchProvider.
            "bing" | "duckduckgo" => scraper_default.until_key_ready(&key).await,
            "serper" | "tavily" => self.api.until_key_ready(&key).await,
            _ => self.knowledge.until_key_ready(&key).await,
        }
    }
}

/// Trait for search backends. Each backend implements web search
/// and returns results in the common SearchResponse format.
#[async_trait]
pub trait SearchBackend: Send + Sync {
    /// Execute a search query and return results.
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse>;

    /// Backend name for logging and diagnostics.
    fn name(&self) -> &str;

    /// Whether this backend requires an API key.
    fn requires_api_key(&self) -> bool {
        false
    }

    /// Whether this backend is available (has required config/keys).
    fn is_available(&self) -> bool {
        true
    }
}

/// Multi-backend search provider with automatic fallback.
///
/// Tries backends in priority order. If the primary fails,
/// falls back to the next available backend.
pub struct SearchProvider {
    backends: Vec<Box<dyn SearchBackend>>,
    /// Limits how fast aggregate searches are issued (avoids tripping scraper rate limits).
    rate_limiter: DefaultDirectRateLimiter,
    backend_limiters: DefaultKeyedRateLimiter<String>,
    backend_rate_limits: Arc<BackendRateLimiters>,
    circuit_breakers: HashMap<String, Arc<BackendHealth>>,
}

impl SearchProvider {
    fn new_rate_limiter() -> DefaultDirectRateLimiter {
        // ~6 searches per 10s sustained: 1 cell per ~1.67s, burst of 6
        RateLimiter::direct(
            Quota::with_period(Duration::from_millis(167))
                .expect("167ms period is valid")
                .allow_burst(NonZeroU32::new(6).unwrap()),
        )
    }

    fn new_backend_limiters() -> DefaultKeyedRateLimiter<String> {
        BackendRateLimiters::default_limiter()
    }

    fn init_circuit_breakers(backends: &[Box<dyn SearchBackend>]) -> HashMap<String, Arc<BackendHealth>> {
        backends
            .iter()
            .map(|b| {
                (
                    b.name().to_string(),
                    Arc::new(BackendHealth::new(3, Duration::from_secs(30))),
                )
            })
            .collect()
    }

    fn from_backends(backends: Vec<Box<dyn SearchBackend>>) -> Self {
        let circuit_breakers = Self::init_circuit_breakers(&backends);
        Self {
            backends,
            rate_limiter: Self::new_rate_limiter(),
            backend_limiters: Self::new_backend_limiters(),
            backend_rate_limits: BackendRateLimiters::new(),
            circuit_breakers,
        }
    }

    /// Create a new provider with the given backends (in priority order).
    pub fn new(backends: Vec<Box<dyn SearchBackend>>) -> Self {
        Self::from_backends(backends)
    }

    /// Create a provider with all available backends auto-detected from env.
    pub fn auto() -> Self {
        let mut backends: Vec<Box<dyn SearchBackend>> = Vec::new();

        // Serper (Google results) — if API key is set
        if let Ok(key) = std::env::var("SERPER_API_KEY") {
            if !key.is_empty() {
                info!("Serper backend enabled (SERPER_API_KEY set)");
                backends.push(Box::new(super::serper::SerperBackend::new(key)));
            }
        }

        // Tavily — if API key is set
        if let Ok(key) = std::env::var("TAVILY_API_KEY") {
            if !key.is_empty() {
                info!("Tavily backend enabled (TAVILY_API_KEY set)");
                backends.push(Box::new(super::tavily::TavilyBackend::new(key)));
            }
        }

        // Bing HTML scraping — no API key, but often CAPTCHA-blocked from datacenter IPs
        info!("Bing backend enabled (no API key, may be blocked from datacenter IPs)");
        backends.push(Box::new(super::bing::BingBackend::new()));

        // Wikipedia — always works from any IP, knowledge-focused
        info!("Wikipedia backend enabled (always works, knowledge-focused)");
        backends.push(Box::new(super::wikipedia::WikipediaBackend::new()));

        // StackExchange — always works from any IP, technical Q&A
        info!("StackExchange backend enabled (always works, technical)");
        backends.push(Box::new(super::stackexchange::StackExchangeBackend::new()));

        // GitHub — always works, code/repo search
        info!("GitHub backend enabled (always works, code/repos)");
        backends.push(Box::new(super::github::GitHubBackend::new()));

        // Wiby — indie web search, always works
        info!("Wiby backend enabled (always works, indie web)");
        backends.push(Box::new(super::wiby::WibyBackend::new()));

        // DDG Instant Answers — knowledge graph, always works (different from HTML scraping)
        info!("DDG Instant Answers backend enabled (always works, knowledge)");
        backends.push(Box::new(super::ddg_instant::DdgInstantBackend::new()));

        // DDG HTML scraping — blocked from most datacenter IPs, last resort
        info!("DuckDuckGo HTML backend enabled (last resort)");
        backends.push(Box::new(super::search::SearchClient::new().unwrap()));

        Self::from_backends(backends)
    }

    fn is_non_retryable(err: &DaedraError) -> bool {
        match err {
            DaedraError::BotProtectionDetected | DaedraError::RateLimitExceeded => true,
            DaedraError::SearchError(msg) => {
                let m = msg.to_lowercase();
                m.contains("403")
                    || m.contains("captcha")
                    || m.contains("bot protection")
                    || m.contains("bot detected")
            }
            _ => false,
        }
    }

    fn is_transient(err: &DaedraError) -> bool {
        matches!(
            err,
            DaedraError::HttpError(_) | DaedraError::Timeout
        )
    }

    async fn query_backend(
        b: &dyn SearchBackend,
        args: &SearchArgs,
        health: Option<Arc<BackendHealth>>,
        limiters: &Arc<BackendRateLimiters>,
        scraper_default: &DefaultKeyedRateLimiter<String>,
    ) -> (String, DaedraResult<SearchResponse>) {
        let name = b.name().to_string();

        limiters.until_ready(&name, scraper_default).await;

        if let Some(ref h) = health {
            if !h.is_available() {
                info!(backend = %name, "Circuit open, skipping");
                return (
                    name.clone(),
                    Err(DaedraError::SearchError(format!(
                        "Backend {} circuit open",
                        name
                    ))),
                );
            }
        }

        info!(backend = %name, query = %args.query, "Querying backend");
        let result = b.search(args).await;

        match &result {
            Ok(r) if !r.data.is_empty() => {
                if let Some(ref h) = health {
                    h.record_success();
                }
                (name, result)
            }
            Ok(_) => {
                // Empty but not an error — don't retry or penalize the circuit.
                (name, result)
            }
            Err(e) if Self::is_non_retryable(e) => {
                if let Some(ref h) = health {
                    h.record_failure();
                }
                (name, result)
            }
            Err(e) if Self::is_transient(e) => {
                if let Some(ref h) = health {
                    h.record_failure();
                }
                warn!(backend = %name, error = %e, "Backend transient error, retrying once");
                let mut backoff = ExponentialBackoff {
                    initial_interval: Duration::from_millis(400),
                    max_interval: Duration::from_secs(2),
                    max_elapsed_time: Some(Duration::from_secs(3)),
                    ..Default::default()
                };
                if let Some(delay) = backoff.next_backoff() {
                    tokio::time::sleep(delay).await;
                }
                let retry_result = b.search(args).await;
                match &retry_result {
                    Ok(r) if !r.data.is_empty() => {
                        if let Some(ref h) = health {
                            h.record_success();
                        }
                    }
                    Err(retry_err) if Self::is_non_retryable(retry_err) => {
                        if let Some(ref h) = health {
                            h.record_failure();
                        }
                    }
                    Err(_) => {
                        if let Some(ref h) = health {
                            h.record_failure();
                        }
                    }
                    _ => {}
                }
                (name, retry_result)
            }
            Err(e) => {
                if let Some(ref h) = health {
                    h.record_failure();
                }
                warn!(backend = %name, error = %e, "Backend error (no retry)");
                (name, result)
            }
        }
    }

    /// Aggregate search across ALL available backends.
    ///
    /// Queries all backends concurrently, merges results, deduplicates by URL,
    /// and interleaves sources for diversity (Wikipedia, StackOverflow, GitHub
    /// results mixed rather than grouped).
    pub async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();
        let target_count = opts.num_results;

        // Throttle aggregate searches — wait instead of failing when over limit.
        self.rate_limiter.until_ready().await;

        let queryable: Vec<_> = self
            .backends
            .iter()
            .filter(|b| b.is_available())
            .filter(|b| {
                self.circuit_breakers
                    .get(b.name())
                    .map(|h| h.is_available())
                    .unwrap_or(true)
            })
            .collect();

        if queryable.is_empty() {
            let open: Vec<String> = self
                .circuit_breakers
                .iter()
                .filter(|(_, h)| !h.is_available())
                .map(|(name, _)| name.clone())
                .collect();
            return Err(DaedraError::SearchError(format!(
                "All search backends have open circuits (cooldown in progress). Open: [{}]",
                open.join(", ")
            )));
        }

        let limiters = Arc::clone(&self.backend_rate_limits);
        let scraper_default = &self.backend_limiters;
        let futures: Vec<_> = queryable
            .iter()
            .map(|b| {
                let a = args.clone();
                let health = self.circuit_breakers.get(b.name()).cloned();
                let limiters = Arc::clone(&limiters);
                async move {
                    Self::query_backend(b.as_ref(), &a, health, &limiters, scraper_default).await
                }
            })
            .collect();

        let results = futures::future::join_all(futures).await;
        let tried: Vec<String> = results.iter().map(|(name, _)| name.clone()).collect();

        // Collect all successful results grouped by backend
        let mut by_source: Vec<(String, Vec<crate::types::SearchResult>)> = Vec::new();
        let mut any_success = false;

        for (name, result) in results {
            info!(
                backend = %name,
                result = match &result {
                    Ok(r) if !r.data.is_empty() => "ok",
                    Ok(_) => "empty",
                    Err(_) => "err",
                },
                count = match &result {
                    Ok(r) => r.data.len(),
                    Err(_) => 0,
                },
                "Backend result"
            );
            match result {
                Ok(response) if !response.data.is_empty() => {
                    any_success = true;
                    by_source.push((name, response.data));
                }
                Ok(_) => {}
                Err(e) => {
                    warn!(backend = %name, error = %e, "Backend failed");
                }
            }
        }

        if !any_success {
            let open_circuits: Vec<String> = self
                .circuit_breakers
                .iter()
                .filter(|(name, h)| tried.contains(name) && !h.is_available())
                .map(|(name, _)| name.clone())
                .collect();
            let circuit_note = if open_circuits.is_empty() {
                String::new()
            } else {
                format!("; open circuits: [{}]", open_circuits.join(", "))
            };
            return Err(DaedraError::SearchError(format!(
                "All {} search backends returned 0 results (tried: {}){}",
                tried.len(),
                tried.join(", "),
                circuit_note
            )));
        }

        // Interleave results from different sources for diversity
        // Round-robin: take 1 from each source, repeat until we have enough
        let mut merged: Vec<crate::types::SearchResult> = Vec::new();
        let mut seen_urls: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut indices: Vec<usize> = vec![0; by_source.len()];

        loop {
            let mut added_this_round = false;
            for (i, (_name, results)) in by_source.iter().enumerate() {
                if merged.len() >= target_count {
                    break;
                }
                while indices[i] < results.len() {
                    let r = &results[indices[i]];
                    indices[i] += 1;
                    if seen_urls.insert(r.url.clone()) {
                        merged.push(r.clone());
                        added_this_round = true;
                        break;
                    }
                }
            }
            if !added_this_round || merged.len() >= target_count {
                break;
            }
        }

        let sources: Vec<String> = by_source.iter().map(|(n, _)| n.clone()).collect();
        info!(
            total = merged.len(),
            sources = ?sources,
            "Aggregated results from {} backends",
            sources.len()
        );

        Ok(SearchResponse::new(args.query.clone(), merged, &opts))
    }

    /// List available backend names.
    pub fn available_backends(&self) -> Vec<&str> {
        self.backends
            .iter()
            .filter(|b| b.is_available())
            .map(|b| b.name())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SearchArgs;

    #[test]
    fn test_auto_has_backends() {
        let provider = SearchProvider::auto();
        let backends = provider.available_backends();
        // Should always have at least 7 no-key backends
        assert!(
            backends.len() >= 7,
            "Expected at least 7 backends, got {}",
            backends.len()
        );
        assert!(backends.contains(&"bing"));
        assert!(backends.contains(&"wikipedia"));
        assert!(backends.contains(&"stackoverflow"));
        assert!(backends.contains(&"github"));
        assert!(backends.contains(&"wiby"));
        assert!(backends.contains(&"ddg-instant"));
        assert!(backends.contains(&"duckduckgo"));
    }

    #[test]
    fn test_empty_provider() {
        let provider = SearchProvider::new(vec![]);
        assert!(provider.available_backends().is_empty());
    }

    #[test]
    fn test_circuit_breaker_opens_after_failures() {
        let health = BackendHealth::new(3, Duration::from_secs(30));
        assert!(health.is_available());
        health.record_failure();
        health.record_failure();
        assert!(health.is_available());
        health.record_failure();
        assert!(!health.is_available());
        health.record_success();
        assert!(health.is_available());
    }

    #[tokio::test]
    async fn test_fallback_chain_live() {
        // This test uses real network — Wikipedia + SO should always return results
        let provider = SearchProvider::auto();
        let args = SearchArgs {
            query: "Rust programming".to_string(),
            options: Some(crate::types::SearchOptions {
                num_results: 3,
                ..Default::default()
            }),
        };
        let response = provider.search(&args).await;
        assert!(
            response.is_ok(),
            "Fallback chain should find results from at least one backend"
        );
        let data = response.unwrap();
        assert!(!data.data.is_empty(), "Should have at least 1 result");
    }
}
