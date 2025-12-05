//! Common types and data structures used throughout Daedra.
//!
//! This module contains all the shared types including:
//! - Search arguments and results
//! - Error types
//! - Configuration structures

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Result type alias for Daedra operations
pub type DaedraResult<T> = Result<T, DaedraError>;

/// Errors that can occur during Daedra operations
#[derive(Error, Debug)]
pub enum DaedraError {
    /// HTTP request failed
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    /// URL parsing failed
    #[error("Invalid URL: {0}")]
    UrlParseError(#[from] url::ParseError),

    /// JSON serialization/deserialization failed
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    /// Search operation failed
    #[error("Search failed: {0}")]
    SearchError(String),

    /// Page fetch failed
    #[error("Failed to fetch page: {0}")]
    FetchError(String),

    /// Invalid arguments provided
    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    /// Server error
    #[error("Server error: {0}")]
    ServerError(String),

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Content extraction failed
    #[error("Content extraction failed: {0}")]
    ExtractionError(String),

    /// Rate limit exceeded
    #[error("Rate limit exceeded, please try again later")]
    RateLimitExceeded,

    /// Bot protection detected
    #[error("Bot protection detected on target page")]
    BotProtectionDetected,

    /// Timeout occurred
    #[error("Operation timed out")]
    Timeout,
}

/// Safe search filtering levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "UPPERCASE")]
pub enum SafeSearchLevel {
    /// No filtering
    Off,
    /// Moderate filtering (default)
    #[default]
    Moderate,
    /// Strict filtering
    Strict,
}

impl SafeSearchLevel {
    /// Convert to DuckDuckGo safe search parameter value
    pub fn to_ddg_value(&self) -> i32 {
        match self {
            SafeSearchLevel::Off => -2,
            SafeSearchLevel::Moderate => -1,
            SafeSearchLevel::Strict => 1,
        }
    }
}

impl std::fmt::Display for SafeSearchLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SafeSearchLevel::Off => write!(f, "OFF"),
            SafeSearchLevel::Moderate => write!(f, "MODERATE"),
            SafeSearchLevel::Strict => write!(f, "STRICT"),
        }
    }
}

impl std::str::FromStr for SafeSearchLevel {
    type Err = DaedraError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "OFF" => Ok(SafeSearchLevel::Off),
            "MODERATE" => Ok(SafeSearchLevel::Moderate),
            "STRICT" => Ok(SafeSearchLevel::Strict),
            _ => Err(DaedraError::InvalidArguments(format!(
                "Invalid safe search level: {}",
                s
            ))),
        }
    }
}

/// Options for search operations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchOptions {
    /// Region for search results (e.g., "us-en", "zh-cn")
    #[serde(default = "default_region")]
    pub region: String,

    /// Safe search filtering level
    #[serde(default)]
    pub safe_search: SafeSearchLevel,

    /// Maximum number of results to return
    #[serde(default = "default_num_results")]
    pub num_results: usize,

    /// Time range filter (e.g., "d" for day, "w" for week, "m" for month)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_range: Option<String>,
}

fn default_region() -> String {
    "wt-wt".to_string() // Worldwide
}

fn default_num_results() -> usize {
    10
}

/// Arguments for the search tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchArgs {
    /// The search query string
    pub query: String,

    /// Optional search configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<SearchOptions>,
}

/// Arguments for the visit_page tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisitPageArgs {
    /// URL of the page to visit
    pub url: String,

    /// Optional CSS selector to target specific content
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,

    /// Whether to include images in the response
    #[serde(default)]
    pub include_images: bool,
}

/// Content type classification for search results
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ContentType {
    /// Documentation pages
    Documentation,
    /// Social media content
    Social,
    /// News articles
    Article,
    /// Forum discussions
    Forum,
    /// Video content
    Video,
    /// E-commerce/shopping
    Shopping,
    /// Other/unknown content
    #[default]
    Other,
}

/// Metadata for a search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultMetadata {
    /// Content type classification
    #[serde(rename = "type")]
    pub content_type: ContentType,

    /// Source domain
    pub source: String,

    /// Favicon URL if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,

    /// Published date if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_date: Option<String>,
}

