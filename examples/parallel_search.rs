//! Opt-in anonymous Parallel search through Daedra's library provider.
//!
//! Run: cargo run --example parallel_search -- "Rust async runtime"

use daedra::tools::{SearchProvider, parallel::ParallelBackend};
use daedra::types::{SafeSearchLevel, SearchArgs, SearchOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let query = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let query = if query.is_empty() {
        "Rust async runtime".to_string()
    } else {
        query
    };
    let provider = SearchProvider::new(vec![Box::new(ParallelBackend::new())]);
    let response = provider
        .search(&SearchArgs {
            query,
            options: Some(SearchOptions {
                num_results: 5,
                safe_search: SafeSearchLevel::Off,
                backends: Some(vec!["parallel".to_string()]),
                ..Default::default()
            }),
        })
        .await?;
    for result in response.data {
        println!("{}\n{}\n{}\n", result.title, result.url, result.description);
    }
    Ok(())
}
