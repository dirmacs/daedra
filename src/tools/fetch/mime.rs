//! Content-type normalisation and binary/textual classification.

use crate::types::{DaedraError, DaedraResult};
use url::Url;

/// Maximum content size (10MB)
pub(super) const MAX_CONTENT_SIZE: usize = 10 * 1024 * 1024;

pub(super) fn normalize_content_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_lowercase()
}

const BINARY_CONTENT_PREFIXES: &[&str] = &[
    "image/",
    "video/",
    "audio/",
    "application/vnd.openxmlformats-",
];

const BINARY_CONTENT_EXACT: &[&str] = &[
    "application/zip",
    "application/gzip",
    "application/x-tar",
    "application/octet-stream",
    "application/vnd.ms-excel",
];

pub(super) fn is_known_binary_content_type(content_type: &str) -> bool {
    let ct = normalize_content_type(content_type);
    BINARY_CONTENT_PREFIXES
        .iter()
        .any(|prefix| ct.starts_with(prefix))
        || BINARY_CONTENT_EXACT.iter().any(|exact| ct == *exact)
}

pub(super) fn is_binary_mime(mime: &str) -> bool {
    is_known_binary_content_type(mime)
        || mime == "application/pdf"
        || mime.starts_with("application/vnd.")
}

pub(super) fn bytes_to_utf8_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

pub(super) fn check_body_size(size: usize) -> DaedraResult<()> {
    if size > MAX_CONTENT_SIZE {
        return Err(DaedraError::FetchError("Content too large".to_string()));
    }
    Ok(())
}

pub(super) fn validate_url(url: &str) -> DaedraResult<Url> {
    let parsed_url = Url::parse(url).map_err(DaedraError::UrlParseError)?;

    if !matches!(parsed_url.scheme(), "http" | "https") {
        return Err(DaedraError::InvalidArguments(
            "Only HTTP(S) URLs are supported".to_string(),
        ));
    }

    Ok(parsed_url)
}
