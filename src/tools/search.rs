//! DuckDuckGo search implementation.
//!
//! This module provides web search functionality using DuckDuckGo's
//! HTML interface. Note: DDG blocks datacenter/VPS IPs since mid-2025.
//! Use as fallback only — prefer Bing/Serper/Tavily backends.

use super::backend::SearchBackend;
use crate::types::{
    ContentType, DaedraError, DaedraResult, ResultMetadata, SearchArgs, SearchOptions,
    SearchResponse, SearchResult,
};
use async_trait::async_trait;
use backoff::{ExponentialBackoff, future::retry};
use futures::future::join_all;
use lazy_static::lazy_static;
use regex::Regex;
use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
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
    async fn execute_search_with_retry(&self, params: &[(&str, String)]) -> DaedraResult<String> {
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
            if let Some(result) = extract_result_from_element(&element) {
                results.push(result);
            }
        }

        if results.is_empty() {
            warn!("No search results found in response");
        }

        Ok(results)
    }
}

/// Extract a single search result from a DDG result div element.
pub(crate) fn extract_result_from_element(element: &ElementRef) -> Option<SearchResult> {
    let title_element = element.select(&TITLE_SELECTOR).next()?;

    let title = clean_text(&title_element.text().collect::<String>());
    let href = title_element.value().attr("href")?;
    let url = extract_actual_url(href);

    if url.is_empty() || !url.starts_with("http") {
        return None;
    }

    let description = element
        .select(&SNIPPET_SELECTOR)
        .next()
        .map(|el| clean_text(&el.text().collect::<String>()))
        .unwrap_or_default();

    let content_type = detect_content_type(&url);
    let source = extract_domain(&url);

    Some(SearchResult {
        title,
        url,
        description,
        metadata: ResultMetadata {
            content_type,
            source,
            favicon: None,
            published_date: None,
        },
    })
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
        && let Some(decoded) = encoded_url.split('&').next()
    {
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
    crate::url_classification::classify_search_url(url)
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

// Implement SearchBackend trait for DDG
#[async_trait]
impl SearchBackend for SearchClient {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        self.search(args).await
    }

    fn name(&self) -> &str { "duckduckgo" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_actual_url() {
        // Test DuckDuckGo redirect URL
        let ddg_url = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpath&rut=abc";
        assert_eq!(extract_actual_url(ddg_url), "https://example.com/path");

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
        assert_eq!(
            extract_domain("https://www.example.com/path"),
            "www.example.com"
        );
        assert_eq!(
            extract_domain("https://docs.rust-lang.org"),
            "docs.rust-lang.org"
        );
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

    #[test]
    fn test_extract_actual_url_no_uddg() {
        let direct = "https://example.com/page";
        assert_eq!(extract_actual_url(direct), direct);
    }

    fn extract_from_result_html(html: &str) -> Option<SearchResult> {
        let fragment = Html::parse_fragment(html);
        let element = fragment.select(&RESULT_SELECTOR).next()?;
        extract_result_from_element(&element)
    }

    #[test]
    fn test_extract_result_from_element_valid() {
        let html = r#"<div class="result"><a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com&rut=abc" class="result__a">Example Title</a><a class="result__snippet">Example snippet</a></div>"#;
        let result = extract_from_result_html(html).unwrap();
        assert_eq!(result.title, "Example Title");
        assert_eq!(result.url, "https://example.com");
        assert_eq!(result.description, "Example snippet");
    }

    #[test]
    fn test_extract_result_from_element_no_title() {
        let html = r#"<div class="result"><a class="result__snippet">Snippet only</a></div>"#;
        assert!(extract_from_result_html(html).is_none());
    }

    #[test]
    fn test_extract_result_from_element_no_href() {
        let html = r#"<div class="result"><a class="result__a">Title without href</a></div>"#;
        assert!(extract_from_result_html(html).is_none());
    }

    #[test]
    fn test_extract_result_from_element_invalid_url() {
        let html = r#"<div class="result"><a href="/not-http" class="result__a">Bad URL</a></div>"#;
        assert!(extract_from_result_html(html).is_none());
    }

    #[test]
    fn test_parse_search_results_empty_html() {
        let client = SearchClient::new().unwrap();
        let results = client.parse_search_results("<html><body></body></html>", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_search_results_with_results() {
        let html = r#"<div class="result"><a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com&rut=abc" class="result__a">Example Title</a><a class="result__snippet">Example snippet</a></div>"#;
        let client = SearchClient::new().unwrap();
        let results = client.parse_search_results(html, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Example Title");
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[0].description, "Example snippet");
    }

    #[test]
    fn test_parse_search_results_respects_max() {
        let mut html = String::new();
        for i in 0..5 {
            html.push_str(&format!(
                r#"<div class="result"><a href="https://example{i}.com" class="result__a">Title {i}</a><a class="result__snippet">Snippet {i}</a></div>"#
            ));
        }
        let client = SearchClient::new().unwrap();
        let results = client.parse_search_results(&html, 2).unwrap();
        assert_eq!(results.len(), 2);
    }
}
