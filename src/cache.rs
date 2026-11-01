//! Caching infrastructure for Daedra.
//!
//! This module provides caching capabilities to improve performance
//! and reduce redundant network requests.

use crate::types::{PageContent, SearchResponse};
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, instrument};

/// Default cache TTL in seconds
pub const DEFAULT_CACHE_TTL_SECS: u64 = 300; // 5 minutes

/// Default maximum cache entries
pub const DEFAULT_MAX_ENTRIES: u64 = 1000;

/// Configuration for the cache
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Time-to-live for cached entries
    pub ttl: Duration,

    /// Maximum number of entries in the cache
    pub max_entries: u64,

    /// Whether caching is enabled
    pub enabled: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(DEFAULT_CACHE_TTL_SECS),
            max_entries: DEFAULT_MAX_ENTRIES,
            enabled: true,
        }
    }
}

/// Cache for search results
#[derive(Clone)]
pub struct SearchCache {
    /// Internal cache for search responses
    search_cache: Arc<Cache<String, SearchResponse>>,

    /// Internal cache for page content
    page_cache: Arc<Cache<String, PageContent>>,

    /// Whether caching is enabled
    enabled: bool,
}

impl SearchCache {
    /// Create a new search cache with the given configuration
    pub fn new(config: CacheConfig) -> Self {
        let search_cache = Cache::builder()
            .max_capacity(config.max_entries)
            .time_to_live(config.ttl)
            .build();

        let page_cache = Cache::builder()
            .max_capacity(config.max_entries)
            .time_to_live(config.ttl)
            .build();

        Self {
            search_cache: Arc::new(search_cache),
            page_cache: Arc::new(page_cache),
            enabled: config.enabled,
        }
    }

    /// Create a cache with default configuration
    pub fn with_defaults() -> Self {
        Self::new(CacheConfig::default())
    }

    /// Create a disabled cache (no-op)
    pub fn disabled() -> Self {
        Self::new(CacheConfig {
            enabled: false,
            ..Default::default()
        })
    }

    /// Generate a cache key for search queries.
    ///
    /// `backend_scope` identifies WHICH backends the request was allowed to
    /// use. It matters because a response carries the corroboration counts
    /// that set produced: asking the same query with `--backend brave` and
    /// then with no filter must not replay the single-backend answer, whose
    /// `corroborated_by` would be None or low rather than reflecting the
    /// multi-source agreement the second call would have observed.
    ///
    /// Deliberately the REQUESTED scope, not the live availability set — an
    /// availability-keyed cache would fragment on every transient circuit
    /// breaker trip and miss far more often than it should.
    fn search_key(query: &str, region: &str, safe_search: &str, backend_scope: &str) -> String {
        format!(
            "search:{}:{}:{}:{}",
            query.to_lowercase(),
            region,
            safe_search,
            backend_scope
        )
    }

    /// Canonical cache-key component for a request's backend selection.
    ///
    /// Sorted so `--backend a --backend b` and `--backend b --backend a` share
    /// an entry; `*` when no filter is applied, so the default path keys
    /// stably rather than on an arbitrary ordering.
    pub fn backend_scope(
        backends: Option<&[String]>,
        exclude_backends: Option<&[String]>,
    ) -> String {
        let fmt = |xs: Option<&[String]>| -> String {
            match xs {
                None => String::new(),
                Some(v) if v.is_empty() => String::new(),
                Some(v) => {
                    let mut s: Vec<&str> = v.iter().map(String::as_str).collect();
                    s.sort_unstable();
                    s.join(",")
                },
            }
        };
        let inc = fmt(backends);
        let exc = fmt(exclude_backends);
        if inc.is_empty() && exc.is_empty() {
            "*".to_string()
        } else {
            format!("+{inc}-{exc}")
        }
    }

    /// Generate a cache key for page content
    fn page_key(url: &str, selector: Option<&str>) -> String {
        match selector {
            Some(sel) => format!("page:{}:{}", url, sel),
            None => format!("page:{}", url),
        }
    }

    /// Get a cached search response
    #[instrument(skip(self))]
    pub async fn get_search(
        &self,
        query: &str,
        region: &str,
        safe_search: &str,
        backend_scope: &str,
    ) -> Option<SearchResponse> {
        if !self.enabled {
            return None;
        }

        let key = Self::search_key(query, region, safe_search, backend_scope);
        let result = self.search_cache.get(&key).await;

        if result.is_some() {
            debug!(query = %query, "Cache hit for search query");
        }

        result
    }

    /// Cache a search response
    #[instrument(skip(self, response))]
    pub async fn set_search(
        &self,
        query: &str,
        region: &str,
        safe_search: &str,
        backend_scope: &str,
        response: SearchResponse,
    ) {
        if !self.enabled {
            return;
        }

        let key = Self::search_key(query, region, safe_search, backend_scope);
        self.search_cache.insert(key, response).await;
        debug!(query = %query, "Cached search response");
    }

