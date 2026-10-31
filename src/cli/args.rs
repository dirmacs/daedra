//! Command-line argument definitions.

use clap::{Parser, Subcommand, ValueEnum};
use daedra::{VERSION, types::SafeSearchLevel};

/// Daedra - High-performance Web Search and Research MCP Server
#[derive(Parser, Debug)]
#[command(
    name = "daedra",
    version = VERSION,
    author = "DIRMACS Global Services <build@dirmacs.com>",
    about = "A high-performance web search and research MCP server",
    long_about = "Daedra is a Model Context Protocol (MCP) server that provides web search and research capabilities.\n\n\
                  It can be used as:\n\
                  - An MCP server (STDIO or SSE transport)\n\
                  - A CLI tool for direct searches and page fetching\n\n\
                  For more information, visit: https://github.com/dirmacs/daedra"
)]
pub struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    pub(crate) verbose: bool,

    /// Disable all logging output (useful for stdio transport)
    #[arg(short, long, global = true)]
    pub(crate) quiet: bool,

    /// Output format
    #[arg(short, long, global = true, default_value = "pretty")]
    pub(crate) format: OutputFormat,

    /// Disable colored output
    #[arg(long, global = true)]
    pub(crate) no_color: bool,

    #[command(flatten)]
    pub(crate) globals: GlobalArgs,

    #[command(subcommand)]
    pub(crate) command: Commands,
}

/// Output format options
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum OutputFormat {
    /// Pretty-printed human-readable output
    #[default]
    Pretty,
    /// JSON output
    Json,
    /// Compact JSON output
    JsonCompact,
}

/// Global options: accepted before or after any subcommand.
#[derive(clap::Args, Clone, Debug)]
pub struct GlobalArgs {
    /// Disable result caching for search and fetch
    #[arg(long, global = true)]
    pub(crate) no_cache: bool,

    /// Cache TTL in seconds
    #[arg(long, global = true, default_value = "300")]
    pub(crate) cache_ttl: u64,

    /// Path to a TOML config file. Defaults to
    /// ~/.config/daedra/daedra.toml when that file exists.
    #[arg(long, global = true)]
    pub(crate) config: Option<String>,
}

/// Available commands
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the MCP server
    Serve {
        /// Transport type to use
        #[arg(short, long, default_value = "stdio")]
        transport: TransportOption,

        /// Port for SSE transport (only used with --transport sse)
        #[arg(short, long, default_value = "3000")]
        port: u16,

        /// Host to bind to for SSE transport
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(long, value_enum)]
        shell: clap_complete::Shell,
    },

    /// Perform a web search
    Search {
        /// Search query
        query: String,

        /// Number of results to return
        #[arg(short, long, default_value = "10")]
        num_results: usize,

        /// Search region (e.g., 'us-en', 'wt-wt' for worldwide)
        #[arg(short, long, default_value = "wt-wt")]
        region: String,

        /// Safe search level
        #[arg(short, long, default_value = "moderate")]
        safe_search: SafeSearchOption,

        /// Time range filter (d=day, w=week, m=month, y=year)
        #[arg(short = 't', long)]
        time_range: Option<String>,

        /// Only use these backends (repeat for several; names from `daedra check`)
        #[arg(long = "backend")]
        backends: Vec<String>,

        /// Skip these backends (repeat for several)
        #[arg(long = "exclude")]
        exclude_backends: Vec<String>,
    },

    /// Fetch and extract content from a web page
    Fetch {
        /// URL to fetch
        url: String,

        /// CSS selector to target specific content
        #[arg(short, long)]
        selector: Option<String>,

        /// Include images in output
        #[arg(long)]
        include_images: bool,

        /// Per-request timeout in seconds
        #[arg(short = 't', long, default_value = "30")]
        timeout: u64,
    },

    /// Crawl a website and extract content from all discovered pages
    Crawl {
        /// Root URL to start crawling from
        url: String,

        /// Maximum number of pages to fetch
        #[arg(short, long, default_value = "25")]
        max_pages: usize,

        /// Maximum concurrent fetches
        #[arg(short, long, default_value = "4")]
        concurrency: usize,

        /// Link layers to follow past discovery (1 = discovered pages only, max 5)
        #[arg(short = 'd', long, default_value = "1")]
        depth: usize,

        /// Delay between page fetch starts in milliseconds
        #[arg(long, default_value = "0")]
        delay_ms: u64,

        /// Ignore robots.txt exclusions for this crawl
        #[arg(long)]
        ignore_robots: bool,

        /// Crawl engine: native (default) or crawlberg (needs the
        /// crawlberg cargo feature)
        #[arg(long, value_enum, default_value = "native")]
        engine: CrawlEngineOption,
    },

    /// Show server information
    Info,

    /// Validate configuration and dependencies
    Check,
}

/// Crawl engine choices
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum CrawlEngineOption {
    /// Built-in crawler (sitemap, robots, depth layers)
    #[default]
    Native,
    /// crawlberg engine (requires the crawlberg cargo feature)
    Crawlberg,
}

/// Transport options for the serve command
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum TransportOption {
    /// Standard input/output (for MCP clients)
    #[default]
    Stdio,
    /// Server-Sent Events over HTTP
    Sse,
}

/// Safe search options
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum SafeSearchOption {
    /// No filtering
    Off,
    /// Moderate filtering
    #[default]
    Moderate,
    /// Strict filtering
    Strict,
}

impl From<SafeSearchOption> for SafeSearchLevel {
    fn from(opt: SafeSearchOption) -> Self {
        match opt {
            SafeSearchOption::Off => SafeSearchLevel::Off,
            SafeSearchOption::Moderate => SafeSearchLevel::Moderate,
            SafeSearchOption::Strict => SafeSearchLevel::Strict,
        }
    }
}

#[cfg(test)]
pub fn safe_search_from_u8(v: u8) -> Option<SafeSearchLevel> {
    match v {
        0 => Some(SafeSearchLevel::Off),
        1 => Some(SafeSearchLevel::Moderate),
        2 => Some(SafeSearchLevel::Strict),
        _ => None,
    }
}
