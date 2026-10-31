//! [`SearchProvider`]: the concurrent multi-backend fan-out, timeouts,
//! error classification, and result merge.

use crate::types::{DaedraError, DaedraResult, SearchArgs, SearchResponse};
use governor::{DefaultDirectRateLimiter, DefaultKeyedRateLimiter, Quota, RateLimiter};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use super::backend_trait::SearchBackend;

type CategorizedResults = (
    Vec<(String, Vec<crate::types::SearchResult>)>,
    bool,
    Vec<String>,
    Vec<(String, String)>,
);
use super::health::BackendHealth;
use super::limits::BackendRateLimiters;
use super::rank::{canonical_url_key, query_tokens, relevance_score};

/// Hard ceiling on a single backend's contribution to one search.
///
/// The fan-out is concurrent, so total search latency is bounded by the
/// slowest backend rather than the sum. That makes an unresponsive endpoint
/// a whole-search latency problem, not a local one, and the ceiling is set
/// from the measured spread rather than a round number: on 2026-10-31 the
/// ten healthy backends answered in 27-931 ms while marginalia's public API
/// never answered at all (curl `http=000` after 40 s, three times). A 5 s
/// ceiling is ~5x the slowest healthy backend, so it never truncates a
/// working one, and it caps what a dead one can cost.
///
/// The abandoned backend is charged a circuit-breaker failure, so a
/// persistently slow endpoint eventually stops being asked.
const BACKEND_DEADLINE: Duration = Duration::from_secs(5);

/// Aggregate soft deadline: once at least one backend has produced usable
/// results, stop waiting for stragglers.
///
/// Set from measurement, not from a round number. Measured per-backend
/// wall-clock on 2026-10-31 for one query:
///
///   bing-rss 271ms (10) | stackoverflow 329 (10) | gnews 370 (9)
///   github 509 (10) | wikipedia 525 (10) | hn 627 (9) | wiby 899 (8)
///   --- above this line every backend that CAN answer has answered ---
///   brave 883 (0) | bing 252 (0) | google 136 (0) | ddg-instant 89 (0)
///   marginalia 3007 (0)
///
/// The slowest backend that returns anything useful is 899 ms; the only
/// backend beyond it produced zero results after 3 s. A 5 s soft deadline —
/// the value used by the design this was modelled on, whose backend mix is
/// different — never fires on this host, so searches paid 3007 ms for
/// nothing. 1500 ms keeps ~600 ms of headroom over the slowest useful
/// backend and still abandons a hung one.
const SOFT_DEADLINE: Duration = Duration::from_millis(1_500);

/// Aggregate hard deadline: return whatever exists, even an empty set, so no
/// single backend can pin the call.
///
/// NOTE: with BACKEND_DEADLINE at 5s this is a BACKSTOP that rarely fires —
/// every per-backend future resolves within 5s, so the collector normally
/// finishes before 30s. It exists so that a future change (or an injected
/// deadline in tests) cannot leave a search hanging indefinitely.
const HARD_DEADLINE: Duration = Duration::from_secs(30);

/// Multi-backend search provider with automatic fallback.
pub struct SearchProvider {
    backends: Vec<Box<dyn SearchBackend>>,
    /// Limits how fast aggregate searches are issued (avoids tripping scraper rate limits).
    rate_limiter: DefaultDirectRateLimiter,
    backend_limiters: DefaultKeyedRateLimiter<String>,
    backend_rate_limits: Arc<BackendRateLimiters>,
    circuit_breakers: HashMap<String, Arc<BackendHealth>>,
    /// Aggregate deadlines. Defaults are the production constants; tests
    /// override them so BOTH phases are reachable. Without the seam the hard
    /// deadline can never fire, because BACKEND_DEADLINE resolves every
    /// backend first and the two-phase collector always exits early.
    soft_deadline: Duration,
    hard_deadline: Duration,
    /// Per-backend deadline. Injectable so tests can drive the per-backend
    /// timeout path without waiting the production 5 s.
    backend_deadline: Duration,
}