    /// Get cached page content
    #[instrument(skip(self))]
    pub async fn get_page(&self, url: &str, selector: Option<&str>) -> Option<PageContent> {
        if !self.enabled {
            return None;
        }

        let key = Self::page_key(url, selector);
        let result = self.page_cache.get(&key).await;

        if result.is_some() {
            debug!(url = %url, "Cache hit for page content");
        }

        result
    }

    /// Cache page content
    #[instrument(skip(self, content))]
    pub async fn set_page(&self, url: &str, selector: Option<&str>, content: PageContent) {
        if !self.enabled {
            return;
        }

        let key = Self::page_key(url, selector);
        self.page_cache.insert(key, content).await;
        debug!(url = %url, "Cached page content");
    }

    /// Clear all cached entries
    pub async fn clear(&self) {
        self.search_cache.invalidate_all();
        self.page_cache.invalidate_all();
        debug!("Cache cleared");
    }

    /// Get statistics about the cache
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            search_entries: self.search_cache.entry_count(),
            page_entries: self.page_cache.entry_count(),
            enabled: self.enabled,
        }
    }
}

impl Default for SearchCache {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Statistics about the cache
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Number of cached search responses
    pub search_entries: u64,

    /// Number of cached page contents
    pub page_entries: u64,

    /// Whether caching is enabled
    pub enabled: bool,
}

impl std::fmt::Display for CacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Cache Stats: {} search entries, {} page entries (enabled: {})",
            self.search_entries, self.page_entries, self.enabled
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentType, ResultMetadata, SearchOptions, SearchResult};

    #[tokio::test]
    async fn test_cache_search() {
        let cache = SearchCache::with_defaults();

        let results = vec![SearchResult {
            title: "Test".to_string(),
            url: "https://example.com".to_string(),
            description: "Test description".to_string(),
            metadata: ResultMetadata {
                content_type: ContentType::Article,
                source: "example.com".to_string(),
                favicon: None,
                published_date: None, ..Default::default() },
        }];

        let options = SearchOptions::default();
        let response = SearchResponse::new("test".to_string(), results, &options);

        // Initially empty
        assert!(
            cache
                .get_search("test", "wt-wt", "MODERATE", "*")
                .await
                .is_none()
        );

        // Set and get
        cache
            .set_search("test", "wt-wt", "MODERATE", "*", response.clone())
            .await;
        let cached = cache.get_search("test", "wt-wt", "MODERATE", "*").await;
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().data.len(), 1);
    }

    #[tokio::test]
    async fn test_cache_page() {
        let cache = SearchCache::with_defaults();

        let content = PageContent {
            url: "https://example.com".to_string(),
            title: "Test Page".to_string(),
            content: "# Hello World".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            word_count: 2,
            content_type: None,
            links: None,
        };

        // Initially empty
        assert!(cache.get_page("https://example.com", None).await.is_none());

        // Set and get
        cache
            .set_page("https://example.com", None, content.clone())
            .await;
        let cached = cache.get_page("https://example.com", None).await;
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().title, "Test Page");
    }

    #[tokio::test]
    async fn test_disabled_cache() {
        let cache = SearchCache::disabled();

        let results = vec![];
        let options = SearchOptions::default();
        let response = SearchResponse::new("test".to_string(), results, &options);

        cache
            .set_search("test", "wt-wt", "MODERATE", "*", response)
            .await;
        assert!(
            cache
                .get_search("test", "wt-wt", "MODERATE", "*")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_cache_stats() {
        let cache = SearchCache::with_defaults();
        let stats = cache.stats();
        assert_eq!(stats.search_entries, 0);
        assert_eq!(stats.page_entries, 0);
        assert!(stats.enabled);
    }
}

#[cfg(test)]
mod backend_scope_tests {
    use super::SearchCache;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unscoped_requests_share_a_stable_key() {
        // The default path must not key on an arbitrary ordering or on None.
        assert_eq!(SearchCache::backend_scope(None, None), "*");
        assert_eq!(SearchCache::backend_scope(Some(&[]), Some(&[])), "*");
    }

    #[test]
    fn backend_order_does_not_change_the_key() {
        let a = SearchCache::backend_scope(Some(&v(&["brave", "hn"])), None);
        let b = SearchCache::backend_scope(Some(&v(&["hn", "brave"])), None);
        assert_eq!(a, b, "same set in different order must share an entry");
    }

    #[test]
    fn different_backend_sets_do_not_collide() {
        // This is the bug: a single-backend answer must never be replayed for
        // a query that asked for all backends, because corroborated_by would
        // report the narrower set's agreement.
        let single = SearchCache::backend_scope(Some(&v(&["brave"])), None);
        let all = SearchCache::backend_scope(None, None);
        assert_ne!(
            single, all,
            "a --backend brave answer must not be served for an unfiltered query"
        );
    }

    #[test]
    fn exclusions_are_part_of_the_key() {
        let none = SearchCache::backend_scope(None, None);
        let excl = SearchCache::backend_scope(None, Some(&v(&["google"])));
        assert_ne!(none, excl);
    }
}
