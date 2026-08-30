//! Mwmbl search backend — no API key needed.
//!
//! Uses the public JSON API at `api.mwmbl.org`. Mwmbl is a non-profit,
//! community-crawled general web index. It answers from any IP and needs
//! no key. Coverage is smaller than a commercial engine. It is the unkeyed
//! general-web JSON backend in this chain.

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

const MWMBL_API: &str = "https://api.mwmbl.org/search/";
const USER_AGENT: &str = concat!(
    "daedra/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/dirmacs/daedra)"
);

#[derive(Debug, Deserialize)]
struct MwmblHit {
    url: String,
    #[serde(default)]
    title: Vec<MwmblFragment>,
    #[serde(default)]
    extract: Vec<MwmblFragment>,
}

#[derive(Debug, Deserialize)]
struct MwmblFragment {
    #[serde(default)]
    value: String,
}

fn join_fragments(parts: &[MwmblFragment]) -> String {
    parts
        .iter()
        .map(|p| p.value.as_str())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Mwmbl public-API search backend.
pub struct MwmblBackend {
    client: Client,
}

impl MwmblBackend {
    /// Create a new Mwmbl search backend instance.
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(20))
            .build()
            .expect("HTTP client");
        Self { client }
    }
}

impl Default for MwmblBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for MwmblBackend {
    fn name(&self) -> &str {
        "mwmbl"
    }

    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();

        let resp = self
            .client
            .get(MWMBL_API)
            .query(&[("s", args.query.as_str())])
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        let status = resp.status();
        if !status.is_success() {
            warn!(status = %status, "Mwmbl returned non-200");
            return Err(DaedraError::SearchError(format!("Mwmbl status {status}")));
        }

        let hits: Vec<MwmblHit> = resp
            .json()
            .await
            .map_err(|e| DaedraError::SearchError(format!("Mwmbl JSON parse: {e}")))?;

        let results: Vec<SearchResult> = hits
            .into_iter()
            .filter(|h| !h.url.trim().is_empty())
            .take(opts.num_results)
            .map(|h| SearchResult {
                title: join_fragments(&h.title),
                url: h.url,
                description: join_fragments(&h.extract),
                metadata: ResultMetadata {
                    content_type: ContentType::Other,
                    source: "mwmbl".to_string(),
                    favicon: None,
                    published_date: None,
                },
            })
            .collect();

        info!(
            query = %args.query,
            count = results.len(),
            "Mwmbl search completed"
        );

        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mwmbl_json() {
        let body = r#"[
            {
                "url": "https://astronomy.stackexchange.com/users/19320/baalateja-kataru",
                "title": [
                    {"value": "User ", "is_bold": false},
                    {"value": "Baalateja", "is_bold": true},
                    {"value": " Kataru - Astronomy Stack Exchange", "is_bold": false}
                ],
                "extract": [],
                "source": "mwmbl"
            },
            {
                "url": "https://gitlab.com/BK-Modding",
                "title": [{"value": "Baalateja Kataru · GitLab"}],
                "extract": [{"value": "GitLab  profile\npage"}],
                "source": "mwmbl"
            }
        ]"#;
        let parsed: Vec<MwmblHit> = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            join_fragments(&parsed[0].title),
            "User Baalateja Kataru - Astronomy Stack Exchange"
        );
        assert!(join_fragments(&parsed[0].extract).is_empty());
        assert_eq!(join_fragments(&parsed[1].extract), "GitLab profile page");
    }

    #[test]
    fn test_parse_empty_results() {
        let parsed: Vec<MwmblHit> = serde_json::from_str("[]").unwrap();
        assert!(parsed.is_empty());
    }

    #[tokio::test]
    #[ignore = "live network"]
    async fn live_search() {
        let backend = MwmblBackend::new();
        let args = SearchArgs {
            query: "rust async runtime".to_string(),
            options: None,
        };
        let resp = backend.search(&args).await.unwrap();
        assert!(!resp.data.is_empty());
    }
}
