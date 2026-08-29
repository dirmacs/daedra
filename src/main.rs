//! Daedra CLI - Web Search and Research MCP Server
//!
//! A command-line interface for the Daedra MCP server.

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use colored::Colorize;
#[cfg(feature = "crawlberg")]
use daedra::tools::crawl_site_with_crawlberg;
use daedra::{
    DaedraResult, SERVER_NAME, VERSION,
    cache::CacheConfig,
    server::{DaedraServer, ServerConfig, TransportType},
    tools::{crawl_site, fetch, search},
    types::{
        CrawlArgs, CrawlResult, DaedraError, PageContent, SafeSearchLevel, SearchArgs,
        SearchOptions, SearchResult, VisitPageArgs,
    },
};
use std::net::ToSocketAddrs;
use std::time::Duration;
use tracing_subscriber::{EnvFilter, fmt};

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
struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Disable all logging output (useful for stdio transport)
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Output format
    #[arg(short, long, global = true, default_value = "pretty")]
    format: OutputFormat,

    /// Disable colored output
    #[arg(long, global = true)]
    no_color: bool,

    #[command(flatten)]
    globals: GlobalArgs,

    #[command(subcommand)]
    command: Commands,
}

/// Output format options
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
enum OutputFormat {
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
struct GlobalArgs {
    /// Disable result caching for search and fetch
    #[arg(long, global = true)]
    no_cache: bool,

    /// Cache TTL in seconds
    #[arg(long, global = true, default_value = "300")]
    cache_ttl: u64,

