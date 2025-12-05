//! Caching example for Daedra
//!
//! Run with: cargo run --example caching

use daedra::cache::{CacheConfig, SearchCache};
use daedra::tools::search;
use daedra::types::{SearchArgs, SearchOptions};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🔍 Daedra Caching Example\n");

    // Create a cache with custom configuration
    let cache = SearchCache::new(CacheConfig {
        ttl: Duration::from_secs(60), // 1 minute TTL
        max_entries: 100,
        enabled: true,
    });

    let search_args = SearchArgs {
        query: "rust caching".to_string(),
        options: Some(SearchOptions {
            num_results: 5,
            ..Default::default()
        }),
    };

    let options = search_args.options.as_ref().unwrap();

    // First search - cache miss
    println!("=== First Search (Cache Miss) ===\n");
    let start = Instant::now();

    let response = search::perform_search(&search_args).await?;
    let first_duration = start.elapsed();

    println!("Search completed in {:?}", first_duration);
    println!("Results: {}", response.data.len());

    // Store in cache
    cache
        .set_search(
            &search_args.query,
            &options.region,
            &options.safe_search.to_string(),
            response.clone(),
        )
        .await;

    println!("\nCache stats after first search:");
    println!("{}", cache.stats());

    // Second search - cache hit
    println!("\n=== Second Search (Cache Hit) ===\n");
    let start = Instant::now();

    let cached_response = cache
        .get_search(
            &search_args.query,
            &options.region,
            &options.safe_search.to_string(),
        )
        .await;

    let second_duration = start.elapsed();

    match cached_response {
        Some(cached) => {
            println!("Cache HIT! Retrieved in {:?}", second_duration);
            println!("Results: {}", cached.data.len());
            println!(
                "Speedup: {:.1}x faster",
                first_duration.as_secs_f64() / second_duration.as_secs_f64()
            );
        },
        None => {
            println!("Cache MISS (unexpected)");
        },
    }

    // Different query - cache miss
    println!("\n=== Different Query (Cache Miss) ===\n");

    let different_args = SearchArgs {
        query: "rust async".to_string(),
        options: Some(SearchOptions {
            num_results: 3,
            ..Default::default()
        }),
    };

    let different_options = different_args.options.as_ref().unwrap();
    let cached = cache
        .get_search(
            &different_args.query,
            &different_options.region,
            &different_options.safe_search.to_string(),
        )
        .await;

    if cached.is_none() {
        println!("Cache MISS for different query (expected)");
    }

    // Clear cache
    println!("\n=== Clearing Cache ===\n");
    cache.clear().await;
    println!("Cache cleared");
    println!("Cache stats after clear:");
    println!("{}", cache.stats());

    // Verify cache is empty
    let after_clear = cache
        .get_search(
            &search_args.query,
            &options.region,
            &options.safe_search.to_string(),
        )
        .await;

    if after_clear.is_none() {
        println!("Cache is empty (verified)");
    }

    // Demonstrate disabled cache
    println!("\n=== Disabled Cache ===\n");
    let disabled_cache = SearchCache::disabled();

    disabled_cache
        .set_search(
            &search_args.query,
            &options.region,
            &options.safe_search.to_string(),
            response,
        )
        .await;

    let from_disabled = disabled_cache
        .get_search(
            &search_args.query,
            &options.region,
            &options.safe_search.to_string(),
        )
        .await;

    if from_disabled.is_none() {
        println!("Disabled cache correctly returns None");
    }

    println!("\n✅ Caching example completed!");
    Ok(())
}
