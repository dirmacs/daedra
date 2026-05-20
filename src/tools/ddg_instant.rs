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

fn abstract_to_result(data: &DdgResponse) -> Option<SearchResult> {
    if data.abstract_text.is_empty() {
        return None;
    }
    Some(SearchResult {
        title: data.heading.clone(),
        url: data.abstract_url.clone(),
        description: data.abstract_text.clone(),
        metadata: ResultMetadata {
            content_type: ContentType::Documentation,
            source: "ddg-instant".to_string(),
            favicon: None,
            published_date: None,
        },
    })
}

fn topic_to_result(topic: &serde_json::Value) -> Option<SearchResult> {
    let text = topic.get("Text")?.as_str()?;
    let url = topic.get("FirstURL")?.as_str()?;
    if url.starts_with("https://duckduckgo.com/c/") {
        return None;
    }
    Some(SearchResult {
        title: text.chars().take(80).collect(),
        url: url.to_string(),
        description: text.to_string(),
        metadata: ResultMetadata {
            content_type: ContentType::Documentation,
            source: "ddg-instant".to_string(),
            favicon: None,
            published_date: None,
        },
    })
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

        if let Some(result) = abstract_to_result(&data) {
            results.push(result);
        }

        for topic in &data.related_topics {
            if results.len() >= opts.num_results {
                break;
            }
            if let Some(result) = topic_to_result(topic) {
                results.push(result);
            }
        }

        info!(backend = "ddg-instant", results = results.len(), "DDG Instant Answers complete");
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }

    fn name(&self) -> &str { "ddg-instant" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ddg_instant_name() {
        assert_eq!(DdgInstantBackend::new().name(), "ddg-instant");
    }

    fn sample_ddg_response() -> DdgResponse {
        serde_json::from_str(
            r#"{
            "AbstractText": "Rust is a systems programming language.",
            "AbstractURL": "https://example.com/rust",
            "Heading": "Rust (programming language)",
            "RelatedTopics": [
                {"Text": "Related topic", "FirstURL": "https://example.com/related"}
            ]
        }"#,
        )
        .unwrap()
    }

    #[test]
    fn test_abstract_to_result_present() {
        let data = sample_ddg_response();
        let result = abstract_to_result(&data).unwrap();
        assert_eq!(result.title, "Rust (programming language)");
        assert_eq!(result.url, "https://example.com/rust");
        assert_eq!(result.description, "Rust is a systems programming language.");
    }

    #[test]
    fn test_abstract_to_result_empty() {
        let data = DdgResponse {
            abstract_text: String::new(),
            abstract_url: "https://example.com".to_string(),
            heading: "Heading".to_string(),
            related_topics: vec![],
        };
        assert!(abstract_to_result(&data).is_none());
    }

    #[test]
    fn test_topic_to_result_valid() {
        let topic = serde_json::json!({
            "Text": "Related topic",
            "FirstURL": "https://example.com/related"
        });
        let result = topic_to_result(&topic).unwrap();
        assert_eq!(result.title, "Related topic");
        assert_eq!(result.url, "https://example.com/related");
        assert_eq!(result.description, "Related topic");
    }

    #[test]
    fn test_topic_to_result_ddg_category() {
        let topic = serde_json::json!({
            "Text": "Category",
            "FirstURL": "https://duckduckgo.com/c/Programming"
        });
        assert!(topic_to_result(&topic).is_none());
    }

    #[test]
    fn test_topic_to_result_missing_fields() {
        assert!(topic_to_result(&serde_json::json!({"Text": "only text"})).is_none());
        assert!(topic_to_result(&serde_json::json!({"FirstURL": "https://example.com"})).is_none());
        assert!(topic_to_result(&serde_json::json!({})).is_none());
    }

    #[test]
    fn test_ddg_response_deserialize() {
        let data = sample_ddg_response();
        assert_eq!(data.abstract_text, "Rust is a systems programming language.");
        assert_eq!(data.abstract_url, "https://example.com/rust");
        assert_eq!(data.heading, "Rust (programming language)");
        assert_eq!(data.related_topics.len(), 1);
        assert_eq!(
            data.related_topics[0].get("Text").and_then(|v| v.as_str()),
            Some("Related topic")
        );
    }
}
