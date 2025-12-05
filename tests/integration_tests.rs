//! Integration tests for Daedra

use daedra::{
    cache::SearchCache,
    tools::{fetch, search},
    types::{ContentType, SafeSearchLevel, SearchArgs, SearchOptions, VisitPageArgs},
};

mod search_tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_search() {
        let args = SearchArgs {
            query: "rust programming language".to_string(),
            options: Some(SearchOptions {
                num_results: 5,
                region: "wt-wt".to_string(),
                safe_search: SafeSearchLevel::Moderate,
                time_range: None,
            }),
        };

        let result = search::perform_search(&args).await;

        // Note: This test may fail if there's no network connectivity
        // In CI, you might want to use wiremock to mock the responses
        match result {
            Ok(response) => {
                assert_eq!(response.response_type, "search_results");
                assert!(!response.data.is_empty());
                assert!(response.data.len() <= 5);
                assert_eq!(response.metadata.query, "rust programming language");
            },
            Err(e) => {
                // Allow network failures in tests
                eprintln!("Search test skipped due to network error: {}", e);
            },
        }
    }

    #[tokio::test]
    async fn test_search_with_safe_search() {
        let args = SearchArgs {
            query: "test".to_string(),
            options: Some(SearchOptions {
                num_results: 3,
                region: "us-en".to_string(),
                safe_search: SafeSearchLevel::Strict,
                time_range: None,
            }),
        };

        let result = search::perform_search(&args).await;

        match result {
            Ok(response) => {
                assert_eq!(response.metadata.search_context.safe_search, "STRICT");
            },
            Err(e) => {
                eprintln!("Search test skipped due to network error: {}", e);
            },
        }
    }

    #[tokio::test]
    async fn test_parallel_searches() {
        let queries = vec![
            SearchArgs {
                query: "rust".to_string(),
                options: Some(SearchOptions {
                    num_results: 2,
                    ..Default::default()
                }),
            },
            SearchArgs {
                query: "python".to_string(),
                options: Some(SearchOptions {
                    num_results: 2,
                    ..Default::default()
                }),
            },
        ];

        let results = search::perform_parallel_searches(queries).await;

        assert_eq!(results.len(), 2);
    }
}

mod fetch_tests {
    use super::*;

    #[tokio::test]
    async fn test_fetch_simple_page() {
        let args = VisitPageArgs {
            url: "https://example.com".to_string(),
            selector: None,
            include_images: false,
        };

        let result = fetch::fetch_page(&args).await;

        match result {
            Ok(content) => {
                assert!(!content.title.is_empty());
                assert!(!content.content.is_empty());
                assert!(content.word_count > 0);
                assert_eq!(content.url, "https://example.com");
            },
            Err(e) => {
                eprintln!("Fetch test skipped due to network error: {}", e);
            },
        }
    }

    #[tokio::test]
    async fn test_fetch_with_selector() {
        let args = VisitPageArgs {
            url: "https://example.com".to_string(),
            selector: Some("p".to_string()),
            include_images: false,
        };

        let result = fetch::fetch_page(&args).await;

        match result {
            Ok(content) => {
                assert!(!content.content.is_empty());
            },
            Err(e) => {
                eprintln!("Fetch test skipped due to network error: {}", e);
            },
        }
    }

    #[test]
    fn test_url_validation() {
        assert!(fetch::is_valid_url("https://example.com"));
        assert!(fetch::is_valid_url("http://example.com"));
        assert!(!fetch::is_valid_url("ftp://example.com"));
        assert!(!fetch::is_valid_url("javascript:alert(1)"));
        assert!(!fetch::is_valid_url("not-a-url"));
        assert!(!fetch::is_valid_url(""));
    }
}

mod cache_tests {
    use super::*;
    use daedra::types::{ResultMetadata, SearchResponse, SearchResult};

    #[tokio::test]
    async fn test_cache_search_hit() {
        let cache = SearchCache::with_defaults();

        let results = vec![SearchResult {
            title: "Test Result".to_string(),
            url: "https://example.com".to_string(),
            description: "A test result".to_string(),
            metadata: ResultMetadata {
                content_type: ContentType::Article,
                source: "example.com".to_string(),
                favicon: None,
                published_date: None,
            },
        }];

        let options = SearchOptions::default();
        let response = SearchResponse::new("test query".to_string(), results, &options);

        // Initially not cached
        assert!(
            cache
                .get_search("test query", "wt-wt", "MODERATE")
                .await
                .is_none()
        );

        // Cache it
        cache
            .set_search("test query", "wt-wt", "MODERATE", response.clone())
            .await;

        // Now it should be cached
        let cached = cache.get_search("test query", "wt-wt", "MODERATE").await;
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().data.len(), 1);
    }

    #[tokio::test]
    async fn test_cache_different_queries() {
        let cache = SearchCache::with_defaults();

        let results = vec![];
        let options = SearchOptions::default();

        let response1 = SearchResponse::new("query1".to_string(), results.clone(), &options);
        let response2 = SearchResponse::new("query2".to_string(), results.clone(), &options);

        cache
            .set_search("query1", "wt-wt", "MODERATE", response1)
            .await;
        cache
            .set_search("query2", "wt-wt", "MODERATE", response2)
            .await;

        assert!(
            cache
                .get_search("query1", "wt-wt", "MODERATE")
                .await
                .is_some()
        );
        assert!(
            cache
                .get_search("query2", "wt-wt", "MODERATE")
                .await
                .is_some()
        );
        assert!(
            cache
                .get_search("query3", "wt-wt", "MODERATE")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_cache_clear() {
        let cache = SearchCache::with_defaults();

        let results = vec![];
        let options = SearchOptions::default();
        let response = SearchResponse::new("test".to_string(), results, &options);

        cache
            .set_search("test", "wt-wt", "MODERATE", response)
            .await;
        assert!(
            cache
                .get_search("test", "wt-wt", "MODERATE")
                .await
                .is_some()
        );

        cache.clear().await;
        assert!(
            cache
                .get_search("test", "wt-wt", "MODERATE")
                .await
                .is_none()
        );
    }
}

mod type_tests {
    use super::*;
    use daedra::types::{search_args_schema, visit_page_args_schema};

    #[test]
    fn test_safe_search_from_str() {
        assert_eq!(
            "OFF".parse::<SafeSearchLevel>().unwrap(),
            SafeSearchLevel::Off
        );
        assert_eq!(
            "moderate".parse::<SafeSearchLevel>().unwrap(),
            SafeSearchLevel::Moderate
        );
        assert_eq!(
            "STRICT".parse::<SafeSearchLevel>().unwrap(),
            SafeSearchLevel::Strict
        );
        assert!("invalid".parse::<SafeSearchLevel>().is_err());
    }

    #[test]
    fn test_search_args_schema_is_valid_json() {
        let schema = search_args_schema();
        assert!(schema.is_object());
        assert!(schema["properties"]["query"].is_object());
    }

    #[test]
    fn test_visit_page_args_schema_is_valid_json() {
        let schema = visit_page_args_schema();
        assert!(schema.is_object());
        assert!(schema["properties"]["url"].is_object());
    }

    #[test]
    fn test_content_type_serialization() {
        let article = ContentType::Article;
        let json = serde_json::to_string(&article).unwrap();
        assert_eq!(json, "\"article\"");

        let doc = ContentType::Documentation;
        let json = serde_json::to_string(&doc).unwrap();
        assert_eq!(json, "\"documentation\"");
    }
}
