//! Benchmarks for Daedra operations

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use daedra::{
    cache::SearchCache,
    types::{ContentType, ResultMetadata, SearchOptions, SearchResponse, SearchResult},
};

fn create_test_response(result_count: usize) -> SearchResponse {
    let results: Vec<SearchResult> = (0..result_count)
        .map(|i| SearchResult {
            title: format!("Test Result {}", i),
            url: format!("https://example{}.com/page", i),
            description: format!(
                "This is test result number {} with some description text",
                i
            ),
            metadata: ResultMetadata {
                content_type: ContentType::Article,
                source: format!("example{}.com", i),
                favicon: None,
                published_date: None,
            },
        })
        .collect();

    let options = SearchOptions::default();
    SearchResponse::new("test query".to_string(), results, &options)
}

fn bench_cache_operations(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("cache_operations");

    for size in [1, 10, 50].iter() {
        group.bench_with_input(BenchmarkId::new("cache_set", size), size, |b, &size| {
            let cache = SearchCache::with_defaults();
            let response = create_test_response(size);

            b.to_async(&runtime).iter(|| async {
                cache
                    .set_search(
                        black_box("test query"),
                        black_box("wt-wt"),
                        black_box("MODERATE"),
                        black_box(response.clone()),
                    )
                    .await;
            });
        });

        group.bench_with_input(BenchmarkId::new("cache_get", size), size, |b, &size| {
            let cache = SearchCache::with_defaults();
            let response = create_test_response(size);

            runtime.block_on(async {
                cache
                    .set_search("test query", "wt-wt", "MODERATE", response)
                    .await;
            });

            b.to_async(&runtime).iter(|| async {
                cache
                    .get_search(
                        black_box("test query"),
                        black_box("wt-wt"),
                        black_box("MODERATE"),
                    )
                    .await
            });
        });
    }

    group.finish();
}

fn bench_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("serialization");

    for size in [1, 10, 50].iter() {
        let response = create_test_response(*size);

        group.bench_with_input(
            BenchmarkId::new("json_serialize", size),
            &response,
            |b, response| {
                b.iter(|| serde_json::to_string(black_box(response)).unwrap());
            },
        );

        let json = serde_json::to_string(&response).unwrap();

        group.bench_with_input(
            BenchmarkId::new("json_deserialize", size),
            &json,
            |b, json| {
                b.iter(|| serde_json::from_str::<SearchResponse>(black_box(json)).unwrap());
            },
        );
    }

    group.finish();
}

fn bench_response_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("response_creation");

    for size in [1, 10, 50].iter() {
        let results: Vec<SearchResult> = (0..*size)
            .map(|i| SearchResult {
                title: format!("Test Result {}", i),
                url: format!("https://example{}.com/page", i),
                description: format!("Description {}", i),
                metadata: ResultMetadata {
                    content_type: ContentType::Article,
                    source: format!("example{}.com", i),
                    favicon: None,
                    published_date: None,
                },
            })
            .collect();

        let options = SearchOptions::default();

        group.bench_with_input(
            BenchmarkId::new("create_search_response", size),
            &(results.clone(), options.clone()),
            |b, (results, options)| {
                b.iter(|| {
                    SearchResponse::new(
                        black_box("test query".to_string()),
                        black_box(results.clone()),
                        black_box(options),
                    )
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_cache_operations,
    bench_serialization,
    bench_response_creation
);
criterion_main!(benches);
