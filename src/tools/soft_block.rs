//! Soft-block classification for scraper backends.
//!
//! Datacenter IPs routinely get served HTTP-200 "blocked" pages that parse to
//! zero results. Treating those as a legitimate empty result conflates
//! anti-bot blocks with genuine no-match queries and starves the fallback
//! chain of the signal it needs to move on. Each scraper backend classifies a
//! zero-result page as either a genuine no-results page or a soft block.

use crate::types::DaedraError;

/// What a zero-result SERP page actually means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyPage {
    /// The engine genuinely has no matches (its own "no results" marker fired).
    GenuineNoResults,
    /// Anti-bot / consent / challenge page in disguise (or an unparseable
    /// page we cannot attribute — equally useless to the caller).
    SoftBlock,
}

/// Classify a zero-result page by scanning case-insensitively for the
/// backend's genuine no-results markers. Unknown pages are treated as soft
/// blocks: a scraper backend that returns neither results nor its own
/// no-results marker is not a trustworthy empty.
pub fn classify(html: &str, no_results_markers: &[&str]) -> EmptyPage {
    let hay = html.to_lowercase();
    if no_results_markers
        .iter()
        .any(|m| hay.contains(&m.to_lowercase()))
    {
        return EmptyPage::GenuineNoResults;
    }
    EmptyPage::SoftBlock
}

/// Find which block marker (if any) is present in the page, for error detail.
pub fn matched_block_marker<'a>(html: &str, block_markers: &[&'a str]) -> Option<&'a str> {
    let hay = html.to_lowercase();
    block_markers.iter().find(|m| hay.contains(*m)).copied()
}

/// Build the error for a soft-blocked page, including which marker fired so
/// aggregate failure messages stay actionable.
pub fn soft_block_error(backend: &str, matched: Option<&str>) -> DaedraError {
    let detail = matched
        .map(|m| format!(" (matched marker: {m})"))
        .unwrap_or_default();
    DaedraError::SearchError(format!(
        "{backend} returned an unparseable page — possible soft block{detail}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genuine_no_results_marker_wins() {
        let html = "<div>No results found for your query</div>";
        assert_eq!(classify(html, &["no results"]), EmptyPage::GenuineNoResults);
    }

    #[test]
    fn unknown_page_is_soft_block() {
        assert_eq!(
            classify("<html></html>", &["no results"]),
            EmptyPage::SoftBlock
        );
    }

    #[test]
    fn marker_matching_is_case_insensitive() {
        assert_eq!(
            matched_block_marker("Page says UNUSUAL TRAFFIC detected", &["unusual traffic"]),
            Some("unusual traffic")
        );
        assert_eq!(matched_block_marker("clean page", &["captcha"]), None);
    }

    #[test]
    fn soft_block_error_names_backend_and_marker() {
        let err = soft_block_error("bing", Some("verify you are human"));
        let msg = err.to_string();
        assert!(msg.contains("bing"), "{msg}");
        assert!(msg.contains("soft block"), "{msg}");
        assert!(msg.contains("verify you are human"), "{msg}");
    }
}
