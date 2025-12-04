//! # Daedra - Web Search and Research MCP Server
//!
//! Daedra is a high-performance Model Context Protocol (MCP) server that provides
//! web search and research capabilities. It is designed to be used as both a library
//! for programmatic access and as a standalone CLI binary.
//!
//! ## Features
//!
//! - **Web Search**: Search the web using DuckDuckGo with customizable options
//! - **Page Fetching**: Extract and convert web page content to Markdown
//! - **Caching**: Built-in response caching for improved performance
//! - **Dual Transport**: Support for both STDIO and HTTP (SSE) transports
//! - **Concurrent Processing**: Parallel processing of search results
//!
//! ## Quick Start
//!
//! ### As a Library
//!
//! ```rust,no_run
//! use daedra::{DaedraServer, ServerConfig, TransportType};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = ServerConfig::default();
//!     let server = DaedraServer::new(config)?;
//!     server.run(TransportType::Stdio).await?;
//!     Ok(())
//! }
//! ```
//!
//! ### Direct Tool Usage
//!
//! ```rust,no_run
//! use daedra::{SearchArgs, tools::search};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let args = SearchArgs {
//!         query: "Rust programming".to_string(),
//!         options: None,
//!     };
//!     let results = search::perform_search(&args).await?;
//!     println!("{:?}", results);
//!     Ok(())
//! }
//! ```
//!
//! ## Architecture
//!
//! The crate is organized into several modules:
//!
//! - [`server`]: MCP server implementation with transport handling
//! - [`tools`]: Individual tool implementations (search, fetch, etc.)
//! - [`types`]: Common types and schemas
//! - [`cache`]: Caching infrastructure for performance optimization

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![warn(rustdoc::missing_crate_level_docs)]

pub mod cache;
pub mod server;
pub mod tools;
pub mod types;

// Re-export commonly used items at crate root
pub use cache::SearchCache;
pub use server::{DaedraServer, ServerConfig, TransportType};
pub use types::{
    ContentType, DaedraError, DaedraResult, SafeSearchLevel, SearchArgs, SearchOptions,
    SearchResponse, SearchResult, VisitPageArgs,
};

/// Crate version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Server name for MCP protocol
pub const SERVER_NAME: &str = "daedra";

/// Server description
pub const SERVER_DESCRIPTION: &str = "Web search and research MCP server";