    /// Path to a TOML config file. Defaults to
    /// ~/.config/daedra/daedra.toml when that file exists.
    #[arg(long, global = true)]
    config: Option<String>,
}

/// Available commands
#[derive(Subcommand, Debug)]
enum Commands {
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
enum CrawlEngineOption {
    /// Built-in crawler (sitemap, robots, depth layers)
    #[default]
    Native,
    /// crawlberg engine (requires the crawlberg cargo feature)
    Crawlberg,
}

/// Transport options for the serve command
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum TransportOption {
    /// Standard input/output (for MCP clients)
    #[default]
    Stdio,
    /// Server-Sent Events over HTTP
    Sse,
}

/// Safe search options
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum SafeSearchOption {
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
fn safe_search_from_u8(v: u8) -> Option<SafeSearchLevel> {
    match v {
        0 => Some(SafeSearchLevel::Off),
        1 => Some(SafeSearchLevel::Moderate),
        2 => Some(SafeSearchLevel::Strict),
        _ => None,
    }
}

fn check_section_message(title: &str) -> String {
    match title {
        "Configuration Check" => "
Checking Daedra configuration..."
            .to_string(),
        "Connectivity Test" => "
Testing search functionality..."
            .to_string(),
        _ => title.to_string(),
    }
}

fn check_summary_message(all_ok: bool, no_color: bool) -> String {
    if all_ok {
        if no_color {
            "All checks passed!".to_string()
        } else {
            "✓ All checks passed!".to_string()
        }
    } else if no_color {
        "Some checks failed. See above for details.".to_string()
    } else {
        "✗ Some checks failed. See above for details.".to_string()
    }
}

fn should_print_banner(
    verbose: bool,
    quiet: bool,
    format: OutputFormat,
    transport: TransportOption,
) -> bool {
    verbose
        && !quiet
        && !matches!(format, OutputFormat::Json | OutputFormat::JsonCompact)
        && matches!(transport, TransportOption::Sse)
}

impl Commands {
    async fn run(
        self,
        format: OutputFormat,
        verbose: bool,
        quiet: bool,
        no_color: bool,
        globals: &GlobalArgs,
    ) -> DaedraResult<()> {
        // Flags win over the config file; the file wins over built-in
        // defaults. Only values left at their default pick up file entries.
        let file_cfg = load_file_config(globals.config.as_deref());
        let mut globals = globals.clone();
        if globals.cache_ttl == 300 {
            globals.cache_ttl = file_cfg.cache_ttl_secs.unwrap_or(300);
        }
        match self {
            Commands::Serve {
                transport,
                port,
                host,
            } => {
                if should_print_banner(verbose, quiet, format, transport) {
                    print_banner();
                }
                run_serve(
                    transport,
                    port,
                    host,
                    build_cache_config(globals.no_cache, globals.cache_ttl),
                )
                .await
            },

            Commands::Completions { shell } => {
                let mut cmd = Cli::command();
                clap_complete::generate(shell, &mut cmd, SERVER_NAME, &mut std::io::stdout());
                Ok(())
            },

            Commands::Search {
                query,
                num_results,
                region,
                safe_search,
                time_range,
                backends,
                exclude_backends,
            } => {
                let mut opts = SearchOptions {
                    region,
                    safe_search: safe_search.into(),
                    num_results,
                    time_range,
                    backends: (!backends.is_empty()).then_some(backends),
                    exclude_backends: (!exclude_backends.is_empty()).then_some(exclude_backends),
                };
                if opts.num_results == 10 {
                    opts.num_results = file_cfg.num_results.unwrap_or(10);
                }
                if opts.region == "wt-wt" {
                    opts.region = file_cfg.region.unwrap_or_else(|| "wt-wt".to_string());
                }
                run_search(query, opts, format, no_color, &globals).await
            },

            Commands::Fetch {
                url,
                selector,
                include_images,
                timeout,
            } => {
                let timeout = if timeout == 30 {
                    file_cfg.timeout_secs.unwrap_or(30)
                } else {
                    timeout
                };
                run_fetch(
                    url,
                    selector,
                    include_images,
                    timeout,
                    format,
                    no_color,
                    &globals,
                )
                .await
            },

            Commands::Crawl {
                url,
                max_pages,
                concurrency,
                depth,
                delay_ms,
                ignore_robots,
                engine,
            } => {
                run_crawl(
                    CrawlArgs {
                        root_url: url,
                        max_pages,
                        concurrency,
                        depth,
                        delay_ms,
                        ignore_robots,
                    },
                    engine,
                    format,
                    no_color,
                )
                .await
            },

            Commands::Info => {
                run_info(no_color, format);
                Ok(())
            },

            Commands::Check => run_check(no_color, format).await,
        }
    }
}

struct CheckReporter {
    no_color: bool,
    /// Every reported outcome, kept in order so `-f json` can print the
    /// same report machine-readably.
    entries: std::cell::RefCell<Vec<(String, bool, String)>>,
}

impl CheckReporter {
    fn new(no_color: bool) -> Self {
        Self {
            no_color,
            entries: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// Emit a report entry with a stable machine key (`status`, `fetch`,
    /// `connectivity`, `api_keys`, `backends`, `summary`).
    fn entry(&self, key: &str, ok: bool, message: &str) {
        self.entries
            .borrow_mut()
            .push((key.to_string(), ok, message.to_string()));
    }

    fn to_json(&self, all_ok: bool) -> serde_json::Value {
        let checks: Vec<serde_json::Value> = self
            .entries
            .borrow()
            .iter()
            .map(|(key, ok, message)| {
                serde_json::json!({ "check": key, "ok": ok, "detail": message })
            })
            .collect();
        serde_json::json!({ "all_ok": all_ok, "checks": checks })
    }

    fn section(&self, title: &str) {
        if self.no_color {
            println!("{}", check_section_message(title));
        } else {
            print_section(title);
        }
    }

    fn ok(&self, message: &str) {
        if self.no_color {
            println!("  [OK] {message}");
        } else {
            print_success(message);
        }
    }

    fn fail(&self, message: &str) {
        if self.no_color {
            println!("  [FAIL] {message}");
        } else {
            print_error(message);
        }
    }

    fn warn(&self, message: &str) {
        if self.no_color {
            println!("  [WARN] {message}");
        } else {
            println!("  {} {}", "⚠".yellow(), message.yellow());
        }
    }

    fn backends(&self, backends: &[&str]) {
        if self.no_color {
            println!("  Backends: {}", backends.join(", "));
        } else {
            println!(
                "  {} {} backends: {}",
                "✓".green(),
                backends.len(),
                backends.join(", ")
            );
        }
    }

    fn summary(&self, all_ok: bool) {
        println!();
        let message = check_summary_message(all_ok, self.no_color);
        if all_ok {
            if self.no_color {
                println!("{message}");
            } else {
                println!("{}", message.green().bold());
            }
        } else if self.no_color {
            println!("{message}");
            std::process::exit(1);
        } else {
            println!("{}", message.red().bold());
            std::process::exit(1);
        }
    }
}

fn check_search_client(reporter: &CheckReporter) -> bool {
    match search::SearchClient::new() {
        Ok(_) => {
            reporter.ok("Search client initialized");
            reporter.entry("status", true, "Search client initialized");
            true
        },
        Err(e) => {
            reporter.fail(&format!("Search client: {e}"));
            reporter.entry("status", false, &format!("Search client: {e}"));
            false
        },
    }
}

/// Report which optional API keys are set. A missing key is not a failure —
/// unkeyed search still works — but `check` must not hide the quality cliff.
fn check_api_keys(reporter: &CheckReporter) {
    let keys = [
        ("SERPER_API_KEY", "Serper (top-tier Google results)"),
        ("TAVILY_API_KEY", "Tavily (LLM-oriented search)"),
        ("GITHUB_TOKEN", "GitHub (higher rate limits)"),
    ];
    let mut any_set = false;
    for (name, what) in keys {
        if std::env::var(name).is_ok_and(|v| !v.trim().is_empty()) {
            reporter.ok(&format!("{name} set ({what})"));
            any_set = true;
        } else {
            reporter.warn(&format!("{name} not set — {what} disabled"));
        }
    }
    if !any_set {
        reporter.warn(
            "No API keys set: search runs on unkeyed backends only (wiki, HN, SO, \
             GitHub, RSS) — not general web search",
        );
    }
    reporter.entry(
        "api_keys",
        true,
        if any_set {
            "at least one API key set"
        } else {
            "no API keys set"
        },
    );
}

fn check_fetch_client(reporter: &CheckReporter) -> bool {
    match fetch::FetchClient::new() {
        Ok(_) => {
            reporter.ok("Fetch client initialized");
            true
        },
        Err(e) => {
            reporter.fail(&format!("Fetch client: {e}"));
            false
        },
    }
}

async fn check_search_connectivity(reporter: &CheckReporter) -> bool {
    let test_args = SearchArgs {
        query: "test".to_string(),
        options: Some(SearchOptions {
            num_results: 1,
            ..Default::default()
        }),
    };

    let provider = daedra::tools::SearchProvider::auto();
    let backends = provider.available_backends();
    reporter.backends(&backends);

    match provider.search(&test_args).await {
        Ok(response) => {
            if response.data.is_empty() {
                reporter.warn("Search returned no results");
                reporter.entry("connectivity", true, "search ran, 0 results");
            } else {
                reporter.ok("Search connectivity verified");
                reporter.entry("connectivity", true, "search verified");
            }
            true
        },
        Err(e) => {
            reporter.fail(&format!("Search test: {e}"));
            reporter.entry("connectivity", false, &format!("Search test: {e}"));
            false
        },
    }
}

/// Set up logging with configurable output destination
///
/// # Arguments
/// * `verbose` - Enable debug-level logging
/// * `use_stderr` - Write logs to stderr instead of stdout (required for stdio transport)
/// * `quiet` - Disable all logging output
fn setup_logging(verbose: bool, use_stderr: bool, quiet: bool) {
    // If quiet mode, use a very restrictive filter that effectively disables logging
    let filter = if quiet {
        EnvFilter::new("off")
    } else if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

    let subscriber = fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_thread_ids(false);

    // For stdio transport, logs MUST go to stderr to avoid corrupting the JSON-RPC stream
    if use_stderr {
        subscriber.with_writer(std::io::stderr).init();
    } else {
        subscriber.init();
    }
}

fn print_banner() {
    println!(
        r#"
{}
╔═══════════════════════════════════════════════════════════════╗
║                                                               ║
║   {}    ║
║   {}                         ║
║                                                               ║
║   A high-performance web search and research MCP server       ║
║                                                               ║
╚═══════════════════════════════════════════════════════════════╝
"#,
        "".clear(),
        format!("🔍 DAEDRA v{}", VERSION).bright_cyan().bold(),
        "by DIRMACS Global Services".bright_black(),
    );
}

fn print_success(message: &str) {
    println!("{} {}", "✓".green().bold(), message);
}

fn print_error(message: &str) {
    eprintln!("{} {}", "✗".red().bold(), message);
}

fn print_info(label: &str, value: &str) {
    println!("  {} {}", format!("{}:", label).bright_blue(), value);
}

fn print_section(title: &str) {
    println!("{}", format_section(title));
}

fn format_section(title: &str) -> String {
    format!(
        "\n{}\n{}",
        title.yellow().bold(),
        "─".repeat(40).bright_black()
    )
}

fn format_info(label: &str, value: &str) -> String {
    format!("  {} {}\n", format!("{}:", label).bright_blue(), value)
}

/// Defaults loaded from a TOML file (--config or
/// ~/.config/daedra/daedra.toml). Flags always win over file values.
#[derive(Debug, Default, serde::Deserialize)]
struct FileConfig {
    num_results: Option<usize>,
    region: Option<String>,
    timeout_secs: Option<u64>,
    cache_ttl_secs: Option<u64>,
}

fn load_file_config(path: Option<&str>) -> FileConfig {
    let resolved = path.map(std::path::PathBuf::from).or_else(|| {
        let home = std::env::var("HOME").ok()?;
        let cfg = std::path::PathBuf::from(home).join(".config/daedra/daedra.toml");
        cfg.is_file().then_some(cfg)
    });
    let Some(p) = resolved else {
        return FileConfig::default();
    };
    match std::fs::read_to_string(&p) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("warning: could not parse {}: {e}", p.display());
            FileConfig::default()
        }),
        Err(_) => FileConfig::default(),
    }
}

fn build_cache_config(no_cache: bool, cache_ttl: u64) -> CacheConfig {
    if no_cache {
        CacheConfig {
            enabled: false,
            ..Default::default()
        }
    } else {
        CacheConfig {
            ttl: Duration::from_secs(cache_ttl),
            enabled: true,
            ..Default::default()
        }
    }
}

fn parse_host_octets(host: &str) -> DaedraResult<[u8; 4]> {
    if host.eq_ignore_ascii_case("localhost") {
        return Ok([127, 0, 0, 1]);
    }
    let parts: Vec<u8> = host.split('.').filter_map(|s| s.parse().ok()).collect();
    if parts.len() == 4 {
        return Ok([parts[0], parts[1], parts[2], parts[3]]);
    }
    // Hostname: resolve it. SSE binds an IPv4 socket, so take the first
    // resolved A record.
    let resolved = (host, 0u16)
        .to_socket_addrs()
        .map_err(|e| DaedraError::InvalidArguments(format!("cannot resolve host {host:?}: {e}")))?
        .find(|a| a.is_ipv4());
    match resolved.and_then(|a| match a.ip() {
        std::net::IpAddr::V4(v4) => Some(v4.octets()),
        std::net::IpAddr::V6(_) => None,
    }) {
        Some(octets) => Ok(octets),
        None => Err(DaedraError::InvalidArguments(format!(
            "cannot resolve host {host:?} to an IPv4 address"
        ))),
    }
}

async fn run_serve(
    transport: TransportOption,
    port: u16,
    host: String,
    cache: CacheConfig,
) -> DaedraResult<()> {
    let config = ServerConfig {
        cache,
        verbose: false,
        ..Default::default()
    };

    let server = DaedraServer::new(config)?;

    let transport_type = match transport {
        TransportOption::Stdio => TransportType::Stdio,
        TransportOption::Sse => TransportType::Sse {
            port,
            host: parse_host_octets(&host)?,
        },
    };

    server.run(transport_type).await
}

fn format_page_header(title: &str, no_color: bool) -> String {
    if no_color {
        format!("\n{}\n{}", title, "=".repeat(50))
    } else {
        format!(
            "\n{}\n{}",
            title.white().bold(),
            "─".repeat(40).bright_black()
        )
    }
}

fn format_search_header_pretty(query: &str, count: usize, region: &str, no_color: bool) -> String {
    if no_color {
        format!(
            "\nSearch Results for: {}\n{}\nFound {} results in region '{}'\n\n",
            query,
            "=".repeat(50),
            count,
            region
        )
    } else {
        format!(
            "{}\nFound {} results in region '{}'\n\n",
            format_section(&format!("Search Results for: {}", query.cyan())),
            count.to_string().green(),
            region.bright_blue()
        )
    }
}

fn format_search_result_pretty(result: &SearchResult, index: usize, no_color: bool) -> String {
    if no_color {
        format!(
            "{}. {}\n   URL: {}\n   {}\n   Source: {} | Type: {:?}\n\n",
            index + 1,
            result.title,
            result.url,
            result.description,
            result.metadata.source,
            result.metadata.content_type
        )
    } else {
        format!(
            "{} {}\n   {} {}\n   {}\n   {} {} {} {:?}\n\n",
            format!("{}.", index + 1).bright_black(),
            result.title.white().bold(),
            "URL:".bright_black(),
            result.url.bright_blue().underline(),
            result.description.bright_white(),
            "Source:".bright_black(),
            result.metadata.source.yellow(),
            "|".bright_black(),
            result.metadata.content_type
        )
    }
}

fn format_page_content_pretty(content: &PageContent, no_color: bool) -> String {
    let mut out = format_page_header(&content.title, no_color);
    if no_color {
        out.push_str(&format!(
            "URL: {}\nFetched: {}\nWords: {}\n\n{}\n",
            content.url, content.timestamp, content.word_count, content.content
        ));
        if let Some(links) = &content.links {
            out.push_str(&format!("\nLinks found ({}):\n", links.len()));
            for link in links.iter().take(10) {
                out.push_str(&format!("  - {} ({})\n", link.text, link.url));
            }
        }
    } else {
        out.push_str(&format_info(
            "URL",
            &content.url.bright_blue().underline().to_string(),
        ));
        out.push_str(&format_info("Fetched", &content.timestamp));
        out.push_str(&format_info(
            "Words",
            &content.word_count.to_string().green().to_string(),
        ));
        out.push_str(&format!("\n{}\n", content.content));
        if let Some(links) = &content.links {
            out.push_str(&format_section(&format!("Links found ({})", links.len())));
            for link in links.iter().take(10) {
                out.push_str(&format!(
                    "  {} {} {}\n",
                    "→".bright_black(),
                    link.text.white(),
                    format!("({})", link.url).bright_blue()
                ));
            }
        }
    }
    out
}

fn print_search_header_pretty(query: &str, count: usize, region: &str, no_color: bool) {
    print!(
        "{}",
        format_search_header_pretty(query, count, region, no_color)
    );
}

fn print_search_result_pretty(result: &SearchResult, index: usize, no_color: bool) {
    print!("{}", format_search_result_pretty(result, index, no_color));
}

fn print_page_content_pretty(content: &PageContent, no_color: bool) {
    print!("{}", format_page_content_pretty(content, no_color));
}

fn print_crawl_result_pretty(result: &CrawlResult, no_color: bool) {
    if no_color {
        println!(
            "\nCrawl complete: {} pages, {} errors",
            result.summary.fetched, result.summary.failed
        );
        for page in &result.pages {
            println!("\n--- {} ---", page.url);
            println!("{}", teaser(&page.markdown, 200));
        }
    } else {
        print_section(&format!(
            "Crawl complete: {} pages, {} errors",
            result.summary.fetched.to_string().green(),
            result.summary.failed.to_string().red()
        ));
        for page in &result.pages {
            println!("\n{} {}", "→".bright_black(), page.url.bright_blue());
            println!("  {}", page.title.white().bold());
            println!("  {}...", teaser(&page.markdown, 150));
        }
    }
}

async fn run_search(
    query: String,
    opts: SearchOptions,
    format: OutputFormat,
    no_color: bool,
    globals: &GlobalArgs,
) -> DaedraResult<()> {
    let args = SearchArgs {
        query: query.clone(),
        options: Some(opts.clone()),
    };

    let cache =
        daedra::cache::SearchCache::new(build_cache_config(globals.no_cache, globals.cache_ttl));
    if let Some(cached) = cache
        .get_search(&query, &opts.region, &format!("{:?}", opts.safe_search))
        .await
    {
        return print_search(&query, cached, format, no_color);
    }

    let provider = daedra::tools::SearchProvider::auto();
    let response = provider.search(&args).await?;
    cache
        .set_search(
            &query,
            &opts.region,
            &format!("{:?}", opts.safe_search),
            response.clone(),
        )
        .await;

    print_search(&query, response, format, no_color)
}

/// Print a search response in the requested format (shared by cache hits
/// and fresh runs).
fn print_search(
    query: &str,
    response: daedra::types::SearchResponse,
    format: OutputFormat,
    no_color: bool,
) -> DaedraResult<()> {
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&response)?),
        OutputFormat::JsonCompact => println!("{}", serde_json::to_string(&response)?),
        OutputFormat::Pretty => {
            print_search_header_pretty(
                query,
                response.data.len(),
                &response.metadata.search_context.region,
                no_color,
            );
            for (i, result) in response.data.iter().enumerate() {
                print_search_result_pretty(result, i, no_color);
            }
        },
    }

    Ok(())
}

