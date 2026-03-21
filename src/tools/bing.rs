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
use reqwest::Client;
use scraper::{Html, Selector};
use std::time::Duration;
use tracing::{info, warn};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const BING_URL: &str = "https://www.bing.com/search";

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

    fn parse_results(&self, html: &str, max: usize) -> Vec<SearchResult> {
        let doc = Html::parse_document(html);
        let result_sel = Selector::parse("li.b_algo").unwrap();
        let title_sel = Selector::parse("h2 a").unwrap();
        let snippet_sel = Selector::parse(".b_caption p, .b_lineclamp2").unwrap();

        let mut results = Vec::new();

        for el in doc.select(&result_sel).take(max) {
            let title_el = match el.select(&title_sel).next() {
                Some(e) => e,
                None => continue,
            };

            let title: String = title_el.text().collect();
            let url = title_el.value().attr("href").unwrap_or_default().to_string();

            if url.is_empty() || title.is_empty() { continue; }
            if url.starts_with("/") || url.contains("bing.com/ck/") { continue; }

            let description: String = el.select(&snippet_sel).next()
                .map(|e| e.text().collect())
                .unwrap_or_default();

            results.push(SearchResult {
                title: title.trim().to_string(),
                url,
                description: description.trim().to_string(),
                metadata: ResultMetadata {
                    content_type: ContentType::Other,
                    source: "bing".to_string(),
                    favicon: None,
                    published_date: None,
                },
            });
        }

        results
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
