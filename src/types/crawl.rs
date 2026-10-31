//! Crawl request and result types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request parameters for a site crawl: seed URL, page and depth limits,
/// concurrency, and which crawler engine to use.
pub struct CrawlArgs {
    /// The root URL of the site to crawl
    pub root_url: String,

    /// Upper bound on the number of pages to fetch
    #[serde(default = "default_crawl_max_pages")]
    pub max_pages: usize,

    /// Maximum number of concurrent fetches
    #[serde(default = "default_crawl_concurrency")]
    pub concurrency: usize,

    /// Link layers to follow past discovery (1 = discovered pages only, 5 max)
    #[serde(default = "default_crawl_depth")]
    pub depth: usize,

    /// Delay between page fetch starts in milliseconds (politeness for small sites)
    #[serde(default)]
    pub delay_ms: u64,

    /// Ignore robots.txt exclusions for this crawl
    #[serde(default)]
    pub ignore_robots: bool,
}

fn default_crawl_max_pages() -> usize {
    25
}
fn default_crawl_concurrency() -> usize {
    4
}
fn default_crawl_depth() -> usize {
    1
}

/// A single page fetched by `crawl_site`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawledPage {
    /// Absolute URL of the fetched page
    pub url: String,

    /// Extracted page title (may be empty)
    pub title: String,

    /// Content converted to Markdown
    pub markdown: String,

    /// Outbound link URLs discovered on the page
    pub links: Vec<String>,
}

/// Error record for a URL that could not be fetched during a crawl.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlError {
    /// URL that failed
    pub url: String,

    /// Human-readable error reason
    pub error: String,
}

/// Summary of crawl activity — counts only, no content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlSummary {
    /// Page budget requested by the caller (post-clamp)
    pub requested: usize,

    /// Number of pages successfully fetched
    pub fetched: usize,

    /// Number of URLs that errored out
    pub failed: usize,
}

/// Return value of `crawl_site`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlResult {
    /// The normalized root URL that was crawled
    pub root_url: String,

    /// Whether a sitemap.xml (or alias) was found and used
    pub sitemap_found: bool,

    /// Counts-only activity summary
    pub summary: CrawlSummary,

    /// Successfully fetched pages
    pub pages: Vec<CrawledPage>,

    /// Per-URL errors that were silently dropped during the batch
    pub errors: Vec<CrawlError>,
}
