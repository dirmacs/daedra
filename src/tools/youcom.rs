//! You.com web search backend.
//!
//! Uses the You.com Web Search API to return unified web and news results.
//! Requires `YDC_API_KEY` and stays opt-in through `SearchProvider::auto()`.

use super::backend::SearchBackend;
use crate::types::{
    ContentType, DaedraError, DaedraResult, ResultMetadata, SearchArgs, SearchOptions,
    SearchResponse, SearchResult, region_to_gl_hl,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tracing::info;
use url::Url;

const YOUCOM_SEARCH_URL: &str = "https://ydc-index.io/v1/search";

/// You.com Search API backend.
pub struct YouComBackend {
    client: Client,
    api_key: String,
    base_url: String,
}

#[derive(Debug, Deserialize)]
struct YouComResponse {
    results: Option<YouComResults>,
}

#[derive(Debug, Deserialize)]
struct YouComResults {
    #[serde(default)]
    web: Vec<YouComWebResult>,
    #[serde(default)]
    news: Vec<YouComNewsResult>,
}

#[derive(Debug, Deserialize)]
struct YouComWebResult {
    title: String,
    url: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    snippets: Vec<String>,
    #[serde(default)]
    thumbnail_url: Option<String>,
    #[serde(default)]
    favicon_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct YouComNewsResult {
    title: String,
    url: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    thumbnail_url: Option<String>,
    #[serde(default)]
    page_age: Option<String>,
}

impl YouComBackend {
    /// Create a new You.com backend instance.
    pub fn new(api_key: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("HTTP client");
        Self {
            client,
            api_key,
            base_url: YOUCOM_SEARCH_URL.to_string(),
        }
    }

    fn search_body(&self, args: &SearchArgs, opts: &SearchOptions) -> serde_json::Value {
        let mut body = serde_json::json!({
            "query": args.query,
            "count": opts.num_results.clamp(1, 100),
        });

        if let Some(tr) = &opts.time_range {
            let freshness = match tr.as_str() {
                "d" => Some("day"),
                "w" => Some("week"),
                "m" => Some("month"),
                "y" => Some("year"),
                other if other.contains("to") => Some(other),
                _ => None,
            };
            if let Some(freshness) = freshness {
                body["freshness"] = serde_json::Value::String(freshness.to_string());
            }
        }

        let (country, language) = region_to_gl_hl(&opts.region);
        if let Some(country) = country {
            body["country"] = serde_json::Value::String(country);
        }
        if let Some(language) = language {
            body["language"] = serde_json::Value::String(language);
        }

        body["safesearch"] = serde_json::Value::String(
            match opts.safe_search {
                crate::types::SafeSearchLevel::Off => "off",
                crate::types::SafeSearchLevel::Moderate => "moderate",
                crate::types::SafeSearchLevel::Strict => "strict",
            }
            .to_string(),
        );

        body
    }

    fn map_results(&self, data: YouComResponse) -> Vec<SearchResult> {
        let mut results = Vec::new();

        if let Some(results_block) = data.results {
            results.extend(
                results_block
                    .web
                    .into_iter()
                    .map(|item| self.web_result_to_search_result(item)),
            );
            results.extend(
                results_block
                    .news
                    .into_iter()
                    .map(|item| self.news_result_to_search_result(item)),
            );
        }

        results
    }

    fn source_from_url(url: &str) -> String {
        Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.domain().map(str::to_string))
            .unwrap_or_else(|| "you.com".to_string())
    }

    fn web_result_to_search_result(&self, item: YouComWebResult) -> SearchResult {
        let description = if item.snippets.is_empty() {
            item.description
        } else {
            item.snippets.join(" ")
        };

        SearchResult {
            title: item.title,
            url: item.url.clone(),
            description,
            metadata: ResultMetadata {
                content_type: ContentType::Other,
                source: Self::source_from_url(&item.url),
                favicon: item.favicon_url.or(item.thumbnail_url),
                published_date: None,
            },
        }
    }

    fn news_result_to_search_result(&self, item: YouComNewsResult) -> SearchResult {
        SearchResult {
            title: item.title,
            url: item.url.clone(),
            description: item.description,
            metadata: ResultMetadata {
                content_type: ContentType::Article,
                source: Self::source_from_url(&item.url),
                favicon: item.thumbnail_url,
                published_date: item.page_age,
            },
        }
    }
}

impl Default for YouComBackend {
    fn default() -> Self {
        Self::new(String::new())
    }
}

#[async_trait]
impl SearchBackend for YouComBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();
        let body = self.search_body(args, &opts);

        let resp = self
            .client
            .post(&self.base_url)
            .header("X-API-Key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        if !resp.status().is_success() {
            return Err(DaedraError::SearchError(format!(
                "You.com API returned {}",
                resp.status()
            )));
        }

        let data: YouComResponse = resp.json().await.map_err(DaedraError::HttpError)?;
        let results = self.map_results(data);

        info!(
            backend = "youcom",
            results = results.len(),
            "You.com search complete"
        );
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }

    fn name(&self) -> &str {
        "youcom"
    }

    fn requires_api_key(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };

    fn backend_for(server: &MockServer) -> YouComBackend {
        YouComBackend {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("HTTP client"),
            api_key: "test-key".to_string(),
            base_url: format!("{}/v1/search", server.uri()),
        }
    }

    #[tokio::test]
    async fn test_youcom_search_maps_web_and_news_results() {
        let server = MockServer::start().await;
        let backend = backend_for(&server);

        Mock::given(method("POST"))
            .and(path("/v1/search"))
            .and(header("X-API-Key", "test-key"))
            .and(body_json(serde_json::json!({
                "query": "rust async runtime",
                "count": 3,
                "freshness": "week",
                "country": "us",
                "language": "en",
                "safesearch": "strict"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": {
                    "web": [
                        {
                            "title": "Tokio tutorial",
                            "url": "https://tokio.rs/",
                            "description": "Tokio is an async runtime for Rust.",
                            "snippets": ["Tokio is an async runtime for Rust."],
                            "favicon_url": "https://tokio.rs/favicon.ico",
                            "thumbnail_url": "https://tokio.rs/thumb.png"
                        }
                    ],
                    "news": [
                        {
                            "title": "Rust async update",
                            "url": "https://example.com/news/rust-async",
                            "description": "A recent update about async Rust.",
                            "thumbnail_url": "https://example.com/thumb.jpg",
                            "page_age": "2026-08-28T12:00:00"
                        }
                    ]
                }
            })))
            .mount(&server)
            .await;

        let args = SearchArgs {
            query: "rust async runtime".to_string(),
            options: Some(SearchOptions {
                region: "us-en".to_string(),
                safe_search: crate::types::SafeSearchLevel::Strict,
                num_results: 3,
                time_range: Some("w".to_string()),
                backends: None,
                exclude_backends: None,
            }),
        };

        let response = backend.search(&args).await.expect("search response");
        assert_eq!(response.data.len(), 2);
        assert_eq!(response.data[0].title, "Tokio tutorial");
        assert_eq!(
            response.data[0].metadata.favicon.as_deref(),
            Some("https://tokio.rs/favicon.ico")
        );
        assert_eq!(response.data[0].metadata.source, "tokio.rs");
        assert_eq!(response.data[1].metadata.content_type, ContentType::Article);
        assert_eq!(
            response.data[1].metadata.published_date.as_deref(),
            Some("2026-08-28T12:00:00")
        );
        assert_eq!(response.metadata.search_context.safe_search, "STRICT");
    }

    #[tokio::test]
    async fn test_youcom_search_errors_on_non_success_status() {
        let server = MockServer::start().await;
        let backend = backend_for(&server);

        Mock::given(method("POST"))
            .and(path("/v1/search"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let args = SearchArgs {
            query: "rust".to_string(),
            options: Some(SearchOptions::default()),
        };

        let err = backend.search(&args).await.expect_err("expected error");
        assert!(err.to_string().contains("You.com API returned 403"));
    }
}
