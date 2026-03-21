//! GitHub search backend — free, no API key for basic search, works from any IP.
//!
//! Searches GitHub repositories, code, and issues via the public API.
//! Rate limit: 10 requests/minute unauthenticated, 30 with GITHUB_TOKEN.

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

const GITHUB_API: &str = "https://api.github.com/search/repositories";

pub struct GitHubBackend {
    client: Client,
    token: Option<String>,
}

#[derive(Deserialize)]
struct GhResponse {
    items: Option<Vec<GhRepo>>,
}

#[derive(Deserialize)]
struct GhRepo {
    full_name: String,
    html_url: String,
    description: Option<String>,
    stargazers_count: u64,
    language: Option<String>,
}

impl GitHubBackend {
    pub fn new() -> Self {
        let token = std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty());
        let client = Client::builder()
            .user_agent("daedra/1.0")
            .timeout(Duration::from_secs(15))
            .build()
            .expect("HTTP client");
        Self { client, token }
    }
}

#[async_trait]
impl SearchBackend for GitHubBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();

        let mut req = self.client
            .get(GITHUB_API)
            .query(&[
                ("q", args.query.as_str()),
                ("per_page", &opts.num_results.min(30).to_string()),
                ("sort", "stars"),
                ("order", "desc"),
            ]);

        if let Some(ref token) = self.token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let resp = req.send().await.map_err(DaedraError::HttpError)?;

        if !resp.status().is_success() {
            return Err(DaedraError::SearchError(
                format!("GitHub API returned {}", resp.status()),
            ));
        }

        let data: GhResponse = resp.json().await.map_err(DaedraError::HttpError)?;

        let results: Vec<SearchResult> = data.items.unwrap_or_default()
            .into_iter()
            .map(|r| {
                let desc = format!(
                    "{} | {} {}",
                    r.description.unwrap_or_default(),
                    r.stargazers_count,
                    r.language.map(|l| format!("| {}", l)).unwrap_or_default(),
                );
                SearchResult {
                    title: r.full_name,
                    url: r.html_url,
                    description: desc,
                    metadata: ResultMetadata {
                        content_type: ContentType::Documentation,
                        source: "github".to_string(),
                        favicon: None,
                        published_date: None,
                    },
                }
            })
            .take(opts.num_results)
            .collect();

        info!(backend = "github", results = results.len(), "GitHub search complete");
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }

    fn name(&self) -> &str { "github" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_github_search_live() {
        let backend = GitHubBackend::new();
        let args = SearchArgs {
            query: "rust async runtime".to_string(),
            options: Some(crate::types::SearchOptions {
                num_results: 3,
                ..Default::default()
            }),
        };
        let response = backend.search(&args).await.unwrap();
        assert!(!response.data.is_empty(), "GitHub should return results");
        assert!(response.data[0].url.contains("github.com"));
    }
}
