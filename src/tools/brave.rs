//! Brave Search HTML backend — no API key needed.
//!
//! Scrapes Brave's HTML results page. Brave serves a real general index and
//! tolerates residential IPs, but rate-limits or challenges datacenter IPs
//! (HTTP 429). The circuit breaker sidelines it where blocked. Honors
//! SafeSearch via Brave's `safesearch` parameter.

use super::backend::SearchBackend;
use super::soft_block::{self, EmptyPage};
use crate::types::{
    ContentType, DaedraError, DaedraError as E, DaedraResult, ResultMetadata, SafeSearchLevel,
    SearchArgs, SearchResponse, SearchResult,
};
use async_trait::async_trait;
use lazy_static::lazy_static;
use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use std::time::Duration;
use tracing::{info, warn};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const BRAVE_URL: &str = "https://search.brave.com/search";

lazy_static! {
    /// Web result cards. Brave's DOM marks them `data-type="web"` inside the
    /// `#search-page` column; the plain `.snippet` fallback covers layout
    /// drift, and a bare page-wide `.snippet` keeps older layouts working.
    ///
    /// The container id is load-bearing: Brave moved from `#results` to
    /// `#search-page`, and a stale container anchor makes every query parse
    /// as zero results while still looking like a genuine empty page.
    static ref RESULT_SELECTOR: Selector = Selector::parse(
        "#search-page .snippet[data-type='web'], \
         #search-page .snippet, \
         #results .snippet, \
         .snippet[data-type='web']"
    )
    .unwrap();
    static ref TITLE_SELECTORS: [Selector; 4] = [
        Selector::parse(".title").unwrap(),
        Selector::parse(".search-snippet-title").unwrap(),
        Selector::parse("a .url").unwrap(),
        Selector::parse("a[href] .snippet-title").unwrap(),
    ];
    static ref LINK_SELECTOR: Selector = Selector::parse("a[href]").unwrap();
    static ref SNIPPET_SELECTORS: [Selector; 4] = [
        Selector::parse(".generic-snippet .content").unwrap(),
        Selector::parse(".snippet-description").unwrap(),
        Selector::parse(".snippet-content").unwrap(),
        Selector::parse(".snippet-description-text").unwrap(),
    ];
}

fn safe_param(level: SafeSearchLevel) -> Option<&'static str> {
    match level {
        SafeSearchLevel::Off => Some("off"),
        SafeSearchLevel::Moderate => Some("moderate"),
        SafeSearchLevel::Strict => Some("strict"),
    }
}

fn is_brave_internal(url: &str) -> bool {
    url.contains("search.brave.com")
        || url.starts_with("/search?")
        || url.starts_with("/images?")
        || url.starts_with("/news?")
        || url.starts_with("/goggles?")
}

fn extract_brave_result(element: &ElementRef) -> Option<SearchResult> {
    // The broad fallback selector matches news/image cards too; the
    // data-type attribute is the authority when present.
    if let Some(dt) = element.value().attr("data-type")
        && dt != "web"
    {
        return None;
    }
    let link_el = element.select(&LINK_SELECTOR).find(|a| {
        a.value()
            .attr("href")
            .is_some_and(|href| href.starts_with("http") && !is_brave_internal(href))
    })?;

    let url = link_el.value().attr("href").unwrap_or_default();

    // Brave puts the title in a text node inside the title element, or as
    // the link's own text.
    let mut title = String::new();
    for sel in TITLE_SELECTORS.iter() {
        if let Some(t) = element.select(sel).next() {
            title = t.text().collect();
            if !title.trim().is_empty() {
                break;
            }
        }
    }
    if title.trim().is_empty() {
        title = link_el.text().collect();
    }
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
            source: "brave".to_string(),
            favicon: None,
            published_date: None,
        },
    })
}

/// Brave HTML scraping backend — general index, no API key.
pub struct BraveBackend {
    client: Client,
    base_url: String,
}

impl BraveBackend {
    /// Create a new Brave backend instance.
    pub fn new() -> Self {
        Self::with_base_url(BRAVE_URL.to_string())
    }

    /// Point at a custom endpoint (tests).
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

    /// Parse a Brave SERP page into results (exposed for regression tests).
    pub fn parse_results(&self, html: &str, max_results: usize) -> Vec<SearchResult> {
        let document = Html::parse_document(html);
        document
            .select(&RESULT_SELECTOR)
            .filter_map(|e| extract_brave_result(&e))
            .take(max_results)
            .collect()
    }
}

impl Default for BraveBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for BraveBackend {
    fn name(&self) -> &str {
        "brave"
    }

    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();

