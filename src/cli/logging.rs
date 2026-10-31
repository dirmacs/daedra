//! Logging setup and terminal presentation helpers.

use colored::Colorize;
use daedra::VERSION;
use tracing_subscriber::{EnvFilter, fmt};



/// Set up logging with configurable output destination
///
/// # Arguments
/// * `verbose` - Enable debug-level logging
/// * `use_stderr` - Write logs to stderr instead of stdout (required for stdio transport)
/// * `quiet` - Disable all logging output
pub fn setup_logging(verbose: bool, use_stderr: bool, quiet: bool) {
    // If quiet mode, use a very restrictive filter that effectively disables logging
    // Default to warnings only: the per-backend progress chatter is useful
    // under --verbose and pure noise in normal runs, where results go to
    // stdout and logs to stderr.
    let filter = if quiet {
        EnvFilter::new("off")
    } else if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("warn")
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


pub(super) fn print_banner() {
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


pub(super) fn print_success(message: &str) {
    println!("{} {}", "✓".green().bold(), message);
}


pub fn print_error(message: &str) {
    eprintln!("{} {}", "✗".red().bold(), message);
}


pub(super) fn print_info(label: &str, value: &str) {
    println!("  {} {}", format!("{}:", label).bright_blue(), value);
}


pub(super) fn print_section(title: &str) {
    println!("{}", format_section(title));
}


pub(super) fn format_section(title: &str) -> String {
    format!(
        "\n{}\n{}",
        title.yellow().bold(),
        "─".repeat(40).bright_black()
    )
}


pub(super) fn format_info(label: &str, value: &str) -> String {
    format!("  {} {}\n", format!("{}:", label).bright_blue(), value)
}
