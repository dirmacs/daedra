//! Daedra CLI - Web Search and Research MCP Server
//!
//! A command-line interface for the Daedra MCP server.

use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
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

    #[command(subcommand)]
    command: Commands,
}

/// Output format options
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum OutputFormat {
    /// Pretty-printed human-readable output
    #[default]
    Pretty,
    /// JSON output
    Json,
    /// Compact JSON output
    JsonCompact,
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

        /// Disable result caching
        #[arg(long)]
        no_cache: bool,

        /// Cache TTL in seconds
        #[arg(long, default_value = "300")]
        cache_ttl: u64,
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
    },

    /// Show server information
    Info,

    /// Validate configuration and dependencies
    Check,
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
Checking Daedra configuration...".to_string(),
        "Connectivity Test" => "
Testing search functionality...".to_string(),
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
    ) -> DaedraResult<()> {
        match self {
            Commands::Serve {
                transport,
                port,
                host,
                no_cache,
                cache_ttl,
            } => {
                if should_print_banner(verbose, quiet, format, transport) {
                    print_banner();
                }
                run_serve(transport, port, host, no_cache, cache_ttl).await
            },

            Commands::Search {
                query,
                num_results,
                region,
                safe_search,
                time_range,
            } => {
                run_search(
                    query,
                    num_results,
                    region,
                    safe_search,
                    time_range,
                    format,
                    no_color,
                )
                .await
            },

            Commands::Fetch {
                url,
                selector,
                include_images,
            } => run_fetch(url, selector, include_images, format, no_color).await,

            Commands::Crawl {
                url,
                max_pages,
                concurrency,
            } => run_crawl(url, max_pages, concurrency, format, no_color).await,

            Commands::Info => {
                run_info(no_color);
                Ok(())
            },

            Commands::Check => run_check(no_color).await,
        }
    }
}

struct CheckReporter {
    no_color: bool,
}

impl CheckReporter {
    fn new(no_color: bool) -> Self {
        Self { no_color }
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
            true
        }
        Err(e) => {
            reporter.fail(&format!("Search client: {e}"));
            false
        }
    }
}

fn check_fetch_client(reporter: &CheckReporter) -> bool {
    match fetch::FetchClient::new() {
        Ok(_) => {
            reporter.ok("Fetch client initialized");
            true
        }
        Err(e) => {
            reporter.fail(&format!("Fetch client: {e}"));
            false
        }
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
            } else {
                reporter.ok("Search connectivity verified");
            }
            true
        }
        Err(e) => {
            reporter.fail(&format!("Search test: {e}"));
            false
        }
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
║   {}   ║
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
    let parts: Vec<u8> = host.split('.').filter_map(|s| s.parse().ok()).collect();
    if parts.len() != 4 {
        return Err(DaedraError::InvalidArguments(
            "Invalid host format".to_string(),
        ));
    }
    Ok([parts[0], parts[1], parts[2], parts[3]])
}

async fn run_serve(
    transport: TransportOption,
    port: u16,
    host: String,
    no_cache: bool,
    cache_ttl: u64,
) -> DaedraResult<()> {
    let config = ServerConfig {
        cache: build_cache_config(no_cache, cache_ttl),
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
    print!("{}", format_search_header_pretty(query, count, region, no_color));
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
            println!("{}", &page.markdown[..page.markdown.len().min(200)]);
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
            println!("  {}...", &page.markdown[..page.markdown.len().min(150)]);
        }
    }
}

async fn run_search(
    query: String,
    num_results: usize,
    region: String,
    safe_search: SafeSearchOption,
    time_range: Option<String>,
    format: OutputFormat,
    no_color: bool,
) -> DaedraResult<()> {
    let args = SearchArgs {
        query: query.clone(),
        options: Some(SearchOptions {
            region,
            safe_search: safe_search.into(),
            num_results,
            time_range,
        }),
    };

    let provider = daedra::tools::SearchProvider::auto();
    let response = provider.search(&args).await?;

    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&response)?),
        OutputFormat::JsonCompact => println!("{}", serde_json::to_string(&response)?),
        OutputFormat::Pretty => {
            print_search_header_pretty(
                &query,
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
    format: OutputFormat,
    no_color: bool,
) -> DaedraResult<()> {
    let args = VisitPageArgs {
        url: url.clone(),
        selector,
        include_images,
    };

    let content = fetch::fetch_page(&args).await?;

    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&content)?),
        OutputFormat::JsonCompact => println!("{}", serde_json::to_string(&content)?),
        OutputFormat::Pretty => print_page_content_pretty(&content, no_color),
    }

    Ok(())
}


async fn run_crawl(
    url: String,
    max_pages: usize,
    concurrency: usize,
    format: OutputFormat,
    no_color: bool,
) -> DaedraResult<()> {
    let args = CrawlArgs {
        root_url: url,
        max_pages,
        concurrency,
    };

    let result = crawl_site(args).await?;

    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
        OutputFormat::JsonCompact => println!("{}", serde_json::to_string(&result)?),
        OutputFormat::Pretty => print_crawl_result_pretty(&result, no_color),
    }

    Ok(())
}


fn run_info(no_color: bool) {
    if no_color {
        println!("\nDaedra Server Information");
        println!("{}", "=".repeat(50));
        println!("  Name: {}", SERVER_NAME);
        println!("  Version: {}", VERSION);
        println!("  Author: DIRMACS Global Services");
        println!("  Repository: https://github.com/dirmacs/daedra");
        println!();
        println!("Available Tools:");
        println!("  - search_duckduckgo: Search the web using DuckDuckGo");
        println!("  - visit_page: Fetch and extract webpage content");
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
        println!(
            "  {} {}",
            "search_duckduckgo".green(),
            "- Search the web using DuckDuckGo".bright_black()
        );
        println!(
            "  {} {}",
            "visit_page".green(),
            "- Fetch and extract webpage content".bright_black()
        );

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

async fn run_check(no_color: bool) -> DaedraResult<()> {
    let reporter = CheckReporter::new(no_color);

    reporter.section("Configuration Check");

    let mut all_ok = check_search_client(&reporter);
    all_ok &= check_fetch_client(&reporter);

    reporter.section("Connectivity Test");
    all_ok &= check_search_connectivity(&reporter).await;

    reporter.summary(all_ok);
    Ok(())
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if cli.no_color {
        colored::control::set_override(false);
    }

    if let Commands::Serve { transport, .. } = &cli.command {
        let use_stderr = matches!(transport, TransportOption::Stdio);
        setup_logging(cli.verbose, use_stderr, cli.quiet);
    }

    let result = cli
        .command
        .run(cli.format, cli.verbose, cli.quiet, cli.no_color)
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
    use super::*;
    use daedra::types::{ContentType, PageLink, ResultMetadata};

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
        assert!(parse_host_octets("127.0.1").is_err());
        assert!(parse_host_octets("not-a-host").is_err());
    }

    fn sample_page_content() -> PageContent {
        PageContent {
            url: "https://example.com/page".to_string(),
            title: "Example Page Title".to_string(),
            content: "Page body text.".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            word_count: 3,
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
            .run(OutputFormat::Pretty, false, true, true)
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
        }
        .run(OutputFormat::Pretty, false, true, true)
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn test_commands_check() {
        let result = Commands::Check
            .run(OutputFormat::Pretty, false, true, true)
            .await;
        assert!(result.is_ok());
    }
}

