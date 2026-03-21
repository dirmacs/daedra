//! DuckDuckGo Instant Answers API — free, no key, works from any IP.
//!
//! Unlike DDG HTML scraping (blocked from datacenter IPs), the Instant
//! Answers API returns structured knowledge: abstracts, related topics,
//! and Wikipedia summaries. Not a full web search but great for factual queries.

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

const DDG_API: &str = "https://api.duckduckgo.com/";

pub struct DdgInstantBackend {
    client: Client,
}

#[derive(Deserialize)]
struct DdgResponse {
    #[serde(rename = "AbstractText", default)]
    abstract_text: String,
    #[serde(rename = "AbstractURL", default)]
    abstract_url: String,
    #[serde(rename = "Heading", default)]
    heading: String,
    #[serde(rename = "RelatedTopics", default)]
    related_topics: Vec<serde_json::Value>,
}

impl DdgInstantBackend {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("daedra/1.0")
            .timeout(Duration::from_secs(10))
            .build()
            .expect("HTTP client");
        Self { client }
    }
}

#[async_trait]
impl SearchBackend for DdgInstantBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();

        let resp = self.client
            .get(DDG_API)
            .query(&[
                ("q", args.query.as_str()),
                ("format", "json"),
                ("no_html", "1"),
                ("skip_disambig", "1"),
            ])
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        let data: DdgResponse = resp.json().await.map_err(DaedraError::HttpError)?;

        let mut results = Vec::new();

        // Add the main abstract if present
        if !data.abstract_text.is_empty() && !data.abstract_url.is_empty() {
            results.push(SearchResult {
                title: data.heading.clone(),
                url: data.abstract_url,
                description: data.abstract_text,
                metadata: ResultMetadata {
                    content_type: ContentType::Documentation,
                    source: "ddg-instant".to_string(),
                    favicon: None,
                    published_date: None,
                },
            });
        }

        // Add related topics
        for topic in &data.related_topics {
            if results.len() >= opts.num_results { break; }
            if let (Some(text), Some(url)) = (
                topic.get("Text").and_then(|v| v.as_str()),
                topic.get("FirstURL").and_then(|v| v.as_str()),
            ) {
                // Skip DDG internal category links
                if url.starts_with("https://duckduckgo.com/c/") { continue; }
                results.push(SearchResult {
                    title: text.chars().take(80).collect(),
                    url: url.to_string(),
                    description: text.to_string(),
                    metadata: ResultMetadata {
                        content_type: ContentType::Documentation,
                        source: "ddg-instant".to_string(),
                        favicon: None,
                        published_date: None,
                    },
                });
            }
        }

        info!(backend = "ddg-instant", results = results.len(), "DDG Instant Answers complete");
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }

    fn name(&self) -> &str { "ddg-instant" }
}
