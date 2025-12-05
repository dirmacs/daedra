//! DuckDuckGo search implementation.
//!
//! This module provides web search functionality using DuckDuckGo's
//! HTML interface to avoid API rate limits.

use crate::types::{
    ContentType, DaedraError, DaedraResult, ResultMetadata, SearchArgs, SearchOptions,
    SearchResponse, SearchResult,
};
use backoff::{future::retry, ExponentialBackoff};
use futures::future::join_all;
use lazy_static::lazy_static;
use regex::Regex;
use reqwest::Client;
use scraper::{Html, Selector};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, instrument, warn};
use url::Url;

/// Default user agent for requests
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// DuckDuckGo HTML search URL
const DDG_HTML_URL: &str = "https://html.duckduckgo.com/html/";

/// Request timeout
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum concurrent requests for parallel processing
const MAX_CONCURRENT_REQUESTS: usize = 5;

lazy_static! {
    /// Selector for search results
    static ref RESULT_SELECTOR: Selector = Selector::parse("div.result").unwrap();

    /// Selector for result title
    static ref TITLE_SELECTOR: Selector = Selector::parse("a.result__a").unwrap();

    /// Selector for result snippet
    static ref SNIPPET_SELECTOR: Selector = Selector::parse("a.result__snippet").unwrap();

    /// Regex for cleaning HTML entities
    static ref HTML_ENTITY_REGEX: Regex = Regex::new(r"&#x([0-9a-fA-F]+);").unwrap();

    /// Regex for domain extraction
    static ref DOMAIN_REGEX: Regex = Regex::new(r"^(?:https?://)?([^/]+)").unwrap();
}

/// HTTP client for making search requests
#[derive(Clone)]
pub struct SearchClient {
    client: Client,
}

impl SearchClient {
    /// Create a new search client
    pub fn new() -> DaedraResult<Self> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .gzip(true)
            .brotli(true)
            .build()
            .map_err(DaedraError::HttpError)?;

        Ok(Self { client })
    }

    /// Perform a DuckDuckGo search
    #[instrument(skip(self), fields(query = %args.query))]
    pub async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let options = args.options.clone().unwrap_or_default();

        info!(query = %args.query, region = %options.region, "Performing search");

        // Build search parameters
        let params = self.build_search_params(&args.query, &options);

        // Execute search with retry
        let html = self.execute_search_with_retry(&params).await?;

        // Parse results
        let results = self.parse_search_results(&html, options.num_results)?;

        info!(
            query = %args.query,
            result_count = results.len(),
            "Search completed"
        );

        Ok(SearchResponse::new(args.query.clone(), results, &options))
    }

    /// Build search parameters for the request
    fn build_search_params(&self, query: &str, options: &SearchOptions) -> Vec<(&str, String)> {
        let mut params = vec![
            ("q", query.to_string()),
            ("kl", options.region.clone()),
            ("kp", options.safe_search.to_ddg_value().to_string()),
        ];

        // Add time range if specified
        if let Some(ref time_range) = options.time_range {
            params.push(("df", time_range.clone()));
        }

        params
    }

    /// Execute search with exponential backoff retry
    async fn execute_search_with_retry(
        &self,
        params: &[(&str, String)],
    ) -> DaedraResult<String> {
        let backoff = ExponentialBackoff {
            max_elapsed_time: Some(Duration::from_secs(60)),
            ..Default::default()
        };

        let client = self.client.clone();
        let params_owned: Vec<(String, String)> = params
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();

        retry(backoff, || async {
            let response = client
                .post(DDG_HTML_URL)
                .form(&params_owned)
                .send()
                .await
                .map_err(|e| {
                    warn!(error = %e, "Search request failed, retrying...");
                    backoff::Error::transient(DaedraError::HttpError(e))
                })?;

            if !response.status().is_success() {
                let status = response.status();
                warn!(status = %status, "Search returned non-success status");

                if status.as_u16() == 429 {
                    return Err(backoff::Error::transient(DaedraError::RateLimitExceeded));
                }

                return Err(backoff::Error::permanent(DaedraError::SearchError(
                    format!("HTTP {}", status),
                )));
            }

            response.text().await.map_err(|e| {
                error!(error = %e, "Failed to read response body");
                backoff::Error::permanent(DaedraError::HttpError(e))
            })
        })
        .await
    }

    /// Parse search results from HTML response
    fn parse_search_results(
        &self,
        html: &str,
        max_results: usize,
    ) -> DaedraResult<Vec<SearchResult>> {
        let document = Html::parse_document(html);
        let mut results = Vec::new();

        for element in document.select(&RESULT_SELECTOR) {
            if results.len() >= max_results {
                break;
            }

            // Extract title and URL
            let title_element = match element.select(&TITLE_SELECTOR).next() {
                Some(el) => el,
                None => continue,
            };

            let title = clean_text(&title_element.text().collect::<String>());
            let url = match title_element.value().attr("href") {
                Some(href) => extract_actual_url(href),
                None => continue,
            };

            // Skip invalid URLs
            if url.is_empty() || !url.starts_with("http") {
                continue;
            }

            // Extract snippet
            let description = element
                .select(&SNIPPET_SELECTOR)
                .next()
                .map(|el| clean_text(&el.text().collect::<String>()))
                .unwrap_or_default();

            // Detect content type and extract source
            let content_type = detect_content_type(&url);
            let source = extract_domain(&url);

            results.push(SearchResult {
                title,
                url,
                description,
                metadata: ResultMetadata {
                    content_type,
                    source,
                    favicon: None,
                    published_date: None,
                },
            });
        }

        if results.is_empty() {
            warn!("No search results found in response");
        }

        Ok(results)
    }
}

