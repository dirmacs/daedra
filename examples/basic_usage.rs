//! Basic usage example for Daedra
//!
//! Run with: cargo run --example basic_usage

use daedra::tools::{fetch, search};
use daedra::types::{SafeSearchLevel, SearchArgs, SearchOptions, VisitPageArgs};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("🔍 Daedra Basic Usage Example\n");

    // Example 1: Basic search
    println!("=== Example 1: Basic Search ===\n");

    let search_args = SearchArgs {
        query: "Rust programming language".to_string(),
        options: Some(SearchOptions {
            num_results: 5,
            region: "wt-wt".to_string(),
            safe_search: SafeSearchLevel::Moderate,
            time_range: None,
        }),
    };

    match search::perform_search(&search_args).await {
        Ok(response) => {
            println!(
                "Found {} results for '{}'\n",
                response.data.len(),
                search_args.query
            );

            for (i, result) in response.data.iter().enumerate() {
                println!("{}. {}", i + 1, result.title);
                println!("   URL: {}", result.url);
                println!("   Type: {:?}", result.metadata.content_type);
                println!("   {}\n", result.description);
            }
        },
        Err(e) => {
            eprintln!("Search failed: {}", e);
        },
    }

    // Example 2: Search with time filter
    println!("\n=== Example 2: Search with Time Filter ===\n");

    let recent_search = SearchArgs {
        query: "rust async".to_string(),
        options: Some(SearchOptions {
            num_results: 3,
            region: "us-en".to_string(),
            safe_search: SafeSearchLevel::Moderate,
            time_range: Some("m".to_string()), // Last month
        }),
    };

    match search::perform_search(&recent_search).await {
        Ok(response) => {
            println!("Found {} recent results\n", response.data.len());
            println!("Query analysis:");
            println!("  Language: {}", response.metadata.query_analysis.language);
            println!("  Topics: {:?}", response.metadata.query_analysis.topics);
        },
        Err(e) => {
            eprintln!("Search failed: {}", e);
        },
    }

    // Example 3: Fetch a webpage
    println!("\n=== Example 3: Fetch Webpage ===\n");

    let fetch_args = VisitPageArgs {
        url: "https://www.rust-lang.org".to_string(),
        selector: None,
        include_images: false,
    };

    match fetch::fetch_page(&fetch_args).await {
        Ok(content) => {
            println!("Page: {}", content.title);
            println!("URL: {}", content.url);
            println!("Word count: {}", content.word_count);
            println!("Fetched at: {}", content.timestamp);
            println!("\nContent preview (first 500 chars):");
            println!("{}", &content.content.chars().take(500).collect::<String>());

            if let Some(links) = &content.links {
                println!("\nFound {} links", links.len());
            }
        },
        Err(e) => {
            eprintln!("Fetch failed: {}", e);
        },
    }

    // Example 4: Fetch with selector
    println!("\n=== Example 4: Fetch with CSS Selector ===\n");

    let selective_fetch = VisitPageArgs {
        url: "https://example.com".to_string(),
        selector: Some("p".to_string()),
        include_images: false,
    };

    match fetch::fetch_page(&selective_fetch).await {
        Ok(content) => {
            println!("Selected content from {}", content.url);
            println!("Content: {}", content.content);
        },
        Err(e) => {
            eprintln!("Fetch failed: {}", e);
        },
    }

    // Example 5: Parallel searches
    println!("\n=== Example 5: Parallel Searches ===\n");

    let queries = vec![
        SearchArgs {
            query: "tokio async runtime".to_string(),
            options: Some(SearchOptions {
                num_results: 2,
                ..Default::default()
            }),
        },
        SearchArgs {
            query: "serde serialization".to_string(),
            options: Some(SearchOptions {
                num_results: 2,
                ..Default::default()
            }),
        },
        SearchArgs {
            query: "reqwest http client".to_string(),
            options: Some(SearchOptions {
                num_results: 2,
                ..Default::default()
            }),
        },
    ];

    let results = search::perform_parallel_searches(queries).await;

    for (i, result) in results.iter().enumerate() {
        match result {
            Ok(response) => {
                println!(
                    "Query {}: '{}' - {} results",
                    i + 1,
                    response.metadata.query,
                    response.data.len()
                );
            },
            Err(e) => {
                println!("Query {} failed: {}", i + 1, e);
            },
        }
    }

    println!("\n✅ Examples completed!");
    Ok(())
}
