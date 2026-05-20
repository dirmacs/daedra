//! Bing HTML search backend — no API key needed.
//!
//! Scrapes Bing's HTML search results page. More permissive than
//! Google/DDG for datacenter IPs. Default backend for self-hosted use.

use super::backend::SearchBackend;
use crate::types::{
    ContentType, DaedraError, DaedraResult, ResultMetadata, SearchArgs, SearchResponse,
    SearchResult,
};
use async_trait::async_trait;
use lazy_static::lazy_static;
use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use std::time::Duration;
use tracing::{info, warn};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const BING_URL: &str = "https://www.bing.com/search";

lazy_static! {
    static ref RESULT_SELECTOR: Selector = Selector::parse("li.b_algo").unwrap();
    static ref TITLE_SELECTOR: Selector = Selector::parse("h2 a").unwrap();
    static ref SNIPPET_SELECTOR: Selector = Selector::parse(".b_caption p, .b_lineclamp2").unwrap();
}

fn extract_bing_result(element: &ElementRef) -> Option<SearchResult> {
    let title_el = element.select(&TITLE_SELECTOR).next()?;

    let title: String = title_el.text().collect();
    let url = title_el.value().attr("href").unwrap_or_default();

    if title.trim().is_empty() || !url.starts_with("http") {
        return None;
    }
    if url.contains("bing.com/ck/") {
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
            source: "bing".to_string(),
            favicon: None,
            published_date: None,
        },
    })
}

pub struct BingBackend {
    client: Client,
}

impl BingBackend {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .gzip(true)
            .brotli(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .expect("Failed to build HTTP client");
        Self { client }
    }

    fn parse_results(&self, html: &str, max_results: usize) -> Vec<SearchResult> {
        let document = Html::parse_document(html);
        document
            .select(&RESULT_SELECTOR)
            .filter_map(|e| extract_bing_result(&e))
            .take(max_results)
            .collect()
    }
}

#[async_trait]
impl SearchBackend for BingBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();

        let resp = self.client
            .get(BING_URL)
            .query(&[
                ("q", args.query.as_str()),
                ("count", &opts.num_results.to_string()),
            ])
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        if !resp.status().is_success() {
            warn!(status = %resp.status(), "Bing returned non-200");
            return Err(DaedraError::SearchError(format!("Bing status {}", resp.status())));
        }

        let html = resp.text().await.map_err(DaedraError::HttpError)?;
        let results = self.parse_results(&html, opts.num_results);

        if results.is_empty() {
            warn!("Bing returned 0 results — may be blocked or CAPTCHA");
        }

        info!(backend = "bing", results = results.len(), "Bing search complete");
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }

    fn name(&self) -> &str { "bing" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_from_result_html(html: &str) -> Option<SearchResult> {
        let fragment = Html::parse_fragment(html);
        let element = fragment.select(&RESULT_SELECTOR).next()?;
        extract_bing_result(&element)
    }

    #[test]
    fn test_extract_bing_result_valid() {
        let html = r#"<li class="b_algo"><h2><a href="https://example.com">Title</a></h2><div class="b_caption"><p>Desc</p></div></li>"#;
        let result = extract_from_result_html(html).unwrap();
        assert_eq!(result.title, "Title");
        assert_eq!(result.url, "https://example.com");
        assert_eq!(result.description, "Desc");
    }

    #[test]
    fn test_extract_bing_result_no_title() {
        let html = r#"<li class="b_algo"><div class="b_caption"><p>Desc only</p></div></li>"#;
        assert!(extract_from_result_html(html).is_none());
    }

    #[test]
    fn test_extract_bing_result_invalid_url() {
        let html = r#"<li class="b_algo"><h2><a href="/not-http">Bad URL</a></h2></li>"#;
        assert!(extract_from_result_html(html).is_none());
    }

    #[test]
    fn test_parse_results_respects_max() {
        let html = (0..5)
            .map(|i| {
                format!(
                    r#"<li class="b_algo"><h2><a href="https://example{i}.com">Title {i}</a></h2></li>"#
                )
            })
            .collect::<String>();
        let b = BingBackend::new();
        let results = b.parse_results(&html, 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://example0.com");
        assert_eq!(results[1].url, "https://example1.com");
    }

    #[test]
    fn test_parse_results_empty_html() {
        let b = BingBackend::new();
        assert!(b.parse_results("<html></html>", 10).is_empty());
    }

    #[test]
    fn test_bing_backend_name() {
        assert_eq!(BingBackend::new().name(), "bing");
    }

    #[test]
    fn test_parse_empty() {
        let b = BingBackend::new();
        assert!(b.parse_results("<html></html>", 10).is_empty());
    }

    #[test]
    fn test_parse_result() {
        let html = r#"<li class="b_algo"><h2><a href="https://example.com">Title</a></h2><div class="b_caption"><p>Desc</p></div></li>"#;
        let b = BingBackend::new();
        let r = b.parse_results(html, 10);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "Title");
        assert_eq!(r[0].url, "https://example.com");
    }

    #[test]
    fn test_skip_internal() {
        let html = r#"<li class="b_algo"><h2><a href="/internal">X</a></h2></li><li class="b_algo"><h2><a href="https://real.com">Y</a></h2></li>"#;
        let b = BingBackend::new();
        let r = b.parse_results(html, 10);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].url, "https://real.com");
    }
}
