//! Page fetching and content extraction.
//!
//! Split into focused submodules so each concern is independently readable:
//!
//! - [`client`]   — [`FetchClient`], the retry loop, and the public entry points
//! - [`mime`]     — content-type normalisation and binary/textual classification
//! - [`extract`]  — PDF, office-document, and inference-based content extraction
//! - [`markdown`] — HTML-to-Markdown conversion and markup cleanup

mod client;
mod extract;
mod markdown;
mod mime;

pub use client::{FetchClient, fetch_page, is_valid_url};
#[cfg(feature = "crawlberg")]
pub use client::html_to_page_content;

#[cfg(test)]
mod tests;
