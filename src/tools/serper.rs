//! Serper.dev search backend — Google results via API.
//!
//! Requires SERPER_API_KEY environment variable.
//! Free tier: 2500 queries. Pay-as-you-go after.

use super::backend::SearchBackend;
use crate::types::{
    ContentType, DaedraError, DaedraResult, ResultMetadata, SearchArgs, SearchResponse,
    SearchResult,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tracing::info;

const SERPER_URL: &str = "https://google.serper.dev/search";

pub struct SerperBackend {
    client: Client,
    api_key: String,
}

#[derive(Deserialize)]
struct SerperResponse {
    organic: Option<Vec<SerperResult>>,
}

#[derive(Deserialize)]
struct SerperResult {
    title: String,
    link: String,
    snippet: Option<String>,
}

impl SerperBackend {
    pub fn new(api_key: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("HTTP client");
        Self { client, api_key }
    }
}

#[async_trait]
impl SearchBackend for SerperBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();
        let body = serde_json::json!({
            "q": args.query,
            "num": opts.num_results
        });

        let resp = self.client
            .post(SERPER_URL)
            .header("X-API-KEY", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        let data: SerperResponse = resp.json().await.map_err(DaedraError::HttpError)?;

        let results: Vec<SearchResult> = data.organic.unwrap_or_default()
            .into_iter()
            .map(|r| SearchResult {
                title: r.title,
                url: r.link.clone(),
                description: r.snippet.unwrap_or_default(),
                metadata: ResultMetadata {
                    content_type: ContentType::Other,
                    source: "serper".to_string(),
                    favicon: None,
                    published_date: None,
                },
            })
            .take(opts.num_results)
            .collect();

        info!(backend = "serper", results = results.len(), "Serper search complete");
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }

    fn name(&self) -> &str { "serper" }
    fn requires_api_key(&self) -> bool { true }
}
