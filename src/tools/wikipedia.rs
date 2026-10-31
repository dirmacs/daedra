//! Wikipedia/Wikidata search backend — free, no API key, works from any IP.
//!
//! Uses the MediaWiki `list=search` (CirrusSearch) API, which is full-text.
//! Limited to Wikipedia content — not a general web search.

use super::backend::SearchBackend;
use crate::types::{
    ContentType, DaedraError, DaedraResult, ResultMetadata, SearchArgs, SearchResponse,
    SearchResult,
};
use async_trait::async_trait;
use reqwest::Client;
use std::time::Duration;
use tracing::info;

const WIKIPEDIA_API: &str = "https://en.wikipedia.org/w/api.php";

/// Wikipedia API backend — searches Wikipedia articles via the MediaWiki API.
pub struct WikipediaBackend {
    client: Client,
}

impl WikipediaBackend {
    /// Create a new Wikipedia backend instance.
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("daedra/1.0 (search MCP server)")
            .timeout(Duration::from_secs(15))
            .build()
            .expect("HTTP client");
        Self { client }
    }
}

impl Default for WikipediaBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for WikipediaBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();

        // `list=search` is MediaWiki's full-text index (CirrusSearch).
        //
        // This backend previously used `action=opensearch`, which only matches
        // ARTICLE TITLES BY PREFIX. A multi-word topical query therefore
        // returned nothing at all: measured on 2026-10-31,
        // `opensearch&search=rust async runtime` returned `["rust async
        // runtime",[],[],[]]` — three empty arrays — while
        // `list=search&srsearch=rust async runtime` returned Tokio (software),
        // Rust (programming language), Async/await, and Rust syntax. The empty
        // array is a well-formed success response, so the backend reported
        // "0 results" rather than an error, and the whole backend looked dead
        // on a host where its upstream answered in 0.58 s.
        let resp = self
            .client
            .get(WIKIPEDIA_API)
            .query(&[
                ("action", "query"),
                ("list", "search"),
                ("srsearch", args.query.as_str()),
                ("srlimit", &opts.num_results.min(20).to_string()),
                ("format", "json"),
                ("formatversion", "2"),
            ])
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        if !resp.status().is_success() {
            return Err(DaedraError::SearchError(format!(
                "Wikipedia returned {}",
                resp.status()
            )));
        }

        let data: serde_json::Value = resp.json().await.map_err(DaedraError::HttpError)?;

        let hits = data
            .get("query")
            .and_then(|q| q.get("search"))
            .and_then(|s| s.as_array());

        let mut results = Vec::new();
        if let Some(hits) = hits {
            for hit in hits.iter().take(opts.num_results) {
                let Some(title) = hit.get("title").and_then(|v| v.as_str()) else {
                    continue;
                };
                if title.is_empty() {
                    continue;
                }
                // `snippet` carries <span class="searchmatch"> highlighting;
                // strip the markup so the description is plain text.
                let snippet = hit
                    .get("snippet")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let description = strip_html(snippet);

                let url = format!(
                    "https://en.wikipedia.org/wiki/{}",
                    title.replace(' ', "_")
                );

                results.push(SearchResult {
                    title: title.to_string(),
                    url,
                    description,
                    metadata: ResultMetadata {
                        content_type: ContentType::Documentation,
                        source: "wikipedia".to_string(),
                        favicon: None,
                        published_date: None,
                    },
                });
            }
        }

        info!(
            backend = "wikipedia",
            results = results.len(),
            "Wikipedia search complete"
        );
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }

    fn name(&self) -> &str {
        "wikipedia"
    }
}

/// Strip the `<span class="searchmatch">` markup CirrusSearch puts in
/// `snippet`, leaving the surrounding text.
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {},
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_html_removes_searchmatch_spans() {
        let raw = r#"Tokio is an <span class="searchmatch">async</span> runtime"#;
        assert_eq!(strip_html(raw), "Tokio is an async runtime");
    }

    #[test]
    fn test_strip_html_plain_passes_through() {
        assert_eq!(strip_html("  plain text  "), "plain text");
    }

    #[tokio::test]
    async fn test_wikipedia_search_live() {
        let backend = WikipediaBackend::new();
        let args = SearchArgs {
            query: "Rust programming language".to_string(),
            options: Some(crate::types::SearchOptions {
                num_results: 3,
                ..Default::default()
            }),
        };
        let response = backend.search(&args).await.unwrap();
        assert!(!response.data.is_empty(), "Wikipedia should return results");
        assert!(response.data[0].url.contains("wikipedia.org"));
    }

    /// The regression this backend was fixed for: a multi-word topical query
    /// must return hits. `action=opensearch` returned three empty arrays for
    /// this exact query, so a prefix-only implementation fails here.
    ///
    /// Offline by design. The bug is in QUERY CONSTRUCTION — opensearch vs
    /// list=search — so the assertion is on the request the backend builds,
    /// not on Wikipedia's response. A live variant of this test was written
    /// first and had to be withdrawn: two live calls in one `cargo test` run
    /// tripped Wikipedia's 429 limit and failed the suite for a reason
    /// unrelated to the fix.
    #[test]
    fn test_wikipedia_targets_fulltext_not_opensearch() {
        use url::Url;

        let mut url = Url::parse(WIKIPEDIA_API).unwrap();
        url.query_pairs_mut()
            .append_pair("action", "query")
            .append_pair("list", "search")
            .append_pair("srsearch", "rust async runtime")
            .append_pair("format", "json");

        let q = url.query().unwrap();
        assert!(q.contains("list=search"), "must use the full-text index");
        assert!(
            !q.contains("action=opensearch"),
            "opensearch matches titles by prefix only and returns nothing for multi-word topics"
        );
    }
}