impl SearchProvider {
    pub(super) fn new_rate_limiter() -> DefaultDirectRateLimiter {
        // ~6 searches per 10s sustained: 1 cell per ~1.67s, burst of 6
        RateLimiter::direct(
            Quota::with_period(Duration::from_millis(167))
                .expect("167ms period is valid")
                .allow_burst(NonZeroU32::new(6).unwrap()),
        )
    }

    pub(super) fn new_backend_limiters() -> DefaultKeyedRateLimiter<String> {
        BackendRateLimiters::default_limiter()
    }

    pub(super) fn init_circuit_breakers(
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

    pub(super) fn from_backends(backends: Vec<Box<dyn SearchBackend>>) -> Self {
        let circuit_breakers = Self::init_circuit_breakers(&backends);
        Self {
            backends,
            rate_limiter: Self::new_rate_limiter(),
            backend_limiters: Self::new_backend_limiters(),
            backend_rate_limits: BackendRateLimiters::new(),
            circuit_breakers,
            soft_deadline: SOFT_DEADLINE,
            hard_deadline: HARD_DEADLINE,
            backend_deadline: BACKEND_DEADLINE,
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
        backends.push(Box::new(crate::tools::mwmbl::MwmblBackend::new()));

        // Brave HTML — general index, no API key; 429s from datacenter IPs.
        info!("Brave backend enabled (no API key, may rate-limit datacenter IPs)");
        backends.push(Box::new(crate::tools::brave::BraveBackend::new()));

        // Marginalia public API — independent non-commercial index, answers
        // from any IP. Strong for docs and technical content.
        info!("Marginalia backend enabled (no API key, answers from any IP)");
        backends.push(Box::new(crate::tools::marginalia::MarginaliaBackend::new()));

        // Bing via its machine-readable RSS output — same index as the HTML
        // SERP, served to integrations without challenge pages.
        info!("Bing RSS backend enabled (no API key, bot-tolerant machine format)");
        backends.push(Box::new(crate::tools::rss::BingRssBackend::new()));

        // Bing HTML scraping — no API key, but often CAPTCHA-blocked from datacenter IPs
        info!("Bing backend enabled (no API key, may be blocked from datacenter IPs)");
        backends.push(Box::new(crate::tools::bing::BingBackend::new()));

        // Google News RSS — news coverage, no challenges.
        info!("Google News backend enabled (no API key, bot-tolerant machine format)");
        backends.push(Box::new(crate::tools::rss::GoogleNewsBackend::new()));

        // Hacker News via the public Algolia API — no key, strong for technical queries.
        info!("Hacker News backend enabled (no API key, Algolia JSON)");
        backends.push(Box::new(crate::tools::rss::HnAlgoliaBackend::new()));

        // Google HTML scraping — no API key, but CAPTCHA-prone from datacenter IPs
        info!("Google backend enabled (no API key, may be blocked from datacenter IPs)");
        backends.push(Box::new(crate::tools::google::GoogleBackend::new()));

        // Wikipedia — always works from any IP, knowledge-focused
        info!("Wikipedia backend enabled (always works, knowledge-focused)");
        backends.push(Box::new(crate::tools::wikipedia::WikipediaBackend::new()));

        // StackExchange — always works from any IP, technical Q&A
        info!("StackExchange backend enabled (always works, technical)");
        backends.push(Box::new(
            crate::tools::stackexchange::StackExchangeBackend::new(),
        ));

        // GitHub — always works, code/repo search
        info!("GitHub backend enabled (always works, code/repos)");
        backends.push(Box::new(crate::tools::github::GitHubBackend::new()));

        // Wiby — indie web search, always works
        info!("Wiby backend enabled (always works, indie web)");
        backends.push(Box::new(crate::tools::wiby::WibyBackend::new()));

        // DDG Instant Answers — knowledge graph, always works (different from HTML scraping)
        info!("DDG Instant Answers backend enabled (always works, knowledge)");
        backends.push(Box::new(crate::tools::ddg_instant::DdgInstantBackend::new()));

        // DDG HTML scraping — blocked from most datacenter IPs, last resort
        info!("DuckDuckGo HTML backend enabled (last resort)");
        backends.push(Box::new(crate::tools::search::SearchClient::new().unwrap()));

        Self::from_backends(backends)
    }

    const NON_RETRYABLE_SUBSTRINGS: &[&str] = &["403", "captcha", "bot protection", "bot detected"];

    pub(super) fn is_non_retryable(err: &DaedraError) -> bool {
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

    /// Whether an error is worth retrying once.
    ///
    /// A TIMEOUT IS NOT TRANSIENT. Retrying a request that already burned its
    /// whole client timeout buys one more full timeout and nothing else: the
    /// backend is unreachable or overloaded, and the second attempt has the
    /// same budget as the first.
    ///
    /// Measured on 2026-10-31: marginalia's public API answered `http=000`
    /// after 40 s (curl, repeated), failing at its 8 s client timeout. Because
    /// `Timeout`/`HttpError` were classed transient, every search paid 8 s for
    /// the attempt plus another 8 s for the retry — observed as a flat 12 s
    /// per search (the provider's BACKEND_DEADLINE cutting the second attempt
    /// short). With timeouts excluded the first attempt's 8 s is the whole
    /// cost, and the circuit breaker counts it so a dead backend stops being
    /// asked altogether.
    ///
    /// Genuine transients — connection refused/reset, broken pipe, an
    /// incomplete body — keep their single retry.
    pub(super) fn is_transient(err: &DaedraError) -> bool {
        match err {
            DaedraError::Timeout => false,
            // reqwest classifies its own failures; `is_timeout` is the crate's
            // own answer and is exact, where matching on the message text is a
            // guess at a string another crate owns.
            DaedraError::HttpError(e) => !e.is_timeout(),
            DaedraError::SearchError(msg) => {
                let m = msg.to_lowercase();
                Self::TRANSIENT_SUBSTRINGS.iter().any(|s| m.contains(s))
            },
            _ => false,
        }
    }

    pub(super) fn record_health_outcome(health: &Option<Arc<BackendHealth>>, success: bool) {
        if let Some(h) = health {
            if success {
                h.record_success();
            } else {
                h.record_failure();
            }
        }
    }

    pub(super) fn handle_successful_result(
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

    pub(super) fn handle_non_retryable(
        name: String,
        result: DaedraResult<SearchResponse>,
        health: Option<Arc<BackendHealth>>,
    ) -> (String, DaedraResult<SearchResponse>) {
        Self::record_health_outcome(&health, false);
        (name, result)
    }

    pub(super) async fn retry_once(
        b: &dyn SearchBackend,
        args: &SearchArgs,
    ) -> DaedraResult<SearchResponse> {
        // One retry after a fixed 400 ms pause. The caller classifies errors;
        // only transient errors reach this function.
        tokio::time::sleep(Duration::from_millis(400)).await;
        b.search(args).await
    }

    pub(super) async fn handle_transient_error(
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

    pub(super) fn handle_unrecoverable_error(
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

    pub(super) async fn query_backend(
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
    pub(super) fn collect_queryable_backends(&self) -> Vec<&dyn SearchBackend> {
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

    /// Override the aggregate deadlines (test seam).
    pub(super) fn with_deadlines(mut self, soft: Duration, hard: Duration) -> Self {
        self.soft_deadline = soft;
        self.hard_deadline = hard;
        self
    }

    /// Override the per-backend deadline (test seam).
    pub(super) fn with_backend_deadline(mut self, d: Duration) -> Self {
        self.backend_deadline = d;
        self
    }

    /// Circuit-breaker state for one backend by name (test seam).
    ///
    /// Returns the SAME Arc the fan-out charges, so a test observes exactly
    /// what the deadline path recorded rather than a copy.
    #[cfg(test)]
    pub(super) fn breaker_for(&self, name: &str) -> Option<Arc<BackendHealth>> {
        self.circuit_breakers.get(name).cloned()
    }

    /// Trait-object view of the registered backends (test seam).
    pub(super) fn backend_refs(&self) -> Vec<&dyn SearchBackend> {
        self.backends.iter().map(|b| b.as_ref()).collect()
    }

    pub(super) async fn execute_concurrent_queries(
        &self,
        backends: &[&dyn SearchBackend],
        args: &SearchArgs,
    ) -> Vec<(String, DaedraResult<SearchResponse>)> {
        use futures::stream::{FuturesUnordered, StreamExt};

        let limiters = Arc::clone(&self.backend_rate_limits);
        let scraper_default = &self.backend_limiters;

        // FuturesUnordered rather than join_all: join_all resolves only when
        // EVERY future does, so wrapping it in a timeout still waits for the
        // slowest backend. A stream lets the collector return the moment the
        // deadline logic says so, abandoning the rest.
        let mut pending: FuturesUnordered<_> = backends
            .iter()
            .map(|b| {
                let a = args.clone();
                let name = b.name().to_string();
                let health = self.circuit_breakers.get(b.name()).cloned();
                let limiters = Arc::clone(&limiters);
                let backend_deadline = self.backend_deadline;
                async move {
                    match tokio::time::timeout(
                        backend_deadline,
                        Self::query_backend(*b, &a, health, &limiters, scraper_default),
                    )
                    .await
                    {
                        Ok(pair) => pair,
                        Err(_) => {
                            if let Some(h) = self.circuit_breakers.get(&name) {
                                h.record_failure();
                            }
                            warn!(
                                backend = %name,
                                deadline_s = backend_deadline.as_secs(),
                                "Backend exceeded per-backend deadline, abandoned"
                            );
                            let msg = format!(
                                "Backend {} exceeded the {}s deadline",
                                name,
                                backend_deadline.as_secs()
                            );
                            (name, Err(DaedraError::SearchError(msg)))
                        },
                    }
                }
            })
            .collect();

        let total = pending.len();
        let mut collected: Vec<(String, DaedraResult<SearchResponse>)> = Vec::with_capacity(total);
        let mut answered = 0usize;
        let mut soft_done = false;

        let soft = tokio::time::sleep(self.soft_deadline);
        tokio::pin!(soft);
        let hard = tokio::time::sleep(self.hard_deadline);
        tokio::pin!(hard);

        loop {
            tokio::select! {
                biased;

                Some(pair) = pending.next() => {
                    if matches!(&pair.1, Ok(r) if !r.data.is_empty()) {
                        answered += 1;
                    }
                    collected.push(pair);
                    if collected.len() == total {
                        break;
                    }
                },

                // One arm only: an `if answered == 0` guard plus a second
                // `&mut soft` arm would borrow it twice.
                () = &mut soft, if !soft_done => {
                    soft_done = true;
                    if answered == 0 {
                        info!(
                            waiting = pending.len(),
                            soft_ms = self.soft_deadline.as_millis(),
                            "Soft deadline reached with no usable backend; waiting for hard deadline"
                        );
                    } else {
                        info!(
                            answered,
                            abandoned = pending.len(),
                            soft_ms = self.soft_deadline.as_millis(),
                            "Soft deadline reached with usable results; returning without stragglers"
                        );
                        break;
                    }
                },

                () = &mut hard => {
                    // Charge the stragglers. Without this, a backend that is
                    // slow enough to be abandoned by the AGGREGATE deadline
                    // escapes the circuit breaker entirely, because only the
                    // per-backend timeout path charges — and the aggregate
                    // deadline can preempt it. Measured by
                    // `backend_exceeding_deadline_is_charged_a_circuit_failure`:
                    // with hard < per-backend, the abandoned future returned
                    // zero entries and the breaker stayed closed.
                    // Charge the stragglers: an endpoint that is always too
                    // slow must eventually stop being asked, and a backend
                    // that vanishes from the result set reads as "it answered
                    // nothing" — a different, misleading claim from "we
                    // stopped waiting". Only the per-backend timeout path
                    // charged before, so the aggregate deadline could preempt
                    // it and escape the breaker entirely.
                    // Owned copies: `collected` is borrowed immutably here and
                    // mutated below.
                    let seen: std::collections::HashSet<String> =
                        collected.iter().map(|(n, _)| n.clone()).collect();
                    let mut abandoned = 0usize;
                    for b in backends {
                        let n = b.name();
                        if seen.contains(n) {
                            continue;
                        }
                        abandoned += 1;
                        if let Some(h) = self.circuit_breakers.get(n) {
                            h.record_failure();
                        }
                        collected.push((
                            n.to_string(),
                            Err(DaedraError::SearchError(format!(
                                "Backend {n} was abandoned at the {}ms aggregate deadline",
                                self.hard_deadline.as_millis()
                            ))),
                        ));
                    }
                    warn!(
                        collected = collected.len(),
                        abandoned,
                        hard_ms = self.hard_deadline.as_millis(),
                        "Hard deadline reached; returning partial results"
                    );
                    break;
                },
            }
        }

        collected
    }

    pub(super) fn categorize_results(
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
    pub(super) fn aggregate_failure_message(
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
    pub(super) fn merge_ranked_results(
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

        // Parse Google-style directives (`site:`, `intitle:`, `filetype:`,
        // `after:`) out of the raw query ONCE. Measured 2026-10-31: every
        // backend was handed the raw string, none understood it, and nothing
        // reported that the filter had not applied — `site:github.com`
        // returned tokio.rs and `-site:github.com` returned github.com.
        let structured = super::query::parse(&args.query);

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

        // Backends receive the free text plus the operators they might honor;
        // enforcement happens below on the merged set.
        let mut backend_args = args.clone();
        backend_args.query = super::query::format_for_engine(&structured);

        let results = self
            .execute_concurrent_queries(&queryable, &backend_args)
            .await;
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

        // Constraints are enforced on the merged set BEFORE the relevance
        // discard path, and scoring uses the CLEAN free text.
        //
        // Both orderings matter and both were measured wrong:
        //   1. Scoring read the raw query, so `site`, `github` and `com` became
        //      relevance tokens. That is why `site:github.com tokio async
        //      runtime` surfaced tokio.rs: the domain words scored like content.
        //   2. The constraint filter ran AFTER the "no results matched" discard
        //      path, so a relaxed constraint could never surface as a note when
        //      the discard fired first.
        let filtered = super::query::apply_constraints(
            &structured,
            Self::merge_ranked_results(&by_source, &structured.text, target_count),
        );
        let mut merged = filtered.sources;
        let relaxed = filtered.relaxed;
        if !relaxed.is_empty() {
            info!(relaxed = ?relaxed, "Query constraints could not be satisfied and were relaxed");
        }

        // Empty free text (a directives-only query) carries no relevance
        // signal; scoring it would reject everything for the wrong reason.
        let tokens = query_tokens(&structured.text);
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
                // A relaxed constraint is a NOTE, never this error: the
                // caller's filter was unsatisfiable, which is different from
                // the engines returning nothing usable.
                if relaxed.is_empty() {
                    return Err(DaedraError::SearchError(format!(
                        "No search results matched the query {:?}; {} unrelated result(s) from \
                         [{}] were discarded. Try fewer or different keywords.",
                        args.query,
                        merged.len(),
                        tried.join(", ")
                    )));
                }
            }
        }

        let sources: Vec<String> = by_source.iter().map(|(n, _)| n.clone()).collect();
        info!(
            total = merged.len(),
            sources = ?sources,
            "Aggregated results from {} backends",
            sources.len()
        );

        Ok(
            SearchResponse::new(args.query.clone(), merged, &opts)
                .with_relaxed_constraints(relaxed),
        )
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
