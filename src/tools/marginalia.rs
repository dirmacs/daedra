//! Marginalia search backend — no API key needed.
//!
//! Uses Marginalia's public JSON API (`api.marginalia.nu`, license key
//! `public`). Marginalia is an independent, non-commercial-focused index
//! that answers from any IP, which makes it the most reliable unkeyed
//! general backend for this chain. Strong for documentation, blogs, and
//! technical content; weak for commercial queries.

use super::backend::SearchBackend;
use crate::types::{
    ContentType, DaedraError, DaedraResult, ResultMetadata, SearchArgs, SearchResponse,
    SearchResult,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tracing::{info, warn};

const MARGINALIA_API_PUBLIC: &str = "https://api.marginalia.nu/public/search";
const MARGINALIA_API_KEYED: &str = "https://api.marginalia.nu";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Debug, Deserialize)]
struct MarginaliaResponse {
    #[serde(default)]
    results: Vec<MarginaliaResult>,
}

#[derive(Debug, Deserialize)]
struct MarginaliaResult {
    url: String,
    title: String,
    #[serde(default)]
    description: String,
}

/// Marginalia public-API search backend.
pub struct MarginaliaBackend {
    client: Client,
    base_url: String,
}

impl MarginaliaBackend {
    /// Create a new Marginalia backend instance.
    pub fn new() -> Self {
        // MARGINALIA_API_KEY selects the keyed endpoint with higher limits;
        // without it the shared `public` license key serves the request.
        // The public key has no SLA — it is a shared courtesy endpoint.
        match std::env::var("MARGINALIA_API_KEY") {
            Ok(key) if !key.trim().is_empty() => {
                info!("Marginalia backend using MARGINALIA_API_KEY");
                Self::with_base_url(format!("{MARGINALIA_API_KEYED}/{key}/search"))
            },
            _ => Self::with_base_url(MARGINALIA_API_PUBLIC.to_string()),
        }
    }

    /// Point at a custom endpoint (tests).
    pub fn with_base_url(base_url: String) -> Self {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");
        Self { client, base_url }
    }
}

impl Default for MarginaliaBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for MarginaliaBackend {
    fn name(&self) -> &str {
        "marginalia"
    }

    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();

        // The public API takes the query in the path. Keep it short: the
        // endpoint rejects very long paths.
        let query: String = args
            .query
            .split_whitespace()
            .take(12)
            .collect::<Vec<_>>()
            .join(" ");
        let url = format!("{}/{}", self.base_url, urlencoding::encode(&query));

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        let status = resp.status();
        if !status.is_success() {
            warn!(status = %status, "Marginalia returned non-200");
            // 429/5xx are worth another try later; the retry loop in the
            // fetch path does not apply here, so surface them as errors the
            // chain understands.
            return Err(DaedraError::SearchError(format!(
                "Marginalia status {status}"
            )));
        }

        let body: MarginaliaResponse = resp
            .json()
            .await
            .map_err(|e| DaedraError::SearchError(format!("Marginalia JSON parse: {e}")))?;

        let results: Vec<SearchResult> = body
            .results
            .into_iter()
            .take(opts.num_results)
            .map(|r| SearchResult {
                title: r.title.trim().to_string(),
                url: r.url,
                description: r.description.trim().to_string(),
                metadata: ResultMetadata {
                    content_type: ContentType::Other,
                    source: "marginalia".to_string(),
                    favicon: None,
                    published_date: None,
                },
            })
            .collect();

        info!(
            query = %args.query,
            count = results.len(),
            "Marginalia search completed"
        );

        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_marginalia_json() {
        let body = r#"{
            "license": "CC-BY-NC-SA 4.0",
            "page": 1,
            "pages": 11,
            "query": "tokio runtime",
            "results": [
                {"url": "https://tokio.rs/blog/2018-03-tokio-runtime",
                 "title": "Announcing the Tokio runtime",
                 "description": "The first iteration of the Tokio Runtime."},
                {"url": "https://example.com/empty-desc", "title": "No description"}
            ]
        }"#;
        let parsed: MarginaliaResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.results.len(), 2);
        assert_eq!(
            parsed.results[0].url,
            "https://tokio.rs/blog/2018-03-tokio-runtime"
        );
        assert!(parsed.results[1].description.is_empty());
    }

    #[test]
    fn test_parse_empty_results() {
        let parsed: MarginaliaResponse =
            serde_json::from_str(r#"{"license":"x","results":[]}"#).unwrap();
        assert!(parsed.results.is_empty());
    }

    #[tokio::test]
    #[ignore = "live network"]
    async fn live_search() {
        let backend = MarginaliaBackend::new();
        let args = SearchArgs {
            query: "tokio runtime".to_string(),
            options: None,
        };
        let resp = backend.search(&args).await.unwrap();
        assert!(!resp.data.is_empty());
    }
}