impl Default for SearchClient {
    fn default() -> Self {
        Self::new().expect("Failed to create default search client")
    }
}

/// Perform a search using the provided arguments
///
/// # Arguments
///
/// * `args` - Search arguments including query and options
///
/// # Returns
///
/// A `SearchResponse` containing the search results and metadata
///
/// # Example
///
/// ```rust,no_run
/// use daedra::{SearchArgs, tools::search::perform_search};
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let args = SearchArgs {
///         query: "Rust programming".to_string(),
///         options: None,
///     };
///     let results = perform_search(&args).await?;
///     println!("Found {} results", results.data.len());
///     Ok(())
/// }
/// ```
pub async fn perform_search(args: &SearchArgs) -> DaedraResult<SearchResponse> {
    let client = SearchClient::new()?;
    client.search(args).await
}

/// Perform multiple searches in parallel
///
/// # Arguments
///
/// * `queries` - Vector of search arguments
///
/// # Returns
///
/// Vector of search responses (or errors) for each query
pub async fn perform_parallel_searches(
    queries: Vec<SearchArgs>,
) -> Vec<DaedraResult<SearchResponse>> {
    let client = Arc::new(SearchClient::new().expect("Failed to create search client"));

    // Process in batches to respect rate limits
    let mut all_results = Vec::with_capacity(queries.len());

    for chunk in queries.chunks(MAX_CONCURRENT_REQUESTS) {
        let futures: Vec<_> = chunk
            .iter()
            .map(|args| {
                let client = Arc::clone(&client);
                let args = args.clone();
                async move { client.search(&args).await }
            })
            .collect();

        let chunk_results = join_all(futures).await;
        all_results.extend(chunk_results);

        // Small delay between batches to be respectful
        if !queries.is_empty() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    all_results
}

/// Extract the actual URL from DuckDuckGo's redirect URL
fn extract_actual_url(href: &str) -> String {
    // DuckDuckGo wraps URLs in a redirect format
    // Example: //duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com
    if href.contains("uddg=")
        && let Some(encoded_url) = href.split("uddg=").nth(1)
            && let Some(decoded) = encoded_url.split('&').next() {
                return urlencoding::decode(decoded)
                    .map(|s| s.into_owned())
                    .unwrap_or_else(|_| href.to_string());
            }

    // Handle direct URLs
    if href.starts_with("//") {
        return format!("https:{}", href);
    }

    href.to_string()
}

/// Detect content type based on URL patterns
fn detect_content_type(url: &str) -> ContentType {
    let lower_url = url.to_lowercase();

    // Documentation sites
    if lower_url.contains("docs.")
        || lower_url.contains("/docs/")
        || lower_url.contains("/documentation/")
        || lower_url.contains("readthedocs")
        || lower_url.contains("javadoc")
        || lower_url.contains("/api/")
    {
        return ContentType::Documentation;
    }

    // Code hosting and Q&A
    if lower_url.contains("github.com")
        || lower_url.contains("gitlab.com")
        || lower_url.contains("stackoverflow.com")
        || lower_url.contains("stackexchange.com")
        || lower_url.contains("bitbucket.org")
    {
        return ContentType::Documentation;
    }

    // Social media
    if lower_url.contains("twitter.com")
        || lower_url.contains("x.com")
        || lower_url.contains("facebook.com")
        || lower_url.contains("linkedin.com")
        || lower_url.contains("instagram.com")
        || lower_url.contains("tiktok.com")
    {
        return ContentType::Social;
    }

    // Forums
    if lower_url.contains("reddit.com")
        || lower_url.contains("forum")
        || lower_url.contains("discourse")
        || lower_url.contains("community.")
    {
        return ContentType::Forum;
    }

    // Video platforms
    if lower_url.contains("youtube.com")
        || lower_url.contains("youtu.be")
        || lower_url.contains("vimeo.com")
        || lower_url.contains("twitch.tv")
    {
        return ContentType::Video;
    }

    // Shopping
    if lower_url.contains("amazon.")
        || lower_url.contains("ebay.")
        || lower_url.contains("shop.")
        || lower_url.contains("/shop/")
        || lower_url.contains("store.")
    {
        return ContentType::Shopping;
    }

    // News sites (common patterns)
    if lower_url.contains("news.")
        || lower_url.contains("/news/")
        || lower_url.contains("bbc.")
        || lower_url.contains("cnn.")
        || lower_url.contains("nytimes.")
        || lower_url.contains("reuters.")
    {
        return ContentType::Article;
    }

    ContentType::Article
}

/// Extract domain from URL
fn extract_domain(url: &str) -> String {
    Url::parse(url)
        .map(|u| u.host_str().unwrap_or("unknown").to_string())
        .unwrap_or_else(|_| {
            DOMAIN_REGEX
                .captures(url)
                .and_then(|caps| caps.get(1))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        })
}

/// Clean text by removing HTML entities and extra whitespace
fn clean_text(text: &str) -> String {
    let mut cleaned = text.to_string();

    // Decode HTML entities
    cleaned = cleaned
        .replace("&#x27;", "'")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");

    // Handle hex entities
    cleaned = HTML_ENTITY_REGEX
        .replace_all(&cleaned, |caps: &regex::Captures| {
            let hex = &caps[1];
            u32::from_str_radix(hex, 16)
                .ok()
                .and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_else(|| caps[0].to_string())
        })
        .to_string();

    // Normalize whitespace
    cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_actual_url() {
        // Test DuckDuckGo redirect URL
        let ddg_url = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpath&rut=abc";
        assert_eq!(
            extract_actual_url(ddg_url),
            "https://example.com/path"
        );

        // Test protocol-relative URL
        let relative_url = "//example.com/path";
        assert_eq!(extract_actual_url(relative_url), "https://example.com/path");

        // Test direct URL
        let direct_url = "https://example.com";
        assert_eq!(extract_actual_url(direct_url), "https://example.com");
    }

    #[test]
    fn test_detect_content_type() {
        assert_eq!(
            detect_content_type("https://docs.rust-lang.org/book/"),
            ContentType::Documentation
        );
        assert_eq!(
            detect_content_type("https://github.com/rust-lang/rust"),
            ContentType::Documentation
        );
        assert_eq!(
            detect_content_type("https://twitter.com/rustlang"),
            ContentType::Social
        );
        assert_eq!(
            detect_content_type("https://reddit.com/r/rust"),
            ContentType::Forum
        );
        assert_eq!(
            detect_content_type("https://youtube.com/watch?v=123"),
            ContentType::Video
        );
        assert_eq!(
            detect_content_type("https://amazon.com/product"),
            ContentType::Shopping
        );
        assert_eq!(
            detect_content_type("https://example.com"),
            ContentType::Article
        );
    }

    #[test]
    fn test_extract_domain() {
        assert_eq!(extract_domain("https://www.example.com/path"), "www.example.com");
        assert_eq!(extract_domain("https://docs.rust-lang.org"), "docs.rust-lang.org");
        // For non-URL strings, the regex still extracts the text as a potential domain
        assert_eq!(extract_domain("invalid"), "invalid");
        // Empty string should return unknown
        assert_eq!(extract_domain(""), "unknown");
    }

    #[test]
    fn test_clean_text() {
        assert_eq!(clean_text("Hello&#x27;s World"), "Hello's World");
        assert_eq!(clean_text("Hello &amp; World"), "Hello & World");
        assert_eq!(clean_text("  Multiple   spaces  "), "Multiple spaces");
        assert_eq!(clean_text("&lt;html&gt;"), "<html>");
    }

    #[test]
    fn test_search_params() {
        let client = SearchClient::new().unwrap();
        let options = SearchOptions {
            region: "us-en".to_string(),
            safe_search: crate::types::SafeSearchLevel::Strict,
            num_results: 10,
            time_range: Some("w".to_string()),
        };

        let params = client.build_search_params("test query", &options);

        assert!(params.iter().any(|(k, v)| *k == "q" && v == "test query"));
        assert!(params.iter().any(|(k, v)| *k == "kl" && v == "us-en"));
        assert!(params.iter().any(|(k, v)| *k == "df" && v == "w"));
    }
}
