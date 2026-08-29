//! Google HTML search backend — no API key needed.
//!
//! Scrapes Google's HTML search results page. CAPTCHA-prone from
//! datacenter IPs; the fallback chain treats bot protection as a
//! fail-fast error so the next backend takes over. Honors the
//! SafeSearch level and region from [`SearchOptions`].

use super::backend::SearchBackend;
use crate::types::{
    region_to_gl_hl, ContentType, DaedraError, DaedraResult, ResultMetadata, SafeSearchLevel,
    SearchArgs, SearchResponse, SearchResult,
};
use async_trait::async_trait;
use lazy_static::lazy_static;
use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use std::time::Duration;
use tracing::{info, warn};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const GOOGLE_URL: &str = "https://www.google.com/search";

lazy_static! {
    static ref RESULT_SELECTOR: Selector = Selector::parse("div.g").unwrap();
    static ref LINK_SELECTOR: Selector = Selector::parse("a[href]").unwrap();
    static ref SNIPPET_SELECTOR: Selector = Selector::parse("div.VwiC3b").unwrap();
}

/// Convert a SafeSearch level to Google's `safe` query parameter.
/// Moderate is Google's default and is expressed by omitting the param.
pub fn safe_param(level: SafeSearchLevel) -> Option<&'static str> {
    match level {
        SafeSearchLevel::Off => Some("off"),
        SafeSearchLevel::Moderate => None,
        SafeSearchLevel::Strict => Some("active"),
    }
}

fn is_google_internal_url(url: &str) -> bool {
    url.contains("google.")
        || url.starts_with("/search?")
        || url.starts_with("/url?")
        || url.contains("webcache.googleusercontent.com")
}

fn extract_google_result(element: &ElementRef) -> Option<SearchResult> {
    let link_el = element
        .select(&LINK_SELECTOR)
        .find(|a| {
            a.value()
                .attr("href")
                .is_some_and(|href| href.starts_with("http") && !is_google_internal_url(href))
        })?;

    let url = link_el.value().attr("href").unwrap_or_default();
    let title: String = link_el.text().collect();

    if title.trim().is_empty() || !url.starts_with("http") {
        return None;
    }

    let description: String = element
        .select(&SNIPPET_SELECTOR)
        .next()
        .map(|e| e.text().collect())
        .unwrap_or_default();

    Some(SearchResult {
        title: title.trim().to_string(),
        url: url.to_string(),
        description: description.trim().to_string(),
        metadata: ResultMetadata {
            content_type: ContentType::Other,
            source: "google".to_string(),
            favicon: None,
            published_date: None,
        },
    })
}

/// Google HTML scraping backend — parses search results from google.com SERP pages.
pub struct GoogleBackend {
    client: Client,
    base_url: String,
}

impl GoogleBackend {
    /// Create a new Google backend instance.
    pub fn new() -> Self {
        Self::with_base_url(GOOGLE_URL.to_string())
    }

    /// Create a backend pointed at a custom search endpoint (tests).
    pub fn with_base_url(base_url: String) -> Self {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .gzip(true)
            .brotli(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .expect("Failed to build HTTP client");
        Self { client, base_url }
    }

    /// Parse a Google SERP HTML page into results (exposed for regression tests).
    pub fn parse_results(&self, html: &str, max_results: usize) -> Vec<SearchResult> {
        let document = Html::parse_document(html);
        document
            .select(&RESULT_SELECTOR)
            .filter_map(|e| extract_google_result(&e))
            .take(max_results)
            .collect()
    }
}

impl Default for GoogleBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for GoogleBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();
        let (gl, hl) = region_to_gl_hl(&opts.region);

        let mut query: Vec<(&str, String)> = vec![
            ("q", args.query.clone()),
            ("num", opts.num_results.to_string()),
            ("pws", "0".to_string()),
        ];
        if let Some(safe) = safe_param(opts.safe_search) {
            query.push(("safe", safe.to_string()));
        }
        if let Some(gl) = &gl {
            query.push(("gl", gl.clone()));
        }
        if let Some(hl) = &hl {
            query.push(("hl", hl.clone()));
        }

        let resp = self
            .client
            .get(&self.base_url)
            .query(&query)
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        if !resp.status().is_success() {
            warn!(status = %resp.status(), "Google returned non-200");
            return Err(DaedraError::SearchError(format!(
                "Google status {}",
                resp.status()
            )));
        }

        let html = resp.text().await.map_err(DaedraError::HttpError)?;

        if html.contains("unusual traffic") || html.contains("g-recaptcha") {
            warn!("Google served a CAPTCHA page");
            return Err(DaedraError::BotProtectionDetected);
        }

        let results = self.parse_results(&html, opts.num_results);

        if results.is_empty() {
            warn!("Google returned 0 results — may be blocked or CAPTCHA");
        }

        info!(
            backend = "google",
            results = results.len(),
            "Google search complete"
        );
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }

    fn name(&self) -> &str {
        "google"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_from_result_html(html: &str) -> Option<SearchResult> {
        let fragment = Html::parse_fragment(html);
        let element = fragment.select(&RESULT_SELECTOR).next()?;
        extract_google_result(&element)
    }

    #[test]
    fn test_extract_google_result_valid() {
        let html = r#"
            <div class="g">
                <a href="https://example.com/article">Example Article</a>
                <div class="VwiC3b">A snippet of the article.</div>
            </div>
        "#;
        let r = extract_from_result_html(html).expect("valid result expected");
        assert_eq!(r.title, "Example Article");
        assert_eq!(r.url, "https://example.com/article");
        assert_eq!(r.description, "A snippet of the article.");
        assert_eq!(r.metadata.source, "google");
    }

    #[test]
    fn test_extract_google_result_skips_internal_links() {
        let html = r#"
            <div class="g">
                <a href="/search?q=related">Related searches</a>
                <a href="https://example.com/real">Real Result</a>
            </div>
        "#;
        let r = extract_from_result_html(html).expect("non-internal link expected");
        assert_eq!(r.url, "https://example.com/real");
    }

    #[test]
    fn test_extract_google_result_empty() {
        let html = r#"<div class="g"><a href="https://example.com"></a></div>"#;
        assert!(extract_from_result_html(html).is_none());
    }

    #[test]
    fn test_safe_param_levels() {
        assert_eq!(safe_param(SafeSearchLevel::Off), Some("off"));
        assert_eq!(safe_param(SafeSearchLevel::Moderate), None);
        assert_eq!(safe_param(SafeSearchLevel::Strict), Some("active"));
    }
}