async fn run_fetch(
    url: String,
    selector: Option<String>,
    include_images: bool,
    timeout_secs: u64,
    format: OutputFormat,
    no_color: bool,
    globals: &GlobalArgs,
) -> DaedraResult<()> {
    let args = VisitPageArgs {
        url: url.clone(),
        selector: selector.clone(),
        include_images,
    };

    let cache =
        daedra::cache::SearchCache::new(build_cache_config(globals.no_cache, globals.cache_ttl));
    if let Some(cached) = cache.get_page(&url, selector.as_deref()).await {
        return print_page(&cached, format, no_color);
    }

    // Honor the flag by building a client with the requested timeout; the
    // default path uses the shared client.
    let content = if timeout_secs == 30 {
        fetch::fetch_page(&args).await?
    } else {
        fetch::FetchClient::with_timeout(Duration::from_secs(timeout_secs))?
            .fetch(&args)
            .await?
    };
    cache
        .set_page(&url, selector.as_deref(), content.clone())
        .await;

    print_page(&content, format, no_color)
}

/// Print fetched page content in the requested format.
fn print_page(
    content: &daedra::types::PageContent,
    format: OutputFormat,
    no_color: bool,
) -> DaedraResult<()> {
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&content)?),
        OutputFormat::JsonCompact => println!("{}", serde_json::to_string(&content)?),
        OutputFormat::Pretty => print_page_content_pretty(content, no_color),
    }

    Ok(())
}

