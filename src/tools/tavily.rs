//! Tavily AI-optimized search backend.
//!
//! Requires TAVILY_API_KEY environment variable.
//! Free tier: 1000 queries/month.

use super::backend::SearchBackend;
use crate::types::{
    DaedraError, DaedraResult, SearchArgs, SearchResponse, SearchResult, ResultMetadata,
    ContentType,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tracing::info;

const TAVILY_URL: &str = "https://api.tavily.com/search";

pub struct TavilyBackend {
    client: Client,
    api_key: String,
}

#[derive(Deserialize)]
struct TavilyResponse {
    results: Option<Vec<TavilyResult>>,
}

#[derive(Deserialize)]
struct TavilyResult {
    title: String,
    url: String,
    content: Option<String>,
}

impl TavilyBackend {
    pub fn new(api_key: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("HTTP client");
        Self { client, api_key }
    }
}

#[async_trait]
impl SearchBackend for TavilyBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();

        let body = serde_json::json!({
            "api_key": self.api_key,
            "query": args.query,
            "max_results": opts.num_results,
            "search_depth": "basic",
        });

        let resp = self.client
            .post(TAVILY_URL)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        let data: TavilyResponse = resp.json().await.map_err(DaedraError::HttpError)?;

        let results: Vec<SearchResult> = data.results.unwrap_or_default()
            .into_iter()
            .map(|r| SearchResult {
                title: r.title,
                url: r.url.clone(),
                description: r.content.unwrap_or_default(),
                metadata: ResultMetadata {
                    content_type: ContentType::Other,
                    source: "tavily".to_string(),
                    favicon: None,
                    published_date: None,
                },
            })
            .take(opts.num_results)
            .collect();

        info!(backend = "tavily", results = results.len(), "Tavily search complete");
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }

    fn name(&self) -> &str { "tavily" }
    fn requires_api_key(&self) -> bool { true }
}
