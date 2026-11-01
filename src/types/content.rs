//! Content classification and per-result metadata.

use serde::{Deserialize, Serialize};

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

    /// How many independent backends returned this URL.
    ///
    /// Agreement is the strongest signal a multi-backend search has: one
    /// engine returning a page is a claim, several returning it is
    /// corroboration. The merge already boosts a result's ranking for it, but
    /// the count was never surfaced, so a caller could not tell a
    /// consensus hit from a single-source one. Defaults to 1 for results that
    /// never went through the merge (single-backend calls, fetches).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub corroborated_by: usize,
}

/// Default corroboration for a result that did not pass through the merge.
fn one() -> usize {
    1
}

fn is_one(n: &usize) -> bool {
    *n <= 1
}

impl Default for ResultMetadata {
    /// A freshly-produced result from a single backend: no agreement yet.
    fn default() -> Self {
        Self {
            content_type: ContentType::Other,
            source: String::new(),
            favicon: None,
            published_date: None,
            corroborated_by: 1,
        }
    }
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

    /// Query constraint dimensions (`site:`, `intitle:`, `filetype:`, ...) that
    /// matched no result and were therefore RELAXED rather than enforced.
    ///
    /// Present so a caller can tell the difference between "the filter
    /// applied" and "the filter could not be satisfied". Without it, a search
    /// that silently ignored `site:arxiv.org` is indistinguishable from one
    /// that honored it — which is the defect this list exists to prevent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relaxed_constraints: Vec<String>,
}
