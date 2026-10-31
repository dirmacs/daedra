//! Search request, options, results, and region/language resolution.

use serde::{Deserialize, Serialize};


use super::analysis::{detect_language, detect_topics};
use super::content::{QueryAnalysis, SearchContext, SearchMetadata, SearchResult};
use super::error::DaedraError;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
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

    /// Convert to Bing's `adlt` query parameter value
    pub fn to_bing_value(&self) -> &'static str {
        match self {
            SafeSearchLevel::Off => "off",
            SafeSearchLevel::Moderate => "moderate",
            SafeSearchLevel::Strict => "strict",
        }
    }
}

/// Split a DuckDuckGo-style region tag (`"us-en"`, `"wt-wt"`) into an
/// optional ISO country code (`gl`) and language code (`hl`), as used by
/// Google. Worldwide (`"wt-wt"`) yields no `gl`.
pub fn region_to_gl_hl(region: &str) -> (Option<String>, Option<String>) {
    let mut parts = region.splitn(2, '-');
    let geo = parts.next().unwrap_or_default();
    let lang = parts.next().unwrap_or_default();
    if geo.is_empty() || geo == "wt" {
        return (None, None);
    }
    (
        Some(geo.to_lowercase()),
        if lang.is_empty() {
            None
        } else {
            Some(lang.to_lowercase())
        },
    )
}

/// Build Bing's `mkt` market tag (`"en-US"`) from a DDG-style region tag.
/// Returns `None` for worldwide or unparseable regions.
pub fn region_to_mkt(region: &str) -> Option<String> {
    let (geo, lang) = region_to_gl_hl(region);
    let lang = lang?;
    let geo = geo?;
    Some(format!("{}-{}", lang, geo.to_uppercase()))
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Restrict the search to these backend names. Unknown names are warned
    /// about and ignored; an empty selection after filtering is an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backends: Option<Vec<String>>,

    /// Remove these backend names from the search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_backends: Option<Vec<String>>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            region: "wt-wt".to_string(),
            safe_search: SafeSearchLevel::Moderate,
            num_results: 10,
            time_range: None,
            backends: None,
            exclude_backends: None,
        }
    }
}

/// Seconds covered by a time-range letter (`d`, `w`, `m`, `y`). `None` for
/// an unknown letter.
pub fn time_range_secs(letter: &str) -> Option<u64> {
    match letter {
        "d" => Some(24 * 60 * 60),
        "w" => Some(7 * 24 * 60 * 60),
        "m" => Some(30 * 24 * 60 * 60),
        "y" => Some(365 * 24 * 60 * 60),
        _ => None,
    }
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

/// Aggregated response from a search across backends.
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
                dropped_constraints: Vec::new(),
            },
        }
    }

    /// Record query-constraint dimensions that could not be satisfied.
    pub fn with_dropped_constraints(mut self, dropped: Vec<String>) -> Self {
        self.metadata.dropped_constraints = dropped;
        self
    }
}
