//! Human-readable output formatting.

use colored::Colorize;
use daedra::types::{
        CrawlResult, PageContent, SearchResult,
    };

use crate::cli::commands::teaser;
use crate::cli::logging::{format_info, format_section, print_section};



pub(super) fn format_page_header(title: &str, no_color: bool) -> String {
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


pub(super) fn format_search_header_pretty(query: &str, count: usize, region: &str, no_color: bool) -> String {
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


pub(super) fn format_search_result_pretty(result: &SearchResult, index: usize, no_color: bool) -> String {
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


pub(super) fn format_page_content_pretty(content: &PageContent, no_color: bool) -> String {
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


pub(super) fn print_search_header_pretty(query: &str, count: usize, region: &str, no_color: bool) {
    print!(
        "{}",
        format_search_header_pretty(query, count, region, no_color)
    );
}


pub(super) fn print_search_result_pretty(result: &SearchResult, index: usize, no_color: bool) {
    print!("{}", format_search_result_pretty(result, index, no_color));
}


pub(super) fn print_page_content_pretty(content: &PageContent, no_color: bool) {
    print!("{}", format_page_content_pretty(content, no_color));
}


pub(super) fn print_crawl_result_pretty(result: &CrawlResult, no_color: bool) {
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
