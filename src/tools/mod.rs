//! Tool implementations for Daedra.
//!
//! Search backends (in fallback order):
//! 1. Serper.dev — Google results via API (needs SERPER_API_KEY)
//! 2. Tavily — AI-optimized search (needs TAVILY_API_KEY)
//! 3. You.com — unified web/news search via API (needs YDC_API_KEY)
//! 4. Bing HTML scraping — no key, but blocked from most datacenter IPs
//! 5. Wikipedia — always works, knowledge-focused
//! 6. StackExchange — always works, technical Q&A
//! 7. DuckDuckGo — blocked from datacenter IPs, last resort

pub mod backend;
pub mod bing;
pub mod brave;
pub mod crawl;
pub mod ddg_instant;
pub mod fetch;
pub mod github;
pub mod google;
pub mod marginalia;
pub mod mojeek;
pub mod rss;
pub mod search;
pub mod serper;
pub mod soft_block;
pub mod stackexchange;
pub mod tavily;
pub mod wiby;
pub mod wikipedia;
pub mod youcom;

pub use backend::*;
pub use crawl::{crawl_site, parse_sitemap};
pub use fetch::*;
pub use search::*;
