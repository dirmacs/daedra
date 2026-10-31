//! Type-level tests: region resolution, language/topic detection, and schemas.

use super::*;
use super::analysis::{detect_language, detect_topics};

    use super::*;

    #[test]
    fn test_safe_search_level_parsing() {
        assert_eq!(
            "OFF".parse::<SafeSearchLevel>().unwrap(),
            SafeSearchLevel::Off
        );
        assert_eq!(
            "MODERATE".parse::<SafeSearchLevel>().unwrap(),
            SafeSearchLevel::Moderate
        );
        assert_eq!(
            "STRICT".parse::<SafeSearchLevel>().unwrap(),
            SafeSearchLevel::Strict
        );
        assert_eq!(
            "moderate".parse::<SafeSearchLevel>().unwrap(),
            SafeSearchLevel::Moderate
        );
    }

    #[test]
    fn test_safe_search_ddg_value() {
        assert_eq!(SafeSearchLevel::Off.to_ddg_value(), -2);
        assert_eq!(SafeSearchLevel::Moderate.to_ddg_value(), -1);
        assert_eq!(SafeSearchLevel::Strict.to_ddg_value(), 1);
    }

    #[test]
    fn test_language_detection() {
        assert_eq!(detect_language("hello world"), "en");
        assert_eq!(detect_language("你好世界"), "zh");
        assert_eq!(detect_language("こんにちは"), "ja");
        assert_eq!(detect_language("안녕하세요"), "ko");
        assert_eq!(detect_language("привет"), "ru");
    }

    #[test]
    fn test_search_args_schema() {
        let schema = search_args_schema();
        assert!(schema["properties"]["query"].is_object());
        assert!(schema["properties"]["options"].is_object());
    }

    #[test]
    fn test_search_response_creation() {
        let results = vec![SearchResult {
            title: "Test".to_string(),
            url: "https://example.com".to_string(),
            description: "Test description".to_string(),
            metadata: ResultMetadata {
                content_type: ContentType::Article,
                source: "example.com".to_string(),
                favicon: None,
                published_date: None,
            },
        }];

        let options = SearchOptions::default();
        let response = SearchResponse::new("test query".to_string(), results, &options);

        assert_eq!(response.response_type, "search_results");
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.metadata.query, "test query");
    }

    #[test]
    fn test_detect_topics_technology() {
        let results = vec![SearchResult {
            title: "Rust repo".to_string(),
            url: "https://github.com/rust-lang/rust".to_string(),
            description: "Source".to_string(),
            metadata: ResultMetadata {
                content_type: ContentType::Documentation,
                source: "github.com".to_string(),
                favicon: None,
                published_date: None,
            },
        }];
        let response = SearchResponse::new("rust".to_string(), results, &SearchOptions::default());
        assert!(
            response
                .metadata
                .query_analysis
                .topics
                .contains(&"technology".to_string())
        );
    }

    #[test]
    fn test_detect_topics_empty() {
        let response = SearchResponse::new("query".to_string(), vec![], &SearchOptions::default());
        assert!(response.metadata.query_analysis.topics.is_empty());
    }

    #[test]
    fn test_safe_search_from_str_invalid() {
        assert!("invalid".parse::<SafeSearchLevel>().is_err());
    }

    #[test]
    fn test_crawl_args_defaults() {
        let args: CrawlArgs =
            serde_json::from_str(r#"{"root_url":"https://example.com"}"#).unwrap();
        assert_eq!(args.root_url, "https://example.com");
        assert_eq!(args.max_pages, 25);
        assert_eq!(args.concurrency, 4);
    }

    #[test]
    fn test_content_type_default() {
        assert_eq!(ContentType::default(), ContentType::Other);
    }

    #[test]
    fn test_page_content_serialize_deserialize() {
        let page = PageContent {
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            content: "# Hello".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            word_count: 1,
            content_type: Some("html".to_string()),
            links: Some(vec![PageLink {
                text: "Link".to_string(),
                url: "https://example.com/other".to_string(),
            }]),
        };
        let json = serde_json::to_string(&page).unwrap();
        let round_trip: PageContent = serde_json::from_str(&json).unwrap();
        assert_eq!(round_trip.url, page.url);
        assert_eq!(round_trip.title, page.title);
        assert_eq!(round_trip.content, page.content);
        assert_eq!(round_trip.word_count, page.word_count);
        assert_eq!(
            round_trip.links.as_ref().map(|v| v.len()),
            page.links.as_ref().map(|v| v.len())
        );
        assert_eq!(
            round_trip
                .links
                .as_ref()
                .and_then(|v| v.first())
                .map(|l| l.url.as_str()),
            page.links
                .as_ref()
                .and_then(|v| v.first())
                .map(|l| l.url.as_str())
        );
    }
