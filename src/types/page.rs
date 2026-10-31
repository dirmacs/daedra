//! Fetched page content and outbound links.

use serde::{Deserialize, Serialize};

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

    /// Content type of the source document ("html", "pdf", "text/svg+xml",
    /// "application/vnd.openxmlformats-...wordprocessingml.document", ...).
    /// Agents use this to tell article text from source dumps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,

    /// Links found on the page
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<PageLink>>,
}

/// A link found on a page
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageLink {
    /// Link text
    pub text: String,

    /// Link URL
    pub url: String,
}