/// A single search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Title of the result
    pub title: String,

    /// URL of the result
    pub url: String,

    /// Description/snippet
    pub description: String,

    /// Additional metadata
    pub metadata: ResultMetadata,
}

/// Query analysis information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryAnalysis {
    /// Detected language of the query
    pub language: String,

    /// Detected topics in results
    pub topics: Vec<String>,
}

/// Search context information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchContext {
    /// Region used for search
    pub region: String,

    /// Safe search level applied
    pub safe_search: String,

    /// Number of results requested
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_results: Option<usize>,
}

/// Metadata about the search operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMetadata {
    /// Original search query
    pub query: String,

    /// ISO timestamp of when search was conducted
    pub timestamp: String,

    /// Number of results returned
    pub result_count: usize,

    /// Search context information
    pub search_context: SearchContext,

    /// Query analysis results
    pub query_analysis: QueryAnalysis,
}

/// Complete search response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    /// Response type discriminator
    #[serde(rename = "type")]
    pub response_type: String,

    /// Array of search results
    pub data: Vec<SearchResult>,

    /// Search metadata
    pub metadata: SearchMetadata,
}

impl SearchResponse {
    /// Create a new search response
    pub fn new(query: String, results: Vec<SearchResult>, options: &SearchOptions) -> Self {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let result_count = results.len();

        // Analyze query for language detection
        let language = detect_language(&query);
        let topics = detect_topics(&results);

        Self {
            response_type: "search_results".to_string(),
            data: results,
            metadata: SearchMetadata {
                query,
                timestamp,
                result_count,
                search_context: SearchContext {
                    region: options.region.clone(),
                    safe_search: options.safe_search.to_string(),
                    num_results: Some(options.num_results),
                },
                query_analysis: QueryAnalysis { language, topics },
            },
        }
    }
}

/// Result of visiting a page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageContent {
    /// URL of the page
    pub url: String,

    /// Page title
    pub title: String,

    /// Extracted content in Markdown format
    pub content: String,

    /// ISO timestamp of when page was fetched
    pub timestamp: String,

    /// Word count of extracted content
    pub word_count: usize,

    /// Links found on the page
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<PageLink>>,
}

/// A link found on a page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageLink {
    /// Link text
    pub text: String,

    /// Link URL
    pub url: String,
}

/// Detect language of a query using simple heuristics
fn detect_language(query: &str) -> String {
    // Check for Chinese characters
    if query
        .chars()
        .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
    {
        return "zh".to_string();
    }

    // Check for Japanese characters (Hiragana/Katakana)
    if query
        .chars()
        .any(|c| ('\u{3040}'..='\u{30ff}').contains(&c))
    {
        return "ja".to_string();
    }

    // Check for Korean characters
    if query
        .chars()
        .any(|c| ('\u{ac00}'..='\u{d7af}').contains(&c))
    {
        return "ko".to_string();
    }

    // Check for Cyrillic
    if query
        .chars()
        .any(|c| ('\u{0400}'..='\u{04ff}').contains(&c))
    {
        return "ru".to_string();
    }

    // Check for Arabic
    if query
        .chars()
        .any(|c| ('\u{0600}'..='\u{06ff}').contains(&c))
    {
        return "ar".to_string();
    }

    // Default to English
    "en".to_string()
}

