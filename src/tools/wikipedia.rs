//! Wikipedia/Wikidata search backend — free, no API key, works from any IP.
//!
//! Uses the MediaWiki opensearch API for instant results.
//! Limited to Wikipedia content — not a general web search.

use super::backend::SearchBackend;
use crate::types::{
    ContentType, DaedraResult, DaedraError, ResultMetadata, SearchArgs, SearchResponse,
    SearchResult,
};
use async_trait::async_trait;
use reqwest::Client;
use std::time::Duration;
use tracing::info;

const WIKIPEDIA_API: &str = "https://en.wikipedia.org/w/api.php";

pub struct WikipediaBackend {
    client: Client,
}

impl WikipediaBackend {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("daedra/1.0 (search MCP server)")
            .timeout(Duration::from_secs(15))
            .build()
            .expect("HTTP client");
        Self { client }
    }
}

#[async_trait]
impl SearchBackend for WikipediaBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();

        let resp = self.client
            .get(WIKIPEDIA_API)
            .query(&[
                ("action", "opensearch"),
                ("search", &args.query),
                ("limit", &opts.num_results.min(20).to_string()),
                ("format", "json"),
            ])
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        // OpenSearch returns: [query, [titles], [descriptions], [urls]]
        let data: serde_json::Value = resp.json().await.map_err(DaedraError::HttpError)?;

        let titles = data.get(1).and_then(|v| v.as_array());
        let descriptions = data.get(2).and_then(|v| v.as_array());
        let urls = data.get(3).and_then(|v| v.as_array());

        let mut results = Vec::new();

        if let (Some(titles), Some(descs), Some(urls)) = (titles, descriptions, urls) {
            for i in 0..titles.len().min(opts.num_results) {
                let title = titles.get(i).and_then(|v| v.as_str()).unwrap_or_default();
                let desc = descs.get(i).and_then(|v| v.as_str()).unwrap_or_default();
                let url = urls.get(i).and_then(|v| v.as_str()).unwrap_or_default();

                if !url.is_empty() {
                    results.push(SearchResult {
                        title: title.to_string(),
                        url: url.to_string(),
                        description: desc.to_string(),
                        metadata: ResultMetadata {
                            content_type: ContentType::Documentation,
                            source: "wikipedia".to_string(),
                            favicon: None,
                            published_date: None,
                        },
                    });
                }
            }
        }

        info!(backend = "wikipedia", results = results.len(), "Wikipedia search complete");
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }

    fn name(&self) -> &str { "wikipedia" }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
