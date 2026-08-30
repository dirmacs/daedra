//! Tool implementations for Daedra.
//!
//! Search backends (in fallback order):
//! 1. Mwmbl JSON — no key, general web from any IP
//! 2. Brave / Bing / Google / DuckDuckGo HTML — no key, may meet a CAPTCHA
//! 3. Marginalia, Bing RSS, Google News, Hacker News — machine formats
//! 4. Wikipedia, StackExchange, GitHub, Wiby, DDG Instant — knowledge APIs

pub mod backend;
pub mod bing;
pub mod brave;
pub mod crawl;
pub mod ddg_instant;
pub mod fetch;
pub mod github;
pub mod google;
pub mod marginalia;
pub mod mwmbl;
pub mod rss;
pub mod search;
pub mod soft_block;
pub mod stackexchange;
pub mod wiby;
pub mod wikipedia;

pub use backend::*;
#[cfg(feature = "crawlberg")]
pub use crawl::crawl_site_with_crawlberg;
pub use crawl::{crawl_site, parse_sitemap};
pub use fetch::*;
pub use search::*;
