//! Daedra CLI - Web Search and Research MCP Server
//!
//! A command-line interface for the Daedra MCP server.

use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use daedra::{
    DaedraResult, SERVER_NAME, VERSION,
    cache::CacheConfig,
    server::{DaedraServer, ServerConfig, TransportType},
    tools::{fetch, search},
    types::{SafeSearchLevel, SearchArgs, SearchOptions, VisitPageArgs},
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
    println!("\n{}", title.yellow().bold());
    println!("{}", "─".repeat(40).bright_black());
}

async fn run_serve(
    transport: TransportOption,
    port: u16,
    host: String,
    no_cache: bool,
    cache_ttl: u64,
) -> DaedraResult<()> {
    let cache_config = if no_cache {
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
    };

    let config = ServerConfig {
        cache: cache_config,
        verbose: false,
        ..Default::default()
    };

    let server = DaedraServer::new(config)?;

    let transport_type = match transport {
        TransportOption::Stdio => TransportType::Stdio,
        TransportOption::Sse => {
            let host_parts: Vec<u8> = host.split('.').filter_map(|s| s.parse().ok()).collect();

            if host_parts.len() != 4 {
                return Err(daedra::types::DaedraError::InvalidArguments(
                    "Invalid host format".to_string(),
                ));
            }

            TransportType::Sse {
                port,
                host: [host_parts[0], host_parts[1], host_parts[2], host_parts[3]],
            }
        },
    };

    server.run(transport_type).await
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

    let response = search::perform_search(&args).await?;

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&response)?);
        },
        OutputFormat::JsonCompact => {
            println!("{}", serde_json::to_string(&response)?);
        },
        OutputFormat::Pretty => {
            if no_color {
                println!("\nSearch Results for: {}", query);
                println!("{}", "=".repeat(50));
                println!(
                    "Found {} results in region '{}'",
                    response.data.len(),
                    response.metadata.search_context.region
                );
                println!();

                for (i, result) in response.data.iter().enumerate() {
                    println!("{}. {}", i + 1, result.title);
                    println!("   URL: {}", result.url);
                    println!("   {}", result.description);
                    println!(
                        "   Source: {} | Type: {:?}",
                        result.metadata.source, result.metadata.content_type
                    );
                    println!();
                }
            } else {
                print_section(&format!("Search Results for: {}", query.cyan()));
                println!(
                    "Found {} results in region '{}'",
                    response.data.len().to_string().green(),
                    response.metadata.search_context.region.bright_blue()
                );
                println!();

                for (i, result) in response.data.iter().enumerate() {
                    println!(
                        "{} {}",
                        format!("{}.", i + 1).bright_black(),
                        result.title.white().bold()
                    );
                    println!(
                        "   {} {}",
                        "URL:".bright_black(),
                        result.url.bright_blue().underline()
                    );
                    println!("   {}", result.description.bright_white());
                    println!(
                        "   {} {} {} {:?}",
                        "Source:".bright_black(),
                        result.metadata.source.yellow(),
                        "|".bright_black(),
                        result.metadata.content_type
                    );
                    println!();
                }
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
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&content)?);
        },
        OutputFormat::JsonCompact => {
            println!("{}", serde_json::to_string(&content)?);
        },
        OutputFormat::Pretty => {
            if no_color {
                println!("\n{}", content.title);
                println!("{}", "=".repeat(50));
                println!("URL: {}", content.url);
                println!("Fetched: {}", content.timestamp);
                println!("Words: {}", content.word_count);
                println!();
                println!("{}", content.content);

                if let Some(links) = content.links {
                    println!("\nLinks found ({}):", links.len());
                    for link in links.iter().take(10) {
                        println!("  - {} ({})", link.text, link.url);
                    }
                }
            } else {
                print_section(&content.title.white().bold().to_string());
                print_info("URL", &content.url.bright_blue().underline().to_string());
                print_info("Fetched", &content.timestamp);
                print_info("Words", &content.word_count.to_string().green().to_string());
                println!();
                println!("{}", content.content);

                if let Some(links) = content.links {
                    print_section(&format!("Links found ({})", links.len()));
                    for link in links.iter().take(10) {
                        println!(
                            "  {} {} {}",
                            "→".bright_black(),
                            link.text.white(),
                            format!("({})", link.url).bright_blue()
                        );
                    }
                }
            }
        },
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
    if no_color {
        println!("\nChecking Daedra configuration...");
    } else {
        print_section("Configuration Check");
    }

    // Check if we can create clients
    let search_result = search::SearchClient::new();
    let fetch_result = fetch::FetchClient::new();

    let mut all_ok = true;

    match search_result {
        Ok(_) => {
            if no_color {
                println!("  [OK] Search client initialized");
            } else {
                print_success("Search client initialized");
            }
        },
        Err(e) => {
            if no_color {
                println!("  [FAIL] Search client: {}", e);
            } else {
                print_error(&format!("Search client: {}", e));
            }
            all_ok = false;
        },
    }

    match fetch_result {
        Ok(_) => {
            if no_color {
                println!("  [OK] Fetch client initialized");
            } else {
                print_success("Fetch client initialized");
            }
        },
        Err(e) => {
            if no_color {
                println!("  [FAIL] Fetch client: {}", e);
            } else {
                print_error(&format!("Fetch client: {}", e));
            }
            all_ok = false;
        },
    }

    // Test a simple search
    if no_color {
        println!("\nTesting search functionality...");
    } else {
        print_section("Connectivity Test");
    }

    let test_args = SearchArgs {
        query: "test".to_string(),
        options: Some(SearchOptions {
            num_results: 1,
            ..Default::default()
        }),
    };

    match search::perform_search(&test_args).await {
        Ok(response) => {
            if response.data.is_empty() {
                if no_color {
                    println!("  [WARN] Search returned no results");
                } else {
                    println!(
                        "  {} {}",
                        "⚠".yellow(),
                        "Search returned no results".yellow()
                    );
                }
            } else if no_color {
                println!("  [OK] Search connectivity verified");
            } else {
                print_success("Search connectivity verified");
            }
        },
        Err(e) => {
            if no_color {
                println!("  [FAIL] Search test: {}", e);
            } else {
                print_error(&format!("Search test: {}", e));
            }
            all_ok = false;
        },
    }

    println!();

    if all_ok {
        if no_color {
            println!("All checks passed!");
        } else {
            println!("{}", "✓ All checks passed!".green().bold());
        }
    } else {
        if no_color {
            println!("Some checks failed. See above for details.");
        } else {
            println!(
                "{}",
                "✗ Some checks failed. See above for details.".red().bold()
            );
        }
        std::process::exit(1);
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Handle color settings
    if cli.no_color {
        colored::control::set_override(false);
    }

    // Set up logging for serve command only
    // For stdio transport, logs MUST go to stderr to avoid corrupting the JSON-RPC stream
    if let Commands::Serve { transport, .. } = &cli.command {
        let use_stderr = matches!(transport, TransportOption::Stdio);
        setup_logging(cli.verbose, use_stderr, cli.quiet);
    }

    let result = match cli.command {
        Commands::Serve {
            transport,
            port,
            host,
            no_cache,
            cache_ttl,
        } => {
            // Only show banner for SSE transport (not stdio) and when verbose and not quiet
            if cli.verbose
                && !cli.quiet
                && !matches!(cli.format, OutputFormat::Json | OutputFormat::JsonCompact)
                && matches!(transport, TransportOption::Sse)
            {
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
                cli.format,
                cli.no_color,
            )
            .await
        },

        Commands::Fetch {
            url,
            selector,
            include_images,
        } => run_fetch(url, selector, include_images, cli.format, cli.no_color).await,

        Commands::Info => {
            run_info(cli.no_color);
            Ok(())
        },

        Commands::Check => run_check(cli.no_color).await,
    };

    if let Err(e) = result {
        if cli.no_color {
            eprintln!("Error: {}", e);
        } else {
            print_error(&e.to_string());
        }
        std::process::exit(1);
    }
}
