//! Search backend trait and multi-backend provider.
//!
//! Daedra supports multiple search backends with automatic fallback:
//! - Bing HTML scraping (default, no API key needed)
//! - Serper.dev (Google results via API, needs SERPER_API_KEY)
//! - Tavily (AI-optimized search, needs TAVILY_API_KEY)
//! - DuckDuckGo HTML scraping (blocked from datacenter IPs, fallback only)

use crate::types::{DaedraResult, SearchArgs, SearchResponse};
use async_trait::async_trait;
use tracing::{info, warn};

/// Trait for search backends. Each backend implements web search
/// and returns results in the common SearchResponse format.
#[async_trait]
pub trait SearchBackend: Send + Sync {
    /// Execute a search query and return results.
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse>;

    /// Backend name for logging and diagnostics.
    fn name(&self) -> &str;

    /// Whether this backend requires an API key.
    fn requires_api_key(&self) -> bool { false }

    /// Whether this backend is available (has required config/keys).
    fn is_available(&self) -> bool { true }
}

/// Multi-backend search provider with automatic fallback.
///
/// Tries backends in priority order. If the primary fails,
/// falls back to the next available backend.
pub struct SearchProvider {
    backends: Vec<Box<dyn SearchBackend>>,
}

impl SearchProvider {
    /// Create a new provider with the given backends (in priority order).
    pub fn new(backends: Vec<Box<dyn SearchBackend>>) -> Self {
        Self { backends }
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

        Self { backends }
    }

    /// Aggregate search across ALL available backends.
    ///
    /// Queries all backends concurrently, merges results, deduplicates by URL,
    /// and interleaves sources for diversity (Wikipedia, StackOverflow, GitHub
    /// results mixed rather than grouped).
    pub async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();
        let target_count = opts.num_results;

        // Query all available backends concurrently
        let futures: Vec<_> = self.backends.iter()
            .filter(|b| b.is_available())
            .map(|b| {
                let a = args.clone();
                let name = b.name().to_string();
                async move {
                    info!(backend = %name, query = %a.query, "Querying backend");
                    (name, b.search(&a).await)
                }
            })
            .collect();

        let results = futures::future::join_all(futures).await;

        // Collect all successful results grouped by backend
        let mut by_source: Vec<(String, Vec<crate::types::SearchResult>)> = Vec::new();
        let mut any_success = false;

        for (name, result) in results {
            match result {
                Ok(response) if !response.data.is_empty() => {
                    info!(backend = %name, count = response.data.len(), "Backend returned results");
                    any_success = true;
                    by_source.push((name, response.data));
                }
                Ok(_) => {
                    warn!(backend = %name, "Backend returned 0 results");
                }
                Err(e) => {
                    warn!(backend = %name, error = %e, "Backend failed");
                }
            }
        }

        if !any_success {
            return Err(crate::types::DaedraError::SearchError(
                "All search backends returned 0 results".into(),
            ));
        }

        // Interleave results from different sources for diversity
        // Round-robin: take 1 from each source, repeat until we have enough
        let mut merged: Vec<crate::types::SearchResult> = Vec::new();
        let mut seen_urls: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut indices: Vec<usize> = vec![0; by_source.len()];

        loop {
            let mut added_this_round = false;
            for (i, (_name, results)) in by_source.iter().enumerate() {
                if merged.len() >= target_count { break; }
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
            if !added_this_round || merged.len() >= target_count { break; }
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
        self.backends.iter()
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
        assert!(backends.len() >= 7, "Expected at least 7 backends, got {}", backends.len());
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
        assert!(response.is_ok(), "Fallback chain should find results from at least one backend");
        let data = response.unwrap();
        assert!(!data.data.is_empty(), "Should have at least 1 result");
    }
}
