//! StackExchange search backend — free, no API key, works from any IP.
//!
//! Searches StackOverflow and other StackExchange sites via their public API.
//! Great for technical/programming queries.

use super::backend::SearchBackend;
use crate::types::{
    ContentType, DaedraResult, DaedraError, ResultMetadata, SearchArgs, SearchResponse,
    SearchResult,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tracing::info;

const STACKEXCHANGE_API: &str = "https://api.stackexchange.com/2.3/search/advanced";

pub struct StackExchangeBackend {
    client: Client,
}

#[derive(Deserialize)]
struct SeResponse {
    items: Option<Vec<SeItem>>,
}

#[derive(Deserialize)]
struct SeItem {
    title: String,
    link: String,
    #[serde(default)]
    score: i64,
    #[serde(default)]
    answer_count: u64,
}

impl StackExchangeBackend {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("daedra/1.0")
            .timeout(Duration::from_secs(15))
            .gzip(true)
            .brotli(true)
            .build()
            .expect("HTTP client");
        Self { client }
    }
}

#[async_trait]
impl SearchBackend for StackExchangeBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();

        let resp = self.client
            .get(STACKEXCHANGE_API)
            .query(&[
                ("q", args.query.as_str()),
                ("order", "desc"),
                ("sort", "relevance"),
                ("site", "stackoverflow"),
                ("pagesize", &opts.num_results.min(25).to_string()),
                ("filter", "default"),
            ])
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        let data: SeResponse = resp.json().await.map_err(DaedraError::HttpError)?;

        let results: Vec<SearchResult> = data.items.unwrap_or_default()
            .into_iter()
            .map(|item| {
                let desc = format!("Score: {} | Answers: {}", item.score, item.answer_count);
                SearchResult {
                    title: html_escape::decode_html_entities(&item.title).to_string(),
                    url: item.link,
                    description: desc,
                    metadata: ResultMetadata {
                        content_type: ContentType::Forum,
                        source: "stackoverflow".to_string(),
                        favicon: None,
                        published_date: None,
                    },
                }
            })
            .take(opts.num_results)
            .collect();

        info!(backend = "stackoverflow", results = results.len(), "StackExchange search complete");
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }

    fn name(&self) -> &str { "stackoverflow" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stackoverflow_search_live() {
        let backend = StackExchangeBackend::new();
        let args = SearchArgs {
            query: "rust borrow checker".to_string(),
            options: Some(crate::types::SearchOptions {
                num_results: 3,
                ..Default::default()
            }),
        };
        let response = backend.search(&args).await.unwrap();
        assert!(!response.data.is_empty(), "StackOverflow should return results");
        assert!(response.data[0].url.contains("stackoverflow.com"));
    }
}
