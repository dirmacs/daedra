//! Daedra CLI - Web Search and Research MCP Server
//!
//! The body lives in [`cli`].

use clap::Parser;
use cli::args::{Cli, Commands, TransportOption};
use cli::logging::{print_error, setup_logging};

mod cli;


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