/// Detect topics from search results
fn detect_topics(results: &[SearchResult]) -> Vec<String> {
    use std::collections::HashSet;

    let mut topics = HashSet::new();

    for result in results {
        let lower_title = result.title.to_lowercase();
        let lower_url = result.url.to_lowercase();

        // Technology indicators
        if lower_url.contains("github.com")
            || lower_url.contains("stackoverflow.com")
            || lower_url.contains("gitlab.com")
            || lower_title.contains("programming")
            || lower_title.contains("code")
        {
            topics.insert("technology".to_string());
        }

        // Documentation indicators
        if lower_url.contains("docs.")
            || lower_url.contains("/docs/")
            || lower_url.contains("/documentation/")
            || lower_title.contains("documentation")
            || lower_title.contains("api reference")
        {
            topics.insert("documentation".to_string());
        }

        // News indicators
        if lower_url.contains("news.")
            || lower_url.contains("/news/")
            || result.metadata.content_type == ContentType::Article
        {
            topics.insert("news".to_string());
        }

        // Academic indicators
        if lower_url.contains(".edu")
            || lower_url.contains("arxiv.org")
            || lower_url.contains("scholar.google")
            || lower_title.contains("research")
            || lower_title.contains("study")
        {
            topics.insert("academic".to_string());
        }
    }

    topics.into_iter().collect()
}

/// JSON Schema for search arguments (used for MCP tool definition)
pub fn search_args_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "The search query string"
            },
            "options": {
                "type": "object",
                "description": "Optional search configuration",
                "properties": {
                    "region": {
                        "type": "string",
                        "description": "Region for search results (e.g., 'us-en', 'wt-wt' for worldwide)",
                        "default": "wt-wt"
                    },
                    "safe_search": {
                        "type": "string",
                        "enum": ["OFF", "MODERATE", "STRICT"],
                        "description": "Safe search filtering level",
                        "default": "MODERATE"
                    },
                    "num_results": {
                        "type": "integer",
                        "description": "Maximum number of results to return",
                        "default": 10,
                        "minimum": 1,
                        "maximum": 50
                    },
                    "time_range": {
                        "type": "string",
                        "description": "Time range filter (d=day, w=week, m=month, y=year)"
                    }
                }
            }
        },
        "required": ["query"]
    })
}

/// JSON Schema for visit_page arguments
pub fn visit_page_args_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "format": "uri",
                "description": "URL of the page to visit"
            },
            "selector": {
                "type": "string",
                "description": "Optional CSS selector to target specific content"
            },
            "include_images": {
                "type": "boolean",
                "description": "Whether to include image references in the response",
                "default": false
            }
        },
        "required": ["url"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_search_level_parsing() {
        assert_eq!(
            "OFF".parse::<SafeSearchLevel>().unwrap(),
            SafeSearchLevel::Off
        );
        assert_eq!(
            "MODERATE".parse::<SafeSearchLevel>().unwrap(),
            SafeSearchLevel::Moderate
        );
        assert_eq!(
            "STRICT".parse::<SafeSearchLevel>().unwrap(),
            SafeSearchLevel::Strict
        );
        assert_eq!(
            "moderate".parse::<SafeSearchLevel>().unwrap(),
            SafeSearchLevel::Moderate
        );
    }

    #[test]
    fn test_safe_search_ddg_value() {
        assert_eq!(SafeSearchLevel::Off.to_ddg_value(), -2);
        assert_eq!(SafeSearchLevel::Moderate.to_ddg_value(), -1);
        assert_eq!(SafeSearchLevel::Strict.to_ddg_value(), 1);
    }

    #[test]
    fn test_language_detection() {
        assert_eq!(detect_language("hello world"), "en");
        assert_eq!(detect_language("你好世界"), "zh");
        assert_eq!(detect_language("こんにちは"), "ja");
        assert_eq!(detect_language("안녕하세요"), "ko");
        assert_eq!(detect_language("привет"), "ru");
    }

    #[test]
    fn test_search_args_schema() {
        let schema = search_args_schema();
        assert!(schema["properties"]["query"].is_object());
        assert!(schema["properties"]["options"].is_object());
    }

    #[test]
    fn test_search_response_creation() {
        let results = vec![SearchResult {
            title: "Test".to_string(),
            url: "https://example.com".to_string(),
            description: "Test description".to_string(),
            metadata: ResultMetadata {
                content_type: ContentType::Article,
                source: "example.com".to_string(),
                favicon: None,
                published_date: None,
            },
        }];

        let options = SearchOptions::default();
        let response = SearchResponse::new("test query".to_string(), results, &options);

        assert_eq!(response.response_type, "search_results");
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.metadata.query, "test query");
    }
}
