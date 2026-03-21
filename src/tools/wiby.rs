//! Wiby search backend — free indie web search, no API key.
//!
//! Wiby.me indexes small, independent websites — blogs, personal sites,
//! hobbyist pages. Complements mainstream engines with human-curated indie web.

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

const WIBY_API: &str = "https://wiby.me/json/";

pub struct WibyBackend {
    client: Client,
}

#[derive(Deserialize)]
struct WibyResult {
    #[serde(rename = "Title")]
    title: String,
    #[serde(rename = "URL")]
    url: String,
    #[serde(rename = "Snippet", default)]
    snippet: String,
}

impl WibyBackend {
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
impl SearchBackend for WibyBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();

        let resp = self.client
            .get(WIBY_API)
            .query(&[("q", args.query.as_str())])
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        let data: Vec<WibyResult> = resp.json().await.map_err(DaedraError::HttpError)?;

        let results: Vec<SearchResult> = data.into_iter()
            .take(opts.num_results)
            .map(|r| SearchResult {
                title: r.title,
                url: r.url,
                description: r.snippet,
                metadata: ResultMetadata {
                    content_type: ContentType::Article,
                    source: "wiby".to_string(),
                    favicon: None,
                    published_date: None,
                },
            })
            .collect();

        info!(backend = "wiby", results = results.len(), "Wiby search complete");
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }

    fn name(&self) -> &str { "wiby" }
}
