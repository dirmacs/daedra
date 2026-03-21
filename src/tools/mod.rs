//! Tool implementations for Daedra.
//!
//! Search backends (in fallback order):
//! 1. Serper.dev — Google results via API (needs SERPER_API_KEY)
//! 2. Tavily — AI-optimized search (needs TAVILY_API_KEY)
//! 3. Bing HTML scraping — no key, but blocked from most datacenter IPs
//! 4. Wikipedia — always works, knowledge-focused
//! 5. StackExchange — always works, technical Q&A
//! 6. DuckDuckGo — blocked from datacenter IPs, last resort

pub mod backend;
pub mod bing;
pub mod fetch;
pub mod search;
pub mod serper;
pub mod stackexchange;
pub mod tavily;
pub mod wikipedia;

pub use backend::*;
pub use fetch::*;
pub use search::*;
