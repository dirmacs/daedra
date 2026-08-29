//! Mojeek search backend — no API key needed.
//!
//! Scrapes Mojeek's HTML results page. Mojeek runs an independent crawler
//! and index, is bot-tolerant from residential IPs, and refuses datacenter
//! IPs outright (plain 403). The fallback chain handles both cases: 403 is
//! a fail-fast error and the circuit breaker sidelines the backend where it
//! is blocked. Honors SafeSearch via Mojeek's `safe` parameter.

use super::backend::SearchBackend;
use super::soft_block::{self, EmptyPage};
use crate::types::{
    ContentType, DaedraError, DaedraResult, ResultMetadata, SafeSearchLevel, SearchArgs,
    SearchResponse, SearchResult,
};
use async_trait::async_trait;
use lazy_static::lazy_static;
use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use std::time::Duration;
use tracing::{info, warn};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const MOJEEK_URL: &str = "https://www.mojeek.com/search";

/// The headers a real browser sends on a top-level navigation. Mojeek's bot
/// wall has two layers: untrusted networks get a plain 403, while trusted
/// networks with a non-browser client (plain reqwest TLS fingerprint) get a
/// silent non-HTML 200 that parses as zero results. The full header set is
/// the cheapest honest way to look like the browser the UA claims to be.
fn browser_default_headers() -> reqwest::header::HeaderMap {
    use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, HeaderMap, HeaderName, HeaderValue};
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        ),
    );
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
    headers.insert(
        HeaderName::from_static("upgrade-insecure-requests"),
        HeaderValue::from_static("1"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-dest"),
        HeaderValue::from_static("document"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-mode"),
        HeaderValue::from_static("navigate"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("none"),
    );
    headers
}

lazy_static! {
    /// Mojeek's organic result items. The list class has been stable for
    /// years; both selectors must fail before a page counts as empty.
    static ref RESULT_SELECTOR: Selector = Selector::parse("ul.results-standard > li, li.results-standard").unwrap();
    /// Result title link (class `ob`) with fallbacks for layout shifts.
    static ref TITLE_SELECTORS: [Selector; 3] = [
        Selector::parse("a.ob").unwrap(),
        Selector::parse("h2 a").unwrap(),
        Selector::parse("a.title").unwrap(),
    ];
    /// Result snippet paragraph (class `s`).
    static ref SNIPPET_SELECTORS: [Selector; 2] = [
        Selector::parse("p.s").unwrap(),
        Selector::parse("p.desc").unwrap(),
    ];
}

fn safe_param(level: SafeSearchLevel) -> Option<&'static str> {
    match level {
        SafeSearchLevel::Off => Some("0"),
        SafeSearchLevel::Moderate => None,
        SafeSearchLevel::Strict => Some("1"),
    }
}

fn extract_mojeek_result(element: &ElementRef) -> Option<SearchResult> {
    let mut link = None;
    for sel in TITLE_SELECTORS.iter() {
        if let Some(a) = element.select(sel).find(|a| {
            a.value()
                .attr("href")
                .is_some_and(|h| h.starts_with("http"))
        }) {
            link = Some(a);
            break;
        }
    }
    let link_el = link?;

    let url = link_el.value().attr("href").unwrap_or_default();
    let title: String = link_el.text().collect();
    if title.trim().is_empty() || !url.starts_with("http") {
        return None;
    }

    let description: String = SNIPPET_SELECTORS
        .iter()
        .find_map(|sel| element.select(sel).next())
        .map(|e| e.text().collect())
        .unwrap_or_default();

    Some(SearchResult {
        title: title.trim().to_string(),
        url: url.to_string(),
        description: description.trim().to_string(),
        metadata: ResultMetadata {
            content_type: ContentType::Other,
            source: "mojeek".to_string(),
            favicon: None,
            published_date: None,
        },
    })
}

/// Mojeek HTML scraping backend — independent index, no API key.
pub struct MojeekBackend {
    client: Client,
    base_url: String,
}

impl MojeekBackend {
    /// Create a new Mojeek backend instance.
    pub fn new() -> Self {
        Self::with_base_url(MOJEEK_URL.to_string())
    }

    /// Point at a custom endpoint (tests).
    pub fn with_base_url(base_url: String) -> Self {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .default_headers(browser_default_headers())
            .timeout(Duration::from_secs(30))
            .gzip(true)
            .brotli(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .expect("Failed to build HTTP client");
        Self { client, base_url }
    }

    /// Parse a Mojeek SERP page into results (exposed for regression tests).
    pub fn parse_results(&self, html: &str, max_results: usize) -> Vec<SearchResult> {
        let document = Html::parse_document(html);
        document
            .select(&RESULT_SELECTOR)
            .filter_map(|e| extract_mojeek_result(&e))
            .take(max_results)
            .collect()
    }
}

impl Default for MojeekBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for MojeekBackend {
    fn name(&self) -> &str {
        "mojeek"
    }

    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();

        let mut query: Vec<(&str, String)> = vec![
            ("q", args.query.clone()),
            ("tlen", opts.num_results.to_string()),
        ];
        if let Some(safe) = safe_param(opts.safe_search) {
            query.push(("safe", safe.to_string()));
        }

        let resp = self
            .client
            .get(&self.base_url)
            .query(&query)
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        if !resp.status().is_success() {
            warn!(status = %resp.status(), "Mojeek returned non-200");
            return Err(DaedraError::SearchError(format!(
                "Mojeek status {}",
                resp.status()
            )));
        }

        // A 200 that is not HTML is Mojeek's quiet bot wall for trusted
        // networks: the request reached it, but its client fingerprinting
        // did not clear the automated client. Say so instead of reporting
        // an unparseable zero-result page.
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_lowercase();
        if !content_type.contains("text/html") && !content_type.contains("application/xhtml") {
            warn!(content_type = %content_type, "Mojeek served a non-HTML response");
            return Err(DaedraError::SearchError(format!(
                "Mojeek served {content_type} instead of HTML — its bot wall flagged the \
                 automated client; results are unavailable from this machine"
            )));
        }

        let html = resp.text().await.map_err(DaedraError::HttpError)?;

        let results = self.parse_results(&html, opts.num_results);

        if results.is_empty() {
            match soft_block::classify(
                &html,
                &[
                    "no results found",
                    "did not match",
                    // Mojeek's empty result list carries this class; the
                    // current stylesheet (v2.119) still ships `.no-results`.
                    "no-results",
                    "there are no results",
                    "no results for",
                ],
            ) {
                EmptyPage::GenuineNoResults => {
                    info!("Mojeek genuinely reports no results for this query");
                },
                EmptyPage::SoftBlock => {
                    warn!("Mojeek returned 200 with zero results and no no-results marker");
                    return Err(DaedraError::BotProtectionDetected);
                },
            }
        }

        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"<html><body><ul class="results-standard">
      <li><a class="ob" href="https://tokio.rs/">Tokio - An asynchronous Rust runtime</a>
          <p class="s">A runtime for writing reliable asynchronous applications.</p></li>
      <li><h2><a href="https://docs.rs/tokio">tokio - docs.rs</a></h2>
          <p class="s">A event-driven, non-blocking I/O platform.</p></li>
      <li><a class="ob" href="/search?q=next">Next page</a></li>
    </ul></body></html>"#;

    #[test]
    fn test_parse_mojeek_results() {
        let backend = MojeekBackend::new();
        let results = backend.parse_results(FIXTURE, 10);
        assert_eq!(results.len(), 2, "internal links must be skipped");
        assert_eq!(results[0].url, "https://tokio.rs/");
        assert_eq!(results[0].metadata.source, "mojeek");
        assert!(results[0].description.contains("reliable asynchronous"));
        // Fallback title selector (h2 a) also parses.
        assert_eq!(results[1].url, "https://docs.rs/tokio");
    }

    #[test]
    fn test_parse_empty_page() {
        let backend = MojeekBackend::new();
        assert!(
            backend
                .parse_results("<html><body><p>no results found</p></body></html>", 10)
                .is_empty()
        );
    }
}
