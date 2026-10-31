//! The crate error type.

use thiserror::Error;

/// Errors surfaced by daedra.
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

    /// Unsupported content type for page extraction
    #[error("Unsupported content type: {0}")]
    UnsupportedContentType(String),

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

/// Crate-wide result alias.
pub type DaedraResult<T> = Result<T, DaedraError>;