        let mut query: Vec<(&str, String)> =
            vec![("q", args.query.clone()), ("source", "web".to_string())];
        if let Some(safe) = safe_param(opts.safe_search) {
            query.push(("safesearch", safe.to_string()));
        }

        let resp = self
            .client
            .get(&self.base_url)
            .query(&query)
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        let status = resp.status();
        if !status.is_success() {
            warn!(status = %status, "Brave returned non-200");
            if status.as_u16() == 429 {
                return Err(E::RateLimitExceeded);
            }
            return Err(DaedraError::SearchError(format!("Brave status {status}")));
        }

        let html = resp.text().await.map_err(DaedraError::HttpError)?;

        let results = self.parse_results(&html, opts.num_results);

        if results.is_empty() {
            match soft_block::classify(&html, &["no results", "no matches"]) {
                EmptyPage::GenuineNoResults => {
                    info!("Brave genuinely reports no results for this query");
                },
                EmptyPage::SoftBlock => {
                    warn!("Brave returned 200 with zero results and no no-results marker");
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

    const FIXTURE: &str = r#"<html><body><div id="results">
      <div class="snippet" data-type="web">
        <a href="https://tokio.rs/"><span class="title">Tokio - An asynchronous Rust runtime</span></a>
        <div class="snippet-content"><span class="snippet-description">A runtime for reliable async apps.</span></div>
      </div>
      <div class="snippet" data-type="web">
        <a href="https://docs.rs/tokio"><span class="title">tokio - Rust</span></a>
        <div class="snippet-content"><span class="snippet-description">Latest crate docs.</span></div>
      </div>
      <div class="snippet" data-type="news">
        <a href="https://brave.com/news/x"><span class="title">Brave news internal</span></a>
      </div>
    </div></body></html>"#;

    #[test]
    fn test_parse_brave_results() {
        let backend = BraveBackend::new();
        let results = backend.parse_results(FIXTURE, 10);
        assert_eq!(
            results.len(),
            2,
            "internal and non-web cards must be skipped"
        );
        assert_eq!(results[0].url, "https://tokio.rs/");
        assert_eq!(results[0].metadata.source, "brave");
        assert!(results[0].description.contains("reliable async"));
        assert_eq!(results[1].url, "https://docs.rs/tokio");
    }

    #[test]
    fn test_parse_empty_page() {
        let backend = BraveBackend::new();
        assert!(
            backend
                .parse_results(
                    "<html><body><div id='search-page'><p>no results</p></div></body></html>",
                    10
                )
                .is_empty()
        );
    }

    /// Markup copied from a live Brave SERP: cards live under `#search-page`
    /// and the description sits in `.generic-snippet .content`. A parser
    /// anchored to the retired `#results` container returns nothing here.
    const LIVE_FIXTURE: &str = r#"<html><body><main id="search-page"><div class="serp-columns">
      <div class="snippet svelte-jmfu5f" data-pos="1" data-type="web">
        <div class="result-body"><div class="result-content">
          <a href="https://www.wikipedia.org/" class="l1">
            <div class="site-name-content"><cite class="snippet-url">wikipedia.org</cite></div>
            <div class="title search-snippet-title line-clamp-1">Wikipedia, the free encyclopedia</div>
          </a>
          <div class="generic-snippet"><div class="content desktop-default-regular">Wikipedia is a free online encyclopedia, created and edited by volunteers.</div></div>
        </div></div>
      </div>
      <div class="snippet svelte-jmfu5f" data-pos="2" data-type="web">
        <div class="result-body"><div class="result-content">
          <a href="https://docs.rs/tokio" class="l1">
            <div class="title search-snippet-title">tokio - Rust</div>
          </a>
          <div class="generic-snippet"><div class="content">Latest crate docs.</div></div>
        </div></div>
      </div>
    </div></main></body></html>"#;

    #[test]
    fn test_parse_live_brave_layout() {
        let backend = BraveBackend::new();
        let results = backend.parse_results(LIVE_FIXTURE, 10);
        assert_eq!(
            results.len(),
            2,
            "cards under #search-page must be parsed; a #results-only selector yields 0"
        );
        assert_eq!(results[0].url, "https://www.wikipedia.org/");
        assert_eq!(results[0].title, "Wikipedia, the free encyclopedia");
        assert!(
            results[0].description.contains("free online encyclopedia"),
            "description must come from .generic-snippet .content, got {:?}",
            results[0].description
        );
        assert_eq!(results[1].url, "https://docs.rs/tokio");
        assert!(results[1].description.contains("Latest crate docs"));
    }
}
