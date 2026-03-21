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

        // DDG — blocked from most datacenter IPs, last resort
        info!("DuckDuckGo backend enabled (last resort)");
        backends.push(Box::new(super::search::SearchClient::new().unwrap()));

        Self { backends }
    }

    /// Search using the fallback chain. Tries each backend in order.
    pub async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let mut last_error = None;

        for backend in &self.backends {
            if !backend.is_available() {
                continue;
            }

            info!(backend = backend.name(), query = %args.query, "Trying search backend");

            match backend.search(args).await {
                Ok(response) if !response.data.is_empty() => {
                    info!(
                        backend = backend.name(),
                        results = response.data.len(),
                        "Search succeeded"
                    );
                    return Ok(response);
                }
                Ok(response) => {
                    warn!(
                        backend = backend.name(),
                        "Search returned 0 results, trying next backend"
                    );
                    last_error = Some(crate::types::DaedraError::SearchError(
                        format!("{}: returned 0 results", backend.name()),
                    ));
                }
                Err(e) => {
                    warn!(
                        backend = backend.name(),
                        error = %e,
                        "Search backend failed, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            crate::types::DaedraError::SearchError("No search backends available".into())
        }))
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
        // Should always have at least Bing, Wikipedia, StackExchange, GitHub, DDG
        assert!(backends.len() >= 5, "Expected at least 5 backends, got {}", backends.len());
        assert!(backends.contains(&"bing"));
        assert!(backends.contains(&"wikipedia"));
        assert!(backends.contains(&"stackoverflow"));
        assert!(backends.contains(&"github"));
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
