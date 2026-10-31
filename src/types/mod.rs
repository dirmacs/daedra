//! Shared types for search, fetching, crawling, and query analysis.
//!
//! Split by domain so a reader looking for one shape does not have to scan
//! every type in the crate:
//!
//! - [`error`]    — [`DaedraError`]
//! - [`search`]   — search args/options/results and region resolution
//! - [`content`]  — content classification and result metadata
//! - [`page`]     — fetched page content and links
//! - [`crawl`]    — crawl request and result types
//! - [`analysis`] — language detection and topic tagging
//! - [`schemas`]  — JSON Schemas for MCP clients

mod analysis;
mod content;
mod crawl;
mod error;
mod page;
mod schemas;
mod search;

pub use content::{
    ContentType, QueryAnalysis, ResultMetadata, SearchContext, SearchMetadata, SearchResult,
};
pub use crawl::{CrawlArgs, CrawlError, CrawlResult, CrawlSummary, CrawledPage};
pub use error::{DaedraError, DaedraResult};
pub use page::{PageContent, PageLink};
pub use schemas::{crawl_args_schema, search_args_schema, visit_page_args_schema};
pub use search::{
    SafeSearchLevel, SearchArgs, SearchOptions, SearchResponse, VisitPageArgs,
    region_to_gl_hl, region_to_mkt, time_range_secs,
};

#[cfg(test)]
mod tests;
