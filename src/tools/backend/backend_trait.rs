//! The [`SearchBackend`] trait that every search backend implements.

use crate::types::{DaedraResult, SearchArgs, SearchResponse};
use async_trait::async_trait;

/// Trait for search backends. Each backend implements web search
/// and returns results in the common SearchResponse format.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait SearchBackend: Send + Sync {
    /// Execute a search query and return results.
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse>;

    /// Backend name for logging and diagnostics.
    fn name(&self) -> &str;

    /// Whether this backend requires an API key.
    fn requires_api_key(&self) -> bool {
        false
    }

    /// Whether this backend is available (has required config/keys).
    fn is_available(&self) -> bool {
        true
    }
}
