//! Search backend trait and multi-backend provider.
//!
//! Daedra supports multiple search backends with automatic fallback:
//! - Mwmbl public JSON (general web, no API key)
//! - Marginalia, Bing RSS, Google News, Hacker News (machine formats)
//! - Brave / Bing / Google / DuckDuckGo HTML (no key; may meet a CAPTCHA)
//! - Wikipedia, StackExchange, GitHub, Wiby, DDG Instant (knowledge APIs)

use crate::types::{DaedraError, DaedraResult, SearchArgs, SearchResponse};
use async_trait::async_trait;
use governor::{DefaultDirectRateLimiter, DefaultKeyedRateLimiter, Quota, RateLimiter};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{info, warn};

/// Circuit breaker state for a single backend — opens after consecutive failures, cools down, then probes.
#[derive(Debug)]
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

/// Per-backend rate limits keyed by backend name (category-specific quotas).
struct BackendRateLimiters {
    knowledge: DefaultKeyedRateLimiter<String>,
}

impl BackendRateLimiters {
    fn new() -> Arc<Self> {
        Arc::new(Self {
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

    /// Knowledge backends: 2 req / s sustained, burst 2.
    fn knowledge_limiter() -> DefaultKeyedRateLimiter<String> {
        RateLimiter::dashmap(Quota::per_second(NonZeroU32::new(2).unwrap()))
    }

    async fn until_ready(&self, name: &str, scraper_default: &DefaultKeyedRateLimiter<String>) {
        let key = name.to_string();
        match name {
            // Scraper backends use the moderate default keyed limiter on SearchProvider.
            "bing" | "duckduckgo" => scraper_default.until_key_ready(&key).await,
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

/// Outcome of aggregating per-backend results: grouped successful results,
/// whether any backend succeeded, every tried backend name, and per-backend
/// failures (name, error) for aggregate error reporting.
type CategorizedResults = (
    Vec<(String, Vec<crate::types::SearchResult>)>,
    bool,
    Vec<String>,
    Vec<(String, String)>,
);

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

    fn init_circuit_breakers(
        backends: &[Box<dyn SearchBackend>],
    ) -> HashMap<String, Arc<BackendHealth>> {
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

        // Mwmbl public JSON — unkeyed general web index. Answers from any IP.
        info!("Mwmbl backend enabled (no API key, answers from any IP)");
        backends.push(Box::new(super::mwmbl::MwmblBackend::new()));

        // Brave HTML — general index, no API key; 429s from datacenter IPs.
        info!("Brave backend enabled (no API key, may rate-limit datacenter IPs)");
        backends.push(Box::new(super::brave::BraveBackend::new()));

        // Marginalia public API — independent non-commercial index, answers
        // from any IP. Strong for docs and technical content.
        info!("Marginalia backend enabled (no API key, answers from any IP)");
        backends.push(Box::new(super::marginalia::MarginaliaBackend::new()));

        // Bing via its machine-readable RSS output — same index as the HTML
        // SERP, served to integrations without challenge pages.
        info!("Bing RSS backend enabled (no API key, bot-tolerant machine format)");
        backends.push(Box::new(super::rss::BingRssBackend::new()));

        // Bing HTML scraping — no API key, but often CAPTCHA-blocked from datacenter IPs
        info!("Bing backend enabled (no API key, may be blocked from datacenter IPs)");
        backends.push(Box::new(super::bing::BingBackend::new()));

        // Google News RSS — news coverage, no challenges.
        info!("Google News backend enabled (no API key, bot-tolerant machine format)");
        backends.push(Box::new(super::rss::GoogleNewsBackend::new()));

        // Hacker News via the public Algolia API — no key, strong for technical queries.
        info!("Hacker News backend enabled (no API key, Algolia JSON)");
        backends.push(Box::new(super::rss::HnAlgoliaBackend::new()));

        // Google HTML scraping — no API key, but CAPTCHA-prone from datacenter IPs
        info!("Google backend enabled (no API key, may be blocked from datacenter IPs)");
        backends.push(Box::new(super::google::GoogleBackend::new()));

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

    const NON_RETRYABLE_SUBSTRINGS: &[&str] = &["403", "captcha", "bot protection", "bot detected"];

    fn is_non_retryable(err: &DaedraError) -> bool {
        match err {
            DaedraError::BotProtectionDetected | DaedraError::RateLimitExceeded => true,
            DaedraError::SearchError(msg) => {
                let m = msg.to_lowercase();
                Self::NON_RETRYABLE_SUBSTRINGS.iter().any(|s| m.contains(s))
            },
            _ => false,
        }
    }

    const TRANSIENT_SUBSTRINGS: &[&str] = &["429", "timed out"];

    fn is_transient(err: &DaedraError) -> bool {
        match err {
            DaedraError::HttpError(_) | DaedraError::Timeout => true,
            DaedraError::SearchError(msg) => {
                let m = msg.to_lowercase();
                Self::TRANSIENT_SUBSTRINGS.iter().any(|s| m.contains(s))
            },
            _ => false,
        }
    }

    fn record_health_outcome(health: &Option<Arc<BackendHealth>>, success: bool) {
        if let Some(h) = health {
            if success {
                h.record_success();
            } else {
                h.record_failure();
            }
        }
    }

    fn handle_successful_result(
        name: String,
        result: DaedraResult<SearchResponse>,
        health: Option<Arc<BackendHealth>>,
    ) -> (String, DaedraResult<SearchResponse>) {
        if let Ok(r) = &result
            && !r.data.is_empty()
        {
            Self::record_health_outcome(&health, true);
        }
        (name, result)
    }

    fn handle_non_retryable(
        name: String,
        result: DaedraResult<SearchResponse>,
        health: Option<Arc<BackendHealth>>,
    ) -> (String, DaedraResult<SearchResponse>) {
        Self::record_health_outcome(&health, false);
        (name, result)
    }

    async fn retry_once(b: &dyn SearchBackend, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        // One retry after a fixed 400 ms pause. The caller classifies errors;
        // only transient errors reach this function.
        tokio::time::sleep(Duration::from_millis(400)).await;
        b.search(args).await
    }

    async fn handle_transient_error(
        b: &dyn SearchBackend,
        args: &SearchArgs,
        name: String,
        result: DaedraResult<SearchResponse>,
        health: Option<Arc<BackendHealth>>,
        _limiters: &Arc<BackendRateLimiters>,
        _scraper_default: &DefaultKeyedRateLimiter<String>,
    ) -> (String, DaedraResult<SearchResponse>) {
        if let Err(e) = &result {
            Self::record_health_outcome(&health, false);
            warn!(backend = %name, error = %e, "Backend transient error, retrying once");
        }
        let retry_result = Self::retry_once(b, args).await;
        match &retry_result {
            Ok(r) if !r.data.is_empty() => Self::record_health_outcome(&health, true),
            Err(retry_err) if Self::is_non_retryable(retry_err) => {
                Self::record_health_outcome(&health, false);
            },
            Err(_) => Self::record_health_outcome(&health, false),
            _ => {},
        }
        (name, retry_result)
    }

    fn handle_unrecoverable_error(
        name: String,
        result: DaedraResult<SearchResponse>,
        health: Option<Arc<BackendHealth>>,
    ) -> (String, DaedraResult<SearchResponse>) {
        if let Err(e) = &result {
            Self::record_health_outcome(&health, false);
            warn!(backend = %name, error = %e, "Backend error (no retry)");
        }
        (name, result)
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

        if let Some(h) = &health
            && !h.is_available()
        {
            info!(backend = %name, "Circuit open, skipping");
            return (
                name.clone(),
                Err(DaedraError::SearchError(format!(
                    "Backend {} circuit open",
                    name
                ))),
            );
        }

        info!(backend = %name, query = %args.query, "Querying backend");
        let result = b.search(args).await;

        match &result {
            Ok(_) => Self::handle_successful_result(name, result, health),
            Err(e) if Self::is_non_retryable(e) => Self::handle_non_retryable(name, result, health),
            Err(e) if Self::is_transient(e) => {
                Self::handle_transient_error(
                    b,
                    args,
                    name,
                    result,
                    health,
                    limiters,
                    scraper_default,
                )
                .await
            },
            Err(_) => Self::handle_unrecoverable_error(name, result, health),
        }
    }
    fn collect_queryable_backends(&self) -> Vec<&dyn SearchBackend> {
        self.backends
            .iter()
            .filter(|b| b.is_available())
            .filter(|b| {
                self.circuit_breakers
                    .get(b.name())
                    .map(|h| h.is_available())
                    .unwrap_or(true)
            })
            .map(|b| b.as_ref())
            .collect()
    }

    async fn execute_concurrent_queries(
        &self,
        backends: &[&dyn SearchBackend],
        args: &SearchArgs,
    ) -> Vec<(String, DaedraResult<SearchResponse>)> {
        let limiters = Arc::clone(&self.backend_rate_limits);
        let scraper_default = &self.backend_limiters;
        let futures: Vec<_> = backends
            .iter()
            .map(|b| {
                let a = args.clone();
                let health = self.circuit_breakers.get(b.name()).cloned();
                let limiters = Arc::clone(&limiters);
                async move { Self::query_backend(*b, &a, health, &limiters, scraper_default).await }
            })
            .collect();
        futures::future::join_all(futures).await
    }

    fn categorize_results(
        results: Vec<(String, DaedraResult<SearchResponse>)>,
    ) -> CategorizedResults {
        let tried: Vec<String> = results.iter().map(|(name, _)| name.clone()).collect();
        let mut by_source: Vec<(String, Vec<crate::types::SearchResult>)> = Vec::new();
        let mut failures: Vec<(String, String)> = Vec::new();
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
                },
                Ok(_) => {},
                Err(e) => {
                    warn!(backend = %name, error = %e, "Backend failed");
                    failures.push((name, e.to_string()));
                },
            }
        }

        (by_source, any_success, tried, failures)
    }

    /// Build the aggregate-failure error message, distinguishing backends that
    /// errored from backends that legitimately returned zero results (they have
    /// different remediations: errors mean rate limits/CAPTCHAs/breakers; empty
    /// means the query has no matches on that backend's index).
    fn aggregate_failure_message(
        tried: &[String],
        failures: &[(String, String)],
        empty: &[String],
        open_circuits: &[String],
    ) -> String {
        let circuit_note = if open_circuits.is_empty() {
            String::new()
        } else {
            format!("; open circuits: [{}]", open_circuits.join(", "))
        };
        let errors = failures
            .iter()
            .map(|(n, e)| format!("{n}: {e}"))
            .collect::<Vec<_>>()
            .join("; ");
        match (failures.is_empty(), empty.is_empty()) {
            (false, true) => format!(
                "All {} search backends failed (tried: {}); errors: [{}{}]",
                tried.len(),
                tried.join(", "),
                errors,
                circuit_note
            ),
            (true, false) => format!(
                "All {} search backends returned 0 results (tried: {}){}",
                tried.len(),
                tried.join(", "),
                circuit_note
            ),
            _ => format!(
                "All {} search backends returned no usable results (tried: {}); failed: [{}]; empty: [{}]{cir\
cuit_note}",
                tried.len(),
                tried.join(", "),
                errors,
                empty.join(", "),
                circuit_note = circuit_note
            ),
        }
    }

    /// Merge results across sources, best first.
    ///
    /// Every result is scored against the query tokens (see
    /// [`relevance_score`]). Each source's results sort by score, then the
    /// merge repeatedly takes the highest-scoring next result across all
    /// sources — a k-way merge, so a weak backend cannot outrank a strong
    /// one on equal scores, and score ties prefer the earlier backend in the
    /// fallback-chain order. Results that share no query token score 0 and
    /// land after every matched result: backends that answered a different
    /// question than the one asked (RSS feeds that ignore the query) no
    /// longer outvote backends that correctly returned nothing relevant.
    fn merge_ranked_results(
        by_source: &[(String, Vec<crate::types::SearchResult>)],
        query: &str,
        target_count: usize,
    ) -> Vec<crate::types::SearchResult> {
        let tokens = query_tokens(query);
        // Canonical-key counting across sources: a result that two or more
        // engines agree on gets a corroboration boost in its score.
        let mut corroboration: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for (_, results) in by_source {
            let mut seen_here: std::collections::HashSet<String> = std::collections::HashSet::new();
            for r in results {
                if let Some(key) = canonical_url_key(&r.url)
                    && seen_here.insert(key.clone())
                {
                    *corroboration.entry(key).or_insert(0) += 1;
                }
            }
        }

        let mut queues: Vec<std::vec::IntoIter<(f64, crate::types::SearchResult)>> = by_source
            .iter()
            .map(|(_, results)| {
                let mut scored: Vec<(f64, crate::types::SearchResult)> = results
                    .iter()
                    .map(|r| {
                        let mut s = relevance_score(r, &tokens);
                        // Corroboration boost: +0.35 per agreeing source
                        // beyond the first, capped at +0.7.
                        if let Some(key) = canonical_url_key(&r.url) {
                            let agreeing = corroboration.get(&key).copied().unwrap_or(1);
                            s += 0.35 * (agreeing.saturating_sub(1)).min(2) as f64;
                        }
                        (s, r.clone())
                    })
                    .collect();
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                scored.into_iter()
            })
            .collect();

        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut merged = Vec::with_capacity(target_count);
        // Start index for tie-breaks: equal best scores rotate across sources
        // so the merge interleaves them exactly like the old round-robin.
        let mut rr = 0usize;

        while merged.len() < target_count {
            for q in queues.iter_mut() {
                while let Some((_, r)) = q.as_slice().first() {
                    let seen_key = canonical_url_key(&r.url).unwrap_or_else(|| r.url.clone());
                    if seen.contains(&seen_key) {
                        q.next();
                    } else {
                        break;
                    }
                }
            }
            let mut best_score = f64::MIN;
            for q in queues.iter() {
                if let Some((s, _)) = q.as_slice().first()
                    && *s > best_score
                {
                    best_score = *s;
                }
            }
            if best_score == f64::MIN {
                break;
            }
            let mut picked = None;
            for off in 0..queues.len() {
                let i = (rr + off) % queues.len();
                if queues[i]
                    .as_slice()
                    .first()
                    .is_some_and(|(s, _)| *s == best_score)
                {
                    picked = Some(i);
                    break;
                }
            }
            let Some(i) = picked else { break };
            if let Some((_, r)) = queues[i].next() {
                let key = canonical_url_key(&r.url).unwrap_or_else(|| r.url.clone());
                seen.insert(key);
                merged.push(r);
                rr = (i + 1) % queues.len();
            }
        }

        merged
    }

    /// Execute a search across all backends with fallback, rate limiting, and circuit breaker protection.
    pub async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();
        let target_count = opts.num_results;

        self.rate_limiter.until_ready().await;

        if args.query.trim().is_empty() {
            return Err(DaedraError::InvalidArguments(
                "query must not be empty".to_string(),
            ));
        }
        if target_count == 0 {
            return Err(DaedraError::InvalidArguments(
                "num_results must be at least 1".to_string(),
            ));
        }
        if let Some(tr) = &opts.time_range
            && crate::types::time_range_secs(tr).is_none()
        {
            return Err(DaedraError::InvalidArguments(format!(
                "invalid time range {tr:?}: use d, w, m, or y"
            )));
        }

        let queryable = self.collect_queryable_backends();
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

        let mut queryable = queryable;
        if let Some(want) = &opts.backends {
            for w in want {
                if !self.backends.iter().any(|b| b.name() == w) {
                    tracing::warn!(backend = %w, "requested backend does not exist");
                }
            }
            queryable.retain(|b| want.iter().any(|w| w == b.name()));
        }
        if let Some(excl) = &opts.exclude_backends {
            queryable.retain(|b| !excl.iter().any(|e| e == b.name()));
        }
        if queryable.is_empty() {
            return Err(DaedraError::InvalidArguments(
                "no search backends match the backend/exclude selection".to_string(),
            ));
        }

        let results = self.execute_concurrent_queries(&queryable, args).await;
        let (by_source, any_success, tried, failures) = Self::categorize_results(results);

        if !any_success {
            let open_circuits: Vec<String> = self
                .circuit_breakers
                .iter()
                .filter(|(name, h)| tried.contains(name) && !h.is_available())
                .map(|(name, _)| name.clone())
                .collect();
            let succeeded: std::collections::HashSet<&str> =
                by_source.iter().map(|(n, _)| n.as_str()).collect();
            let failed: std::collections::HashSet<&str> =
                failures.iter().map(|(n, _)| n.as_str()).collect();
            let empty: Vec<String> = tried
                .iter()
                .filter(|n| !succeeded.contains(n.as_str()) && !failed.contains(n.as_str()))
                .cloned()
                .collect();
            return Err(DaedraError::SearchError(Self::aggregate_failure_message(
                &tried,
                &failures,
                &empty,
                &open_circuits,
            )));
        }

        let mut merged = Self::merge_ranked_results(&by_source, &args.query, target_count);

        let tokens = query_tokens(&args.query);
        if !tokens.is_empty() {
            let matched: Vec<bool> = merged
                .iter()
                .map(|r| relevance_score(r, &tokens) > 0.0)
                .collect();
            if matched.iter().any(|m| *m) {
                // Real matches exist: drop the never-matched filler instead of
                // padding the list to `num_results` with it.
                let keep: Vec<crate::types::SearchResult> = merged
                    .into_iter()
                    .zip(matched)
                    .filter(|(_, m)| *m)
                    .map(|(r, _)| r)
                    .collect();
                merged = keep;
            } else {
                // Nothing matched, not even a fragment: an engine ignored the
                // query, and its output is worse than none for an agent.
                let tried: Vec<String> = by_source.iter().map(|(n, _)| n.clone()).collect();
                return Err(DaedraError::SearchError(format!(
                    "No search results matched the query {:?}; {} unrelated result(s) from [{}] \
                     were discarded. Try fewer or different keywords.",
                    args.query,
                    merged.len(),
                    tried.join(", ")
                )));
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

/// Canonical identity of a result URL for dedup and cross-engine
/// corroboration: lowercase host without `www.`, path without a trailing
/// slash, query stripped of tracking parameters. `None` when the URL does
/// not parse; the raw string is used instead in that case.
fn canonical_url_key(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let host = u
        .host_str()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let mut path = u.path().trim_end_matches('/').to_string();
    if path.is_empty() {
        path.push('/');
    }
    let kept: Vec<String> = u
        .query_pairs()
        .filter(|(k, _)| {
            let k = k.to_ascii_lowercase();
            !(k.starts_with("utm_")
                || matches!(
                    k.as_ref(),
                    "fbclid" | "gclid" | "ref" | "spm" | "mc_cid" | "mc_eid"
                ))
        })
        .map(|(k, v)| format!("{}={}", urlencoding::encode(&k), urlencoding::encode(&v)))
        .collect();
    let query = if kept.is_empty() {
        String::new()
    } else {
        format!("?{}", kept.join("&"))
    };
    Some(format!("{host}{path}{query}"))
}

/// Words too common to carry query meaning. Kept deliberately small: a
/// missed stopword only costs a little precision; a wrongly listed content
/// word costs recall.
fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        "the"
            | "an"
            | "of"
            | "in"
            | "on"
            | "for"
            | "to"
            | "and"
            | "or"
            | "is"
            | "are"
            | "was"
            | "were"
            | "be"
            | "at"
            | "by"
            | "it"
            | "its"
            | "with"
            | "from"
            | "as"
            | "that"
            | "this"
            | "how"
            | "what"
            | "why"
            | "do"
            | "does"
            | "did"
            | "my"
            | "me"
            | "you"
            | "your"
            | "he"
            | "she"
            | "we"
            | "they"
            | "not"
            | "no"
            | "de"
            | "la"
    )
}

/// Lowercase significant tokens of a query: alphanumeric runs of length 2+,
/// stopwords removed.
fn query_tokens(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2 && !is_stopword(t))
        .map(str::to_string)
        .collect()
}

/// True when `hay` contains a leading fragment of `token` at least half the
/// token long (minimum four characters). Search engines answer rare names
/// with stem matches ("baalateja" → "Baalat Gebal"); the literal token never
/// appears, so pure containment scoring rejects every result. The half-token
/// bar keeps keyboard-mash fragments ("asdf" inside "asdfqwertzxcv") from
/// counting: close-name stems pass, coincidental slivers do not.
fn partial_hit(token: &str, hay: &str) -> bool {
    let min = 4;
    if token.len() <= min {
        return false;
    }
    let need = token.len().div_ceil(2).max(min);
    let max = token.len() - 1;
    (need..=max).rev().any(|l| hay.contains(&token[..l]))
}

/// Share of query tokens present in a result's title, URL, or description.
/// The URL counts — a domain like `tokio.rs` should match "tokio". An empty
/// token list scores everything neutral (1.0), which keeps the k-way merge
/// order equivalent to the old round-robin when the query carries no signal.
/// A token whose leading fragment appears in the result counts half.
fn relevance_score(r: &crate::types::SearchResult, tokens: &[String]) -> f64 {
    if tokens.is_empty() {
        return 1.0;
    }
    let title_hay = format!("{} {}", r.title, r.url).to_lowercase();
    let hay = format!("{title_hay} {}", r.description).to_lowercase();
    let mut score = 0.0;
    for t in tokens {
        if hay.contains(t.as_str()) {
            score += 1.0;
        } else if partial_hit(t, &hay) {
            score += 0.5;
        }
    }
    score /= tokens.len() as f64;
    // Full-phrase bonus: the query words in order ("capital ... france" in
    // "Capital of France") outrank a story that merely contains both words.
    if tokens_in_order(&title_hay, tokens) {
        score += 1.0;
    }
    score
}

/// True when every token appears in `hay`, in order, with anything (or
/// nothing) between them.
fn tokens_in_order(hay: &str, tokens: &[String]) -> bool {
    let mut from = 0;
    for t in tokens {
        match hay[from..].find(t.as_str()) {
            Some(p) => from += p + t.len(),
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SearchArgs;

    #[test]
    fn test_auto_has_backends() {
        let provider = SearchProvider::auto();
        let backends = provider.available_backends();
        assert_eq!(
            backends.len(),
            14,
            "Expected 14 unkeyed backends, got {}",
            backends.len()
        );
        assert!(backends.contains(&"mwmbl"));
        assert!(backends.contains(&"bing"));
        assert!(backends.contains(&"wikipedia"));
        assert!(backends.contains(&"stackoverflow"));
        assert!(backends.contains(&"github"));
        assert!(backends.contains(&"wiby"));
        assert!(backends.contains(&"ddg-instant"));
        assert!(backends.contains(&"duckduckgo"));
        assert!(
            !backends.contains(&"mojeek"),
            "Mojeek HTML scrape was removed; it served CAPTCHAs to this client"
        );
        assert!(
            !backends.contains(&"serper"),
            "paid search keys were removed from the crate"
        );
        assert!(
            !backends.contains(&"tavily"),
            "paid search keys were removed from the crate"
        );
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

    #[test]
    fn test_circuit_breaker_half_open() {
        let health = BackendHealth::new(3, Duration::from_millis(50));
        for _ in 0..3 {
            health.record_failure();
        }
        assert!(!health.is_available());
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            health.is_available(),
            "after cooldown, circuit should be half-open (probe allowed)"
        );
    }

    #[test]
    fn test_circuit_breaker_stays_open_on_failure() {
        let health = BackendHealth::new(3, Duration::from_millis(50));
        for _ in 0..3 {
            health.record_failure();
        }
        std::thread::sleep(Duration::from_millis(60));
        assert!(health.is_available(), "half-open probe window");
        health.record_failure();
        assert!(
            !health.is_available(),
            "failed probe should keep circuit open"
        );
    }

    #[test]
    fn test_is_non_retryable() {
        assert!(SearchProvider::is_non_retryable(
            &DaedraError::BotProtectionDetected
        ));
        assert!(SearchProvider::is_non_retryable(
            &DaedraError::RateLimitExceeded
        ));
        assert!(SearchProvider::is_non_retryable(&DaedraError::SearchError(
            "HTTP 403 forbidden".to_string()
        )));
        assert!(SearchProvider::is_non_retryable(&DaedraError::SearchError(
            "CAPTCHA required".to_string()
        )));
        assert!(!SearchProvider::is_non_retryable(&DaedraError::Timeout));
    }

    #[tokio::test]
    async fn test_is_transient() {
        let client = reqwest::Client::new();
        let http_err =
            DaedraError::HttpError(client.get("http://127.0.0.1:1").send().await.unwrap_err());
        assert!(SearchProvider::is_transient(&http_err));
        assert!(SearchProvider::is_transient(&DaedraError::Timeout));
        assert!(!SearchProvider::is_transient(&DaedraError::SearchError(
            "not transient".to_string()
        )));
        assert!(!SearchProvider::is_transient(
            &DaedraError::BotProtectionDetected
        ));
    }

    #[test]
    fn test_backend_rate_limiters_default() {
        let limiter = BackendRateLimiters::default_limiter();
        assert!(limiter.check_key(&"bing".to_string()).is_ok());
    }

    fn test_search_result(url: &str, title: &str) -> crate::types::SearchResult {
        use crate::types::{ContentType, ResultMetadata, SearchResult};
        SearchResult {
            title: title.to_string(),
            url: url.to_string(),
            description: "desc".to_string(),
            metadata: ResultMetadata {
                content_type: ContentType::Other,
                source: "test".to_string(),
                favicon: None,
                published_date: None,
            },
        }
    }

    #[test]
    fn test_merge_ranked_interleaves_equal_scores() {
        // All results score equally against the query, so the k-way merge
        // degenerates to the old round-robin interleave.
        let a1 = test_search_result("https://a/1", "a1");
        let a2 = test_search_result("https://a/2", "a2");
        let b1 = test_search_result("https://b/1", "b1");
        let b2 = test_search_result("https://b/2", "b2");
        let by_source = vec![
            ("a".to_string(), vec![a1.clone(), a2.clone()]),
            ("b".to_string(), vec![b1.clone(), b2.clone()]),
        ];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 4);
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0].url, "https://a/1");
        assert_eq!(merged[1].url, "https://b/1");
        assert_eq!(merged[2].url, "https://a/2");
        assert_eq!(merged[3].url, "https://b/2");
    }

    #[test]
    fn test_merge_ranked_demotes_off_topic_results() {
        // Backend "rss" answers a different question than the one asked; its
        // results must land after every matched result.
        let on_topic_1 = test_search_result("https://tokio.rs/guide", "Tokio runtime guide");
        let on_topic_2 = test_search_result("https://tokio.rs/tutorial", "Tokio tutorial");
        let junk_1 = test_search_result("https://betting.example/vn", "Football betting odds");
        let junk_2 = test_search_result("https://install.example/chrome", "Chrome install");
        let by_source = vec![
            ("rss".to_string(), vec![junk_1, junk_2]),
            ("api".to_string(), vec![on_topic_1, on_topic_2]),
        ];
        let merged = SearchProvider::merge_ranked_results(&by_source, "tokio runtime", 4);
        assert_eq!(merged[0].url, "https://tokio.rs/guide");
        assert_eq!(merged[1].url, "https://tokio.rs/tutorial");
        assert_eq!(merged[2].url, "https://betting.example/vn");
        assert_eq!(merged[3].url, "https://install.example/chrome");
    }

    #[test]
    fn test_relevance_score_gives_prefix_credit() {
        // Rare names: engines answer with stem matches ("baalateja" →
        // "Baalat") and the literal token never appears. The fragment must
        // still score half, and only when it covers half the token.
        let tokens = query_tokens("baalateja");
        let r = test_search_result("https://en.wikipedia.org/wiki/Baalat_Gebal", "Baalat Gebal");
        let score = relevance_score(&r, &tokens);
        assert!((score - 0.5).abs() < 1e-9, "expected 0.5, got {score}");
        // A sliver shorter than half the token earns nothing.
        let sliver = test_search_result("https://en.wikipedia.org/wiki/Baal", "Baal - Wikipedia");
        assert_eq!(relevance_score(&sliver, &tokens), 0.0);
        // Short tokens cannot produce a meaningful fragment: no credit.
        let short = query_tokens("rust");
        let unrelated = test_search_result("https://example.com/x", "Unrelated page");
        assert_eq!(relevance_score(&unrelated, &short), 0.0);
    }

    #[test]
    fn test_merge_ranked_all_zero_still_ranks_prefix_matches() {
        let stem = test_search_result("https://en.wikipedia.org/wiki/Baalat_Gebal", "Baalat Gebal");
        let junk = test_search_result("https://betting.example/vn", "Football betting odds");
        let by_source = vec![
            ("bing-rss".to_string(), vec![junk]),
            ("wikipedia".to_string(), vec![stem]),
        ];
        let merged = SearchProvider::merge_ranked_results(&by_source, "baalateja", 2);
        // The fragment match outranks the fully unrelated result.
        assert_eq!(merged[0].url, "https://en.wikipedia.org/wiki/Baalat_Gebal");
    }

    #[test]
    fn test_query_tokens_drops_stopwords_and_short_runs() {
        let tokens = query_tokens("The capital of France");
        assert_eq!(tokens, vec!["capital".to_string(), "france".to_string()]);
        assert!(query_tokens("a of the").is_empty());
    }

    #[test]
    fn test_relevance_score_matches_url_and_neutral_empty() {
        let r = test_search_result("https://tokio.rs/runtime", "Guide");
        let tokens = query_tokens("tokio");
        // Token hit (1.0) plus full-phrase bonus in the URL (1.0).
        assert_eq!(relevance_score(&r, &tokens), 2.0);
        // No signal at all: neutral score keeps the plain round-robin order.
        assert_eq!(relevance_score(&r, &[]), 1.0);
    }

    #[test]
    fn test_canonical_url_key_dedup_and_tracking() {
        let a = canonical_url_key("https://www.Example.com/docs/guide/?utm_source=x").unwrap();
        let b = canonical_url_key("https://example.com/docs/guide").unwrap();
        assert_eq!(a, b, "www, trailing slash, utm params must not matter");
        let c = canonical_url_key("https://example.com/docs/other").unwrap();
        assert_ne!(a, c);
        assert!(canonical_url_key("not a url").is_none());
    }

    #[test]
    fn test_merge_corroboration_boost() {
        // The same page found by two engines outranks an otherwise equal
        // single-engine hit.
        let dup_a = test_search_result("https://tokio.rs/", "Tokio runtime");
        let dup_b = test_search_result("https://www.tokio.rs/?utm_source=x", "Tokio runtime");
        let other = test_search_result("https://other.test/tokio", "Tokio runtime guide");
        let by_source = vec![
            ("a".to_string(), vec![other]),
            ("b".to_string(), vec![dup_a]),
            ("c".to_string(), vec![dup_b]),
        ];
        let merged = SearchProvider::merge_ranked_results(&by_source, "tokio runtime", 3);
        assert_eq!(
            merged[0].url, "https://tokio.rs/",
            "corroborated result first"
        );
        // The www/utm variant is deduped by canonical key.
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn test_relevance_score_phrase_bonus_breaks_ties() {
        let wiki = test_search_result(
            "https://en.wikipedia.org/wiki/Capital_of_France",
            "Capital of France",
        );
        let news = test_search_result(
            "https://example.test/marseille",
            "Marseille is France's new capital of overtourism",
        );
        let tokens = query_tokens("capital of France");
        let wiki_score = relevance_score(&wiki, &tokens);
        let news_score = relevance_score(&news, &tokens);
        assert!(
            wiki_score > news_score,
            "phrase match must outrank word-bag match"
        );
    }

    #[test]
    fn test_merge_ranked_dedup() {
        let shared = test_search_result("https://dup", "dup");
        let other = test_search_result("https://other", "other");
        let by_source = vec![
            ("a".to_string(), vec![shared.clone()]),
            ("b".to_string(), vec![shared, other.clone()]),
        ];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 10);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].url, "https://dup");
        assert_eq!(merged[1].url, "https://other");
    }

