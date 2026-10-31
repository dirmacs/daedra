//! `daedra check`: configuration, key, and connectivity diagnostics.

use colored::Colorize;
use daedra::{
    DaedraResult,
    tools::{fetch, search},
    types::{SearchArgs, SearchOptions},
};

use crate::cli::args::{OutputFormat, TransportOption};
use crate::cli::logging::*;

pub(crate) fn check_section_message(title: &str) -> String {
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

pub(crate) fn check_summary_message(all_ok: bool, no_color: bool) -> String {
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

pub(crate) fn should_print_banner(
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

pub(crate) struct CheckReporter {
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

pub(crate) fn check_search_client(reporter: &CheckReporter) -> bool {
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

/// Report which optional quota tokens are set. A missing token is not a
/// failure — search still runs on public endpoints.
pub(crate) fn check_api_keys(reporter: &CheckReporter) {
    let keys = [
        ("GITHUB_TOKEN", "GitHub (higher rate limits)"),
        ("MARGINALIA_API_KEY", "Marginalia (higher rate limits)"),
    ];
    let mut any_set = false;
    for (name, what) in keys {
        if std::env::var(name).is_ok_and(|v| !v.trim().is_empty()) {
            reporter.ok(&format!("{name} set ({what})"));
            any_set = true;
        } else {
            reporter.warn(&format!("{name} not set — {what} use the public pool"));
        }
    }
    if !any_set {
        reporter.warn(
            "No quota tokens set. Search uses public endpoints. GitHub uses the unauthenticated pool.",
        );
    }
    reporter.entry(
        "api_keys",
        true,
        if any_set {
            "at least one quota token set"
        } else {
            "no quota tokens set"
        },
    );
}

pub(crate) fn check_fetch_client(reporter: &CheckReporter) -> bool {
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

pub(super) async fn check_search_connectivity(reporter: &CheckReporter) -> bool {
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
                // A successful call that yields nothing is NOT a passing check.
                // The previous version logged a warning and then still recorded
                // `ok: true`, so `check` printed "All checks passed!" on a host
                // where wikipedia answered zero and brave/bing/google/duckduckgo
                // were all bot-blocked. A gate that reports success without
                // producing a result is the failure mode this repo's own
                // directives call a false green.
                reporter.fail("Search ran but returned 0 results");
                reporter.entry("connectivity", false, "search ran, 0 results");
                false
            } else {
                reporter.ok(&format!(
                    "Search connectivity verified ({} result(s))",
                    response.data.len()
                ));
                reporter.entry("connectivity", true, "search verified");
                true
            }
        },
        Err(e) => {
            reporter.fail(&format!("Search test: {e}"));
            reporter.entry("connectivity", false, &format!("Search test: {e}"));
            false
        },
    }
}

pub(super) async fn run_check(no_color: bool, format: OutputFormat) -> DaedraResult<()> {
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
