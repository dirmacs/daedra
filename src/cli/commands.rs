//! Subcommand dispatch.

use clap::CommandFactory;
#[cfg(feature = "crawlberg")]
use daedra::tools::crawl_site_with_crawlberg;
use colored::Colorize;
use daedra::{
    DaedraResult, SERVER_NAME, VERSION,
    tools::{crawl_site, fetch},
    types::{CrawlArgs, DaedraError, SearchArgs, SearchOptions, VisitPageArgs},
};
use std::time::Duration;

use crate::cli::args::*;
use crate::cli::check::*;
use crate::cli::config::*;
use crate::cli::logging::*;
use crate::cli::pretty::*;
use crate::cli::serve::*;

impl Commands {
    pub async fn run(
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

pub(super) async fn run_search(
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
pub(crate) fn print_search(
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

pub(super) async fn run_fetch(
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
pub(crate) fn print_page(
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

pub(super) async fn run_crawl(
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
pub(crate) fn teaser(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// The four MCP tools, with the same names and descriptions the server
/// advertises over `tools/list`. `info` must never drift from this.
pub(crate) const MCP_TOOLS: [(&str, &str); 4] = [
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

pub(crate) fn run_info(no_color: bool, format: OutputFormat) {
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