    #[test]
    fn test_merge_ranked_respects_target() {
        let results: Vec<_> = (0..5)
            .map(|i| test_search_result(&format!("https://x/{}", i), &format!("r{}", i)))
            .collect();
        let by_source = vec![("x".to_string(), results)];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 3);
        assert_eq!(merged.len(), 3);
    }

    #[test]
    fn test_is_non_retryable_patterns() {
        for msg in [
            "HTTP 403 forbidden",
            "CAPTCHA required",
            "bot protection triggered",
            "bot detected on page",
        ] {
            assert!(
                SearchProvider::is_non_retryable(&DaedraError::SearchError(msg.to_string())),
                "expected non-retryable: {msg}"
            );
        }
        assert!(!SearchProvider::is_non_retryable(&DaedraError::Timeout));
        assert!(!SearchProvider::is_non_retryable(
            &DaedraError::SearchError("connection reset".to_string())
        ));
    }

    #[test]
    fn test_categorize_results_all_success() {
        use crate::types::SearchOptions;
        let opts = SearchOptions::default();
        let ok = |name: &str, url: &str| {
            (
                name.to_string(),
                Ok(SearchResponse::new(
                    "q".to_string(),
                    vec![test_search_result(url, name)],
                    &opts,
                )),
            )
        };
        let results = vec![ok("a", "https://a"), ok("b", "https://b")];
        let (by_source, any_success, tried, failures) = SearchProvider::categorize_results(results);
        assert!(any_success);
        assert_eq!(tried.len(), 2);
        assert_eq!(by_source.len(), 2);
        assert!(failures.is_empty());
    }

    #[test]
    fn test_aggregate_failure_message_all_errors() {
        let tried = vec!["bing".to_string(), "wikipedia".to_string()];
        let failures = vec![
            ("bing".to_string(), "rate limit exceeded".to_string()),
            ("wikipedia".to_string(), "HTTP status 503".to_string()),
        ];
        let msg = SearchProvider::aggregate_failure_message(&tried, &failures, &[], &[]);
        assert!(msg.contains("2 search backends failed"), "{msg}");
        assert!(msg.contains("bing: rate limit exceeded"), "{msg}");
        assert!(msg.contains("wikipedia: HTTP status 503"), "{msg}");
        assert!(!msg.contains("returned 0 results"), "{msg}");
    }

    #[test]
    fn test_aggregate_failure_message_all_empty() {
        let tried = vec!["wikipedia".to_string(), "github".to_string()];
        let msg = SearchProvider::aggregate_failure_message(&tried, &[], &tried.clone(), &[]);
        assert!(
            msg.contains("All 2 search backends returned 0 results"),
            "{msg}"
        );
    }

    #[test]
    fn test_aggregate_failure_message_mixed() {
        let tried = vec![
            "bing".to_string(),
            "wikipedia".to_string(),
            "wiby".to_string(),
        ];
        let failures = vec![("bing".to_string(), "CAPTCHA".to_string())];
        let empty = vec!["wiby".to_string()];
        let msg = SearchProvider::aggregate_failure_message(
            &tried,
            &failures,
            &empty,
            &["bing".to_string()],
        );
        assert!(msg.contains("returned no usable results"), "{msg}");
        assert!(msg.contains("failed: [bing: CAPTCHA]"), "{msg}");
        assert!(msg.contains("empty: [wiby]"), "{msg}");
        assert!(msg.contains("open circuits: [bing]"), "{msg}");
    }

    #[test]
    fn test_categorize_results_all_failure() {
        let results = vec![
            (
                "a".to_string(),
                Err(DaedraError::SearchError("fail a".to_string())),
            ),
            (
                "b".to_string(),
                Err(DaedraError::SearchError("fail b".to_string())),
            ),
        ];
        let (by_source, any_success, tried, failures) = SearchProvider::categorize_results(results);
        assert!(!any_success);
        assert_eq!(tried.len(), 2);
        assert!(by_source.is_empty());
        assert_eq!(
            failures,
            vec![
                ("a".to_string(), "Search failed: fail a".to_string()),
                ("b".to_string(), "Search failed: fail b".to_string()),
            ]
        );
    }

    #[test]
    fn test_categorize_results_mixed() {
        use crate::types::SearchOptions;
        let opts = SearchOptions::default();
        let results = vec![
            (
                "ok".to_string(),
                Ok(SearchResponse::new(
                    "q".to_string(),
                    vec![test_search_result("https://ok", "ok")],
                    &opts,
                )),
            ),
            (
                "fail".to_string(),
                Err(DaedraError::SearchError("fail".to_string())),
            ),
        ];
        let (by_source, any_success, tried, failures) = SearchProvider::categorize_results(results);
        assert!(any_success);
        assert_eq!(tried.len(), 2);
        assert_eq!(by_source.len(), 1);
        assert_eq!(by_source[0].0, "ok");
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, "fail");
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

    #[test]
    fn test_record_health_outcome_success() {
        let health = Arc::new(BackendHealth::new(3, Duration::from_secs(30)));
        health.record_failure();
        health.record_failure();
        assert!(health.is_available());
        SearchProvider::record_health_outcome(&Some(health.clone()), true);
        assert!(health.is_available());
    }

    #[test]
    fn test_record_health_outcome_failure() {
        let health = Arc::new(BackendHealth::new(3, Duration::from_secs(30)));
        health.record_failure();
        health.record_failure();
        SearchProvider::record_health_outcome(&Some(health.clone()), false);
        assert!(!health.is_available());
    }

    #[test]
    fn test_record_health_outcome_no_health() {
        SearchProvider::record_health_outcome(&None, true);
        SearchProvider::record_health_outcome(&None, false);
    }

    struct TransientThenOkBackend {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl SearchBackend for TransientThenOkBackend {
        async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                return Err(DaedraError::Timeout);
            }
            let opts = args.options.clone().unwrap_or_default();
            Ok(SearchResponse::new(
                args.query.clone(),
                vec![test_search_result("https://mock/1", "mock")],
                &opts,
            ))
        }

        fn name(&self) -> &str {
            "mock-transient"
        }
    }

    #[test]
    fn test_is_transient_rate_limit() {
        assert!(SearchProvider::is_transient(&DaedraError::SearchError(
            "HTTP 429 Too Many Requests".to_string(),
        )));
    }

    #[test]
    fn test_is_transient_timeout() {
        assert!(SearchProvider::is_transient(&DaedraError::SearchError(
            "connection timed out".to_string(),
        )));
    }

    #[test]
    fn test_is_non_retryable_bot_protection() {
        assert!(SearchProvider::is_non_retryable(
            &DaedraError::BotProtectionDetected,
        ));
    }

    #[test]
    fn test_handle_successful_result_records_health() {
        use crate::types::SearchOptions;
        let health = Arc::new(BackendHealth::new(3, Duration::from_secs(30)));
        health.record_failure();
        health.record_failure();
        let opts = SearchOptions::default();
        let ok = Ok(SearchResponse::new(
            "q".to_string(),
            vec![test_search_result("https://ok", "ok")],
            &opts,
        ));
        let (_name, _) = SearchProvider::handle_successful_result(
            "backend".to_string(),
            ok,
            Some(health.clone()),
        );
        assert!(health.is_available());
    }

    #[test]
    fn test_handle_non_retryable_records_failure() {
        let health = Arc::new(BackendHealth::new(3, Duration::from_secs(30)));
        health.record_failure();
        health.record_failure();
        let err = Err(DaedraError::BotProtectionDetected);
        let (_name, _) =
            SearchProvider::handle_non_retryable("backend".to_string(), err, Some(health.clone()));
        assert!(!health.is_available());
    }

    #[test]
    fn test_handle_unrecoverable_records_failure() {
        let health = Arc::new(BackendHealth::new(3, Duration::from_secs(30)));
        health.record_failure();
        health.record_failure();
        let err = Err(DaedraError::SearchError("unknown failure".to_string()));
        let (_name, _) = SearchProvider::handle_unrecoverable_error(
            "backend".to_string(),
            err,
            Some(health.clone()),
        );
        assert!(!health.is_available());
    }

    #[test]
    fn test_merge_ranked_results_empty() {
        let merged = SearchProvider::merge_ranked_results(&[], "query", 10);
        assert!(merged.is_empty());
    }

    #[test]
    fn test_merge_ranked_results_single_source() {
        let results: Vec<_> = (0..3)
            .map(|i| test_search_result(&format!("https://only/{}", i), &format!("r{}", i)))
            .collect();
        let by_source = vec![("only".to_string(), results.clone())];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 10);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].url, "https://only/0");
        assert_eq!(merged[1].url, "https://only/1");
        assert_eq!(merged[2].url, "https://only/2");
    }

    #[test]
    fn test_merge_ranked_results_multiple_sources() {
        let a1 = test_search_result("https://a/1", "a1");
        let b1 = test_search_result("https://b/1", "b1");
        let by_source = vec![
            ("a".to_string(), vec![a1.clone()]),
            ("b".to_string(), vec![b1.clone()]),
        ];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 2);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].url, "https://a/1");
        assert_eq!(merged[1].url, "https://b/1");
    }

    #[test]
    fn test_merge_ranked_empty_sources() {
        let merged = SearchProvider::merge_ranked_results(&[], "query", 10);
        assert!(merged.is_empty());
    }

    #[test]
    fn test_merge_ranked_single_source() {
        let results: Vec<_> = (0..3)
            .map(|i| test_search_result(&format!("https://only/{}", i), &format!("r{}", i)))
            .collect();
        let by_source = vec![("only".to_string(), results)];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 10);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].url, "https://only/0");
        assert_eq!(merged[1].url, "https://only/1");
        assert_eq!(merged[2].url, "https://only/2");
    }

    #[test]
    fn test_merge_ranked_uneven_sources() {
        let a: Vec<_> = (0..3)
            .map(|i| test_search_result(&format!("https://a/{}", i), &format!("a{}", i)))
            .collect();
        let b = vec![test_search_result("https://b/0", "b0")];
        let by_source = vec![("a".to_string(), a), ("b".to_string(), b)];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 10);
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0].url, "https://a/0");
        assert_eq!(merged[1].url, "https://b/0");
        assert_eq!(merged[2].url, "https://a/1");
        assert_eq!(merged[3].url, "https://a/2");
    }

    #[test]
    fn test_merge_ranked_all_duplicates() {
        let dup = test_search_result("https://dup", "dup");
        let by_source = vec![
            ("a".to_string(), vec![dup.clone()]),
            ("b".to_string(), vec![dup]),
        ];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 10);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].url, "https://dup");
    }

    #[test]
    fn test_merge_ranked_target_zero() {
        let results = vec![test_search_result("https://x/0", "r0")];
        let by_source = vec![("x".to_string(), results)];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 0);
        assert!(merged.is_empty());
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn test_handle_transient_error() {
        let backend = TransientThenOkBackend {
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let health = Arc::new(BackendHealth::new(3, Duration::from_secs(30)));
        let limiters = Arc::new(BackendRateLimiters::new());
        let scraper_default = BackendRateLimiters::default_limiter();
        let args = SearchArgs {
            query: "transient-retry".to_string(),
            options: None,
        };
        let first_err = backend.search(&args).await.unwrap_err();
        assert!(SearchProvider::is_transient(&first_err));

        let (_name, result) = SearchProvider::handle_transient_error(
            &backend,
            &args,
            backend.name().to_string(),
            Err(first_err),
            Some(health.clone()),
            &limiters,
            &scraper_default,
        )
        .await;
        assert!(result.is_ok());
        assert!(!result.unwrap().data.is_empty());
        assert!(health.is_available());
    }
}
