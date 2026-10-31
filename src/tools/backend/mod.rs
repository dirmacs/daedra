//! Multi-backend search: the backend trait, health tracking, rate limiting,
//! the fan-out provider, and result ranking.
//!
//! - `health`   — circuit breaker state per backend
//! - `limits`   — per-backend and aggregate rate limiters
//! - `provider` — `SearchProvider`, the concurrent fan-out and merge
//! - `query`    — structured query parsing and lenient constraint filtering
//! - `rank`     — relevance scoring and URL canonicalisation
//! - `backend_trait` — the `SearchBackend` trait every backend implements

mod backend_trait;
mod health;
mod limits;
mod provider;
mod query;
mod rank;

pub use backend_trait::SearchBackend;
pub use health::BackendHealth;
pub use provider::SearchProvider;
pub use query::{ConstraintFilter, StructuredQuery, apply_constraints, parse as parse_query};

#[cfg(test)]
mod tests;