async fn run_crawl(
    args: CrawlArgs,
    engine: CrawlEngineOption,
    format: OutputFormat,
    no_color: bool,
) -> DaedraResult<()> {
    let result = match engine {
        CrawlEngineOption::Native => crawl_site(args).await?,
        CrawlEngineOption::Crawlberg => {
            #[cfg(feature = "crawlberg")]
            {
                crawl_site_with_crawlberg(args).await?
            }
            #[cfg(not(feature = "crawlberg"))]
            {
                let _ = args;
                return Err(DaedraError::InvalidArguments(
                    "this binary was built without the crawlberg feature; rebuild with \
                     --features crawlberg"
                        .to_string(),
                ));
            }
        },
    };

    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
        OutputFormat::JsonCompact => println!("{}", serde_json::to_string(&result)?),
        OutputFormat::Pretty => print_crawl_result_pretty(&result, no_color),
    }

    // A silent empty success is the worst crawl failure mode: the summary
    // printed above says zero, and the exit code must agree.
    if result.summary.fetched == 0 {
        return Err(DaedraError::FetchError(
            "crawl fetched 0 pages (see summary above); the site may block crawlers \
             or serve no sitemap and same-origin links"
                .to_string(),
        ));
    }

    Ok(())
}

/// Char-boundary-safe teaser: first `n` characters of `s`, never splitting
/// a multi-byte character.
fn teaser(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// The four MCP tools, with the same names and descriptions the server
/// advertises over `tools/list`. `info` must never drift from this.
const MCP_TOOLS: [(&str, &str); 4] = [
    (
        "web_search",
        "Search the web across all backends with fallback",
    ),
    (
        "search_duckduckgo",
        "Search via the DuckDuckGo HTML backend",
    ),
    ("visit_page", "Fetch and extract webpage content"),
    ("crawl_site", "Crawl a site via sitemap or link discovery"),
];

fn run_info(no_color: bool, format: OutputFormat) {
    if format == OutputFormat::Json || format == OutputFormat::JsonCompact {
        let info = serde_json::json!({
            "name": SERVER_NAME,
            "version": VERSION,
            "author": "DIRMACS Global Services",
            "repository": "https://github.com/dirmacs/daedra",
            "tools": MCP_TOOLS.iter().map(|(name, desc)| serde_json::json!({
                "name": name,
                "description": desc,
            })).collect::<Vec<_>>(),
            "transports": ["stdio", "sse"],
        });
        if format == OutputFormat::Json {
            println!("{}", serde_json::to_string_pretty(&info).unwrap());
        } else {
            println!("{}", serde_json::to_string(&info).unwrap());
        }
        return;
    }

    if no_color {
        println!("\nDaedra Server Information");
        println!("{}", "=".repeat(50));
        println!("  Name: {}", SERVER_NAME);
        println!("  Version: {}", VERSION);
        println!("  Author: DIRMACS Global Services");
        println!("  Repository: https://github.com/dirmacs/daedra");
        println!();
        println!("Available Tools:");
        for (name, desc) in MCP_TOOLS {
            println!("  - {name}: {desc}");
        }
        println!();
        println!("Supported Transports:");
        println!("  - stdio: Standard I/O for MCP clients");
        println!("  - sse: Server-Sent Events over HTTP");
    } else {
        print_banner();

        print_section("Server Information");
        print_info("Name", SERVER_NAME);
        print_info("Version", VERSION);
        print_info("Author", "DIRMACS Global Services");
        print_info("Repository", "https://github.com/dirmacs/daedra");

        print_section("Available Tools");
        for (name, desc) in MCP_TOOLS {
            println!("  {} {}", name.green(), format!("- {desc}").bright_black());
        }

        print_section("Supported Transports");
        println!(
            "  {} {}",
            "stdio".cyan(),
            "- Standard I/O for MCP clients".bright_black()
        );
        println!(
            "  {} {}",
            "sse".cyan(),
            "- Server-Sent Events over HTTP".bright_black()
        );
    }
}

async fn run_check(no_color: bool, format: OutputFormat) -> DaedraResult<()> {
    let reporter = CheckReporter::new(no_color);

    reporter.section("Configuration Check");

    let mut all_ok = check_search_client(&reporter);
    all_ok &= check_fetch_client(&reporter);

    reporter.section("API Keys");
    check_api_keys(&reporter);

    reporter.section("Connectivity Test");
    all_ok &= check_search_connectivity(&reporter).await;

    reporter.entry(
        "summary",
        all_ok,
        if all_ok {
            "all checks passed"
        } else {
            "checks failed"
        },
    );

    if format == OutputFormat::Json || format == OutputFormat::JsonCompact {
        let report = reporter.to_json(all_ok);
        if format == OutputFormat::Json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!("{}", serde_json::to_string(&report)?);
        }
        if !all_ok {
            std::process::exit(1);
        }
        return Ok(());
    }

    reporter.summary(all_ok);
    Ok(())
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if cli.no_color {
        colored::control::set_override(false);
    }

    // Every command logs to stderr; only serve-sse writes elsewhere and only
    // stdio serve must keep stdout pristine for the JSON-RPC stream.
    let use_stderr = match &cli.command {
        Commands::Serve { transport, .. } => matches!(transport, TransportOption::Stdio),
        _ => true,
    };
    setup_logging(cli.verbose, use_stderr, cli.quiet);

    let result = cli
        .command
        .run(
            cli.format,
            cli.verbose,
            cli.quiet,
            cli.no_color,
            &cli.globals,
        )
        .await;

    if let Err(e) = result {
        if cli.no_color {
            eprintln!("Error: {}", e);
        } else {
            print_error(&e.to_string());
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    fn test_globals() -> GlobalArgs {
        GlobalArgs {
            no_cache: true,
            cache_ttl: 300,
            config: None,
        }
    }

    use super::*;
    use daedra::types::{ContentType, PageLink, ResultMetadata};

    #[test]
    fn test_teaser_is_char_boundary_safe() {
        // "héllo" — byte 2 would split the é (2 bytes). 150-byte slicing
        // panicked on such pages; teaser must not.
        let s = "héllo wörld — ünïcode téxt";
        let t = teaser(s, 3);
        assert!(s.starts_with(t));
        assert_eq!(t, "hél");
        assert_eq!(teaser("short", 150), "short");
    }

    #[test]
    fn test_should_print_banner_verbose_sse() {
        assert!(should_print_banner(
            true,
            false,
            OutputFormat::Pretty,
            TransportOption::Sse,
        ));
    }

    #[test]
    fn test_should_print_banner_quiet() {
        assert!(!should_print_banner(
            true,
            true,
            OutputFormat::Pretty,
            TransportOption::Sse,
        ));
    }

    #[test]
    fn test_should_print_banner_stdio() {
        assert!(!should_print_banner(
            true,
            false,
            OutputFormat::Pretty,
            TransportOption::Stdio,
        ));
    }

    #[test]
    fn test_should_print_banner_json_format() {
        assert!(!should_print_banner(
            true,
            false,
            OutputFormat::Json,
            TransportOption::Sse,
        ));
    }

    #[test]
    fn test_check_reporter_section_output() {
        assert_eq!(
            check_section_message("Configuration Check"),
            "
Checking Daedra configuration..."
        );
        assert_eq!(
            check_section_message("Connectivity Test"),
            "
Testing search functionality..."
        );
        assert_eq!(check_section_message("Custom"), "Custom");
    }

    #[test]
    fn test_check_reporter_summary_output() {
        assert_eq!(check_summary_message(true, true), "All checks passed!");
        assert_eq!(
            check_summary_message(false, true),
            "Some checks failed. See above for details."
        );
        assert!(check_summary_message(true, false).contains("All checks passed"));
        assert!(check_summary_message(false, false).contains("failed"));
    }

    #[test]
    fn test_safe_search_from_u8() {
        assert_eq!(safe_search_from_u8(0), Some(SafeSearchLevel::Off));
        assert_eq!(safe_search_from_u8(1), Some(SafeSearchLevel::Moderate));
        assert_eq!(safe_search_from_u8(2), Some(SafeSearchLevel::Strict));
        assert_eq!(safe_search_from_u8(3), None);
    }

    #[test]
    fn test_build_cache_config_disabled() {
        let config = build_cache_config(true, 300);
        assert!(!config.enabled);
    }

    #[test]
    fn test_build_cache_config_enabled() {
        let config = build_cache_config(false, 120);
        assert!(config.enabled);
        assert_eq!(config.ttl, Duration::from_secs(120));
    }

    #[test]
    fn test_parse_host_octets_valid() {
        assert_eq!(parse_host_octets("127.0.0.1").unwrap(), [127, 0, 0, 1]);
    }

    #[test]
    fn test_parse_host_octets_invalid() {
        assert!(parse_host_octets("not-a-real-host-daedra-invalid").is_err());
    }

    #[test]
    fn test_parse_host_octets_localhost() {
        assert_eq!(parse_host_octets("localhost").unwrap(), [127, 0, 0, 1]);
    }

    fn sample_page_content() -> PageContent {
        PageContent {
            url: "https://example.com/page".to_string(),
            title: "Example Page Title".to_string(),
            content: "Page body text.".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            word_count: 3,
            content_type: Some("html".to_string()),
            links: Some(vec![PageLink {
                text: "Other".to_string(),
                url: "https://example.com/other".to_string(),
            }]),
        }
    }

    fn sample_search_result() -> SearchResult {
        SearchResult {
            title: "Example Result".to_string(),
            url: "https://example.com".to_string(),
            description: "A short description.".to_string(),
            metadata: ResultMetadata {
                content_type: ContentType::Article,
                source: "example.com".to_string(),
                favicon: None,
                published_date: None,
            },
        }
    }

    #[test]
    fn test_print_page_content_pretty_no_color() {
        let content = sample_page_content();
        let output = format_page_content_pretty(&content, true);
        assert!(output.contains("Example Page Title"));
        assert!(output.contains("URL: https://example.com/page"));
        assert!(output.contains("Page body text."));
        assert!(output.contains("Links found (1):"));
    }

    #[test]
    fn test_print_page_content_pretty_with_color() {
        let content = sample_page_content();
        let output = format_page_content_pretty(&content, false);
        assert!(output.contains("Example Page Title"));
        assert!(output.contains("https://example.com/page"));
    }

    #[test]
    fn test_print_search_header_pretty_no_color() {
        let output = format_search_header_pretty("rust lang", 5, "wt-wt", true);
        assert!(output.contains("Search Results for: rust lang"));
        assert!(output.contains("Found 5 results in region 'wt-wt'"));
    }

    #[test]
    fn test_print_search_result_pretty_no_color() {
        let result = sample_search_result();
        let output = format_search_result_pretty(&result, 0, true);
        assert!(output.contains("Example Result"));
        assert!(output.contains("URL: https://example.com"));
        assert!(output.contains("A short description."));
    }

    #[test]
    fn test_print_search_result_pretty_with_color() {
        let result = sample_search_result();
        let output = format_search_result_pretty(&result, 0, false);
        assert!(output.contains("Example Result"));
        assert!(output.contains("https://example.com"));
    }

    #[tokio::test]
    async fn test_commands_info() {
        let result = Commands::Info
            .run(OutputFormat::Pretty, false, true, true, &test_globals())
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn test_commands_search_default() {
        let result = Commands::Search {
            query: "rust programming".to_string(),
            num_results: 1,
            region: "wt-wt".to_string(),
            safe_search: SafeSearchOption::default(),
            time_range: None,
            backends: Vec::new(),
            exclude_backends: Vec::new(),
        }
        .run(OutputFormat::Pretty, false, true, true, &test_globals())
        .await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_search_options_cli_mapping() {
        let backends = vec!["hn".to_string()];
        let opts = SearchOptions {
            region: "wt-wt".to_string(),
            safe_search: SafeSearchLevel::Moderate,
            num_results: 10,
            time_range: None,
            backends: (!backends.is_empty()).then_some(backends),
            exclude_backends: None,
        };
        assert_eq!(
            opts.backends.as_deref(),
            Some(["hn".to_string()].as_slice())
        );
        assert!(opts.exclude_backends.is_none());
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn test_commands_check() {
        let result = Commands::Check
            .run(OutputFormat::Pretty, false, true, true, &test_globals())
            .await;
        assert!(result.is_ok());
    }
}
