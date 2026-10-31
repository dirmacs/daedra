//! Provider, circuit-breaker, and ranking tests.

use super::*;
use crate::tools::backend::{BackendHealth, SearchProvider};
use super::limits::BackendRateLimiters;
use super::rank::{canonical_url_key, query_tokens, relevance_score};
use crate::types::{DaedraError, DaedraResult, SearchResponse};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

        use crate::types::SearchArgs;

    #[test]
    fn test_auto_has_backends() {
        let provider = SearchProvider::auto();
        let backends = provider.available_backends();
        assert_eq!(
            backends.len(),
            14,
            "Expected 14 unkeyed backends, got {}",
            backends.len()
        );
        assert!(backends.contains(&"mwmbl"));
        assert!(backends.contains(&"bing"));
        assert!(backends.contains(&"wikipedia"));
        assert!(backends.contains(&"stackoverflow"));
        assert!(backends.contains(&"github"));
        assert!(backends.contains(&"wiby"));
        assert!(backends.contains(&"ddg-instant"));
        assert!(backends.contains(&"duckduckgo"));
        assert!(
            !backends.contains(&"mojeek"),
            "Mojeek HTML scrape was removed; it served CAPTCHAs to this client"
        );
        assert!(
            !backends.contains(&"serper"),
            "paid search keys were removed from the crate"
        );
        assert!(
            !backends.contains(&"tavily"),
            "paid search keys were removed from the crate"
        );
    }

    #[test]
    fn test_empty_provider() {
        let provider = SearchProvider::new(vec![]);
        assert!(provider.available_backends().is_empty());
    }

    #[test]
    fn test_circuit_breaker_opens_after_failures() {
        let health = BackendHealth::new(3, Duration::from_secs(30));
        assert!(health.is_available());
        health.record_failure();
        health.record_failure();
        assert!(health.is_available());
        health.record_failure();
        assert!(!health.is_available());
        health.record_success();
        assert!(health.is_available());
    }

    #[test]
    fn test_circuit_breaker_half_open() {
        let health = BackendHealth::new(3, Duration::from_millis(50));
        for _ in 0..3 {
            health.record_failure();
        }
        assert!(!health.is_available());
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            health.is_available(),
            "after cooldown, circuit should be half-open (probe allowed)"
        );
    }

    #[test]
    fn test_circuit_breaker_stays_open_on_failure() {
        let health = BackendHealth::new(3, Duration::from_millis(50));
        for _ in 0..3 {
            health.record_failure();
        }
        std::thread::sleep(Duration::from_millis(60));
        assert!(health.is_available(), "half-open probe window");
        health.record_failure();
        assert!(
            !health.is_available(),
            "failed probe should keep circuit open"
        );
    }

    #[test]
    fn test_is_non_retryable() {
        assert!(SearchProvider::is_non_retryable(
            &DaedraError::BotProtectionDetected
        ));
        assert!(SearchProvider::is_non_retryable(
            &DaedraError::RateLimitExceeded
        ));
        assert!(SearchProvider::is_non_retryable(&DaedraError::SearchError(
            "HTTP 403 forbidden".to_string()
        )));
        assert!(SearchProvider::is_non_retryable(&DaedraError::SearchError(
            "CAPTCHA required".to_string()
        )));
        assert!(!SearchProvider::is_non_retryable(&DaedraError::Timeout));
    }

    #[tokio::test]
    async fn test_transient_classification() {
        let client = reqwest::Client::new();

        // A connection refused to a closed port is the genuine transient this
        // retry exists for: the second attempt can plausibly succeed.
        let conn_err =
            DaedraError::HttpError(client.get("http://127.0.0.1:1").send().await.unwrap_err());
        assert!(SearchProvider::is_transient(&conn_err));

        // A TIMEOUT IS NOT TRANSIENT. Retrying it buys one more full timeout
        // and nothing else. Measured 2026-10-31: marginalia's public API never
        // answered (curl http=000 after 40 s), and treating this as transient
        // made every search pay 8 s for the attempt plus 8 s for the retry,
        // observed as a flat 12 s per search.
        assert!(!SearchProvider::is_transient(&DaedraError::Timeout));

        assert!(!SearchProvider::is_transient(&DaedraError::SearchError(
            "not transient".to_string()
        )));
        assert!(!SearchProvider::is_transient(
            &DaedraError::BotProtectionDetected
        ));
    }

    #[test]
    fn test_backend_rate_limiters_default() {
        let limiter = BackendRateLimiters::default_limiter();
        assert!(limiter.check_key(&"bing".to_string()).is_ok());
    }

    fn test_search_result(url: &str, title: &str) -> crate::types::SearchResult {
        use crate::types::{ContentType, ResultMetadata, SearchResult};
        SearchResult {
            title: title.to_string(),
            url: url.to_string(),
            description: "desc".to_string(),
            metadata: ResultMetadata {
                content_type: ContentType::Other,
                source: "test".to_string(),
                favicon: None,
                published_date: None,
            },
        }
    }

    #[test]
    fn test_merge_ranked_interleaves_equal_scores() {
        // All results score equally against the query, so the k-way merge
        // degenerates to the old round-robin interleave.
        let a1 = test_search_result("https://a/1", "a1");
        let a2 = test_search_result("https://a/2", "a2");
        let b1 = test_search_result("https://b/1", "b1");
        let b2 = test_search_result("https://b/2", "b2");
        let by_source = vec![
            ("a".to_string(), vec![a1.clone(), a2.clone()]),
            ("b".to_string(), vec![b1.clone(), b2.clone()]),
        ];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 4);
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0].url, "https://a/1");
        assert_eq!(merged[1].url, "https://b/1");
        assert_eq!(merged[2].url, "https://a/2");
        assert_eq!(merged[3].url, "https://b/2");
    }

    #[test]
    fn test_merge_ranked_demotes_off_topic_results() {
        // Backend "rss" answers a different question than the one asked; its
        // results must land after every matched result.
        let on_topic_1 = test_search_result("https://tokio.rs/guide", "Tokio runtime guide");
        let on_topic_2 = test_search_result("https://tokio.rs/tutorial", "Tokio tutorial");
        let junk_1 = test_search_result("https://betting.example/vn", "Football betting odds");
        let junk_2 = test_search_result("https://install.example/chrome", "Chrome install");
        let by_source = vec![
            ("rss".to_string(), vec![junk_1, junk_2]),
            ("api".to_string(), vec![on_topic_1, on_topic_2]),
        ];
        let merged = SearchProvider::merge_ranked_results(&by_source, "tokio runtime", 4);
        assert_eq!(merged[0].url, "https://tokio.rs/guide");
        assert_eq!(merged[1].url, "https://tokio.rs/tutorial");
        assert_eq!(merged[2].url, "https://betting.example/vn");
        assert_eq!(merged[3].url, "https://install.example/chrome");
    }

    #[test]
    fn test_relevance_score_gives_prefix_credit() {
        // Rare names: engines answer with stem matches ("baalateja" →
        // "Baalat") and the literal token never appears. The fragment must
        // still score half, and only when it covers half the token.
        let tokens = query_tokens("baalateja");
        let r = test_search_result("https://en.wikipedia.org/wiki/Baalat_Gebal", "Baalat Gebal");
        let score = relevance_score(&r, &tokens);
        assert!((score - 0.5).abs() < 1e-9, "expected 0.5, got {score}");
        // A sliver shorter than half the token earns nothing.
        let sliver = test_search_result("https://en.wikipedia.org/wiki/Baal", "Baal - Wikipedia");
        assert_eq!(relevance_score(&sliver, &tokens), 0.0);
        // Short tokens cannot produce a meaningful fragment: no credit.
        let short = query_tokens("rust");
        let unrelated = test_search_result("https://example.com/x", "Unrelated page");
        assert_eq!(relevance_score(&unrelated, &short), 0.0);
    }

    #[test]
    fn test_merge_ranked_all_zero_still_ranks_prefix_matches() {
        let stem = test_search_result("https://en.wikipedia.org/wiki/Baalat_Gebal", "Baalat Gebal");
        let junk = test_search_result("https://betting.example/vn", "Football betting odds");
        let by_source = vec![
            ("bing-rss".to_string(), vec![junk]),
            ("wikipedia".to_string(), vec![stem]),
        ];
        let merged = SearchProvider::merge_ranked_results(&by_source, "baalateja", 2);
        // The fragment match outranks the fully unrelated result.
        assert_eq!(merged[0].url, "https://en.wikipedia.org/wiki/Baalat_Gebal");
    }

    #[test]
    fn test_query_tokens_drops_stopwords_and_short_runs() {
        let tokens = query_tokens("The capital of France");
        assert_eq!(tokens, vec!["capital".to_string(), "france".to_string()]);
        assert!(query_tokens("a of the").is_empty());
    }

    #[test]
    fn test_relevance_score_matches_url_and_neutral_empty() {
        let r = test_search_result("https://tokio.rs/runtime", "Guide");
        let tokens = query_tokens("tokio");
        // Token hit (1.0) plus full-phrase bonus in the URL (1.0).
        assert_eq!(relevance_score(&r, &tokens), 2.0);
        // No signal at all: neutral score keeps the plain round-robin order.
        assert_eq!(relevance_score(&r, &[]), 1.0);
    }

    #[test]
    fn test_canonical_url_key_dedup_and_tracking() {
        let a = canonical_url_key("https://www.Example.com/docs/guide/?utm_source=x").unwrap();
        let b = canonical_url_key("https://example.com/docs/guide").unwrap();
        assert_eq!(a, b, "www, trailing slash, utm params must not matter");
        let c = canonical_url_key("https://example.com/docs/other").unwrap();
        assert_ne!(a, c);
        assert!(canonical_url_key("not a url").is_none());
    }

    #[test]
    fn test_merge_corroboration_boost() {
        // The same page found by two engines outranks an otherwise equal
        // single-engine hit.
        let dup_a = test_search_result("https://tokio.rs/", "Tokio runtime");
        let dup_b = test_search_result("https://www.tokio.rs/?utm_source=x", "Tokio runtime");
        let other = test_search_result("https://other.test/tokio", "Tokio runtime guide");
        let by_source = vec![
            ("a".to_string(), vec![other]),
            ("b".to_string(), vec![dup_a]),
            ("c".to_string(), vec![dup_b]),
        ];
        let merged = SearchProvider::merge_ranked_results(&by_source, "tokio runtime", 3);
        assert_eq!(
            merged[0].url, "https://tokio.rs/",
            "corroborated result first"
        );
        // The www/utm variant is deduped by canonical key.
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn test_relevance_score_phrase_bonus_breaks_ties() {
        let wiki = test_search_result(
            "https://en.wikipedia.org/wiki/Capital_of_France",
            "Capital of France",
        );
        let news = test_search_result(
            "https://example.test/marseille",
            "Marseille is France's new capital of overtourism",
        );
        let tokens = query_tokens("capital of France");
        let wiki_score = relevance_score(&wiki, &tokens);
        let news_score = relevance_score(&news, &tokens);
        assert!(
            wiki_score > news_score,
            "phrase match must outrank word-bag match"
        );
    }

    #[test]
    fn test_merge_ranked_dedup() {
        let shared = test_search_result("https://dup", "dup");
        let other = test_search_result("https://other", "other");
        let by_source = vec![
            ("a".to_string(), vec![shared.clone()]),
            ("b".to_string(), vec![shared, other.clone()]),
        ];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 10);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].url, "https://dup");
        assert_eq!(merged[1].url, "https://other");
    }

    #[test]
    fn test_merge_ranked_respects_target() {
        let results: Vec<_> = (0..5)
            .map(|i| test_search_result(&format!("https://x/{}", i), &format!("r{}", i)))
            .collect();
        let by_source = vec![("x".to_string(), results)];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 3);
        assert_eq!(merged.len(), 3);
    }

    #[test]
    fn test_is_non_retryable_patterns() {
        for msg in [
            "HTTP 403 forbidden",
            "CAPTCHA required",
            "bot protection triggered",
            "bot detected on page",
        ] {
            assert!(
                SearchProvider::is_non_retryable(&DaedraError::SearchError(msg.to_string())),
                "expected non-retryable: {msg}"
            );
        }
        assert!(!SearchProvider::is_non_retryable(&DaedraError::Timeout));
        assert!(!SearchProvider::is_non_retryable(
            &DaedraError::SearchError("connection reset".to_string())
        ));
    }

    #[test]
    fn test_categorize_results_all_success() {
        use crate::types::SearchOptions;
        let opts = SearchOptions::default();
        let ok = |name: &str, url: &str| {
            (
                name.to_string(),
                Ok(SearchResponse::new(
                    "q".to_string(),
                    vec![test_search_result(url, name)],
                    &opts,
                )),
            )
        };
        let results = vec![ok("a", "https://a"), ok("b", "https://b")];
        let (by_source, any_success, tried, failures) = SearchProvider::categorize_results(results);
        assert!(any_success);
        assert_eq!(tried.len(), 2);
        assert_eq!(by_source.len(), 2);
        assert!(failures.is_empty());
    }

    #[test]
    fn test_aggregate_failure_message_all_errors() {
        let tried = vec!["bing".to_string(), "wikipedia".to_string()];
        let failures = vec![
            ("bing".to_string(), "rate limit exceeded".to_string()),
            ("wikipedia".to_string(), "HTTP status 503".to_string()),
        ];
        let msg = SearchProvider::aggregate_failure_message(&tried, &failures, &[], &[]);
        assert!(msg.contains("2 search backends failed"), "{msg}");
        assert!(msg.contains("bing: rate limit exceeded"), "{msg}");
        assert!(msg.contains("wikipedia: HTTP status 503"), "{msg}");
        assert!(!msg.contains("returned 0 results"), "{msg}");
    }

    #[test]
    fn test_aggregate_failure_message_all_empty() {
        let tried = vec!["wikipedia".to_string(), "github".to_string()];
        let msg = SearchProvider::aggregate_failure_message(&tried, &[], &tried.clone(), &[]);
        assert!(
            msg.contains("All 2 search backends returned 0 results"),
            "{msg}"
        );
    }

    #[test]
    fn test_aggregate_failure_message_mixed() {
        let tried = vec![
            "bing".to_string(),
            "wikipedia".to_string(),
            "wiby".to_string(),
        ];
        let failures = vec![("bing".to_string(), "CAPTCHA".to_string())];
        let empty = vec!["wiby".to_string()];
        let msg = SearchProvider::aggregate_failure_message(
            &tried,
            &failures,
            &empty,
            &["bing".to_string()],
        );
        assert!(msg.contains("returned no usable results"), "{msg}");
        assert!(msg.contains("failed: [bing: CAPTCHA]"), "{msg}");
        assert!(msg.contains("empty: [wiby]"), "{msg}");
        assert!(msg.contains("open circuits: [bing]"), "{msg}");
    }

    #[test]
    fn test_categorize_results_all_failure() {
        let results = vec![
            (
                "a".to_string(),
                Err(DaedraError::SearchError("fail a".to_string())),
            ),
            (
                "b".to_string(),
                Err(DaedraError::SearchError("fail b".to_string())),
            ),
        ];
        let (by_source, any_success, tried, failures) = SearchProvider::categorize_results(results);
        assert!(!any_success);
        assert_eq!(tried.len(), 2);
        assert!(by_source.is_empty());
        assert_eq!(
            failures,
            vec![
                ("a".to_string(), "Search failed: fail a".to_string()),
                ("b".to_string(), "Search failed: fail b".to_string()),
            ]
        );
    }

    #[test]
    fn test_categorize_results_mixed() {
        use crate::types::SearchOptions;
        let opts = SearchOptions::default();
        let results = vec![
            (
                "ok".to_string(),
                Ok(SearchResponse::new(
                    "q".to_string(),
                    vec![test_search_result("https://ok", "ok")],
                    &opts,
                )),
            ),
            (
                "fail".to_string(),
                Err(DaedraError::SearchError("fail".to_string())),
            ),
        ];
        let (by_source, any_success, tried, failures) = SearchProvider::categorize_results(results);
        assert!(any_success);
        assert_eq!(tried.len(), 2);
        assert_eq!(by_source.len(), 1);
        assert_eq!(by_source[0].0, "ok");
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, "fail");
    }

    #[tokio::test]
    async fn test_fallback_chain_live() {
        // This test uses real network — Wikipedia + SO should always return results
        let provider = SearchProvider::auto();
        let args = SearchArgs {
            query: "Rust programming".to_string(),
            options: Some(crate::types::SearchOptions {
                num_results: 3,
                ..Default::default()
            }),
        };
        let response = provider.search(&args).await;
        assert!(
            response.is_ok(),
            "Fallback chain should find results from at least one backend"
        );
        let data = response.unwrap();
        assert!(!data.data.is_empty(), "Should have at least 1 result");
    }

    #[test]
    fn test_record_health_outcome_success() {
        let health = Arc::new(BackendHealth::new(3, Duration::from_secs(30)));
        health.record_failure();
        health.record_failure();
        assert!(health.is_available());
        SearchProvider::record_health_outcome(&Some(health.clone()), true);
        assert!(health.is_available());
    }

    #[test]
    fn test_record_health_outcome_failure() {
        let health = Arc::new(BackendHealth::new(3, Duration::from_secs(30)));
        health.record_failure();
        health.record_failure();
        SearchProvider::record_health_outcome(&Some(health.clone()), false);
        assert!(!health.is_available());
    }

    #[test]
    fn test_record_health_outcome_no_health() {
        SearchProvider::record_health_outcome(&None, true);
        SearchProvider::record_health_outcome(&None, false);
    }

    struct TransientThenOkBackend {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl SearchBackend for TransientThenOkBackend {
        async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                return Err(DaedraError::Timeout);
            }
            let opts = args.options.clone().unwrap_or_default();
            Ok(SearchResponse::new(
                args.query.clone(),
                vec![test_search_result("https://mock/1", "mock")],
                &opts,
            ))
        }

        fn name(&self) -> &str {
            "mock-transient"
        }
    }

    #[test]
    fn test_is_transient_rate_limit() {
        assert!(SearchProvider::is_transient(&DaedraError::SearchError(
            "HTTP 429 Too Many Requests".to_string(),
        )));
    }

    #[test]
    fn test_is_transient_timeout() {
        assert!(SearchProvider::is_transient(&DaedraError::SearchError(
            "connection timed out".to_string(),
        )));
    }

    #[test]
    fn test_is_non_retryable_bot_protection() {
        assert!(SearchProvider::is_non_retryable(
            &DaedraError::BotProtectionDetected,
        ));
    }

    #[test]
    fn test_handle_successful_result_records_health() {
        use crate::types::SearchOptions;
        let health = Arc::new(BackendHealth::new(3, Duration::from_secs(30)));
        health.record_failure();
        health.record_failure();
        let opts = SearchOptions::default();
        let ok = Ok(SearchResponse::new(
            "q".to_string(),
            vec![test_search_result("https://ok", "ok")],
            &opts,
        ));
        let (_name, _) = SearchProvider::handle_successful_result(
            "backend".to_string(),
            ok,
            Some(health.clone()),
        );
        assert!(health.is_available());
    }

    #[test]
    fn test_handle_non_retryable_records_failure() {
        let health = Arc::new(BackendHealth::new(3, Duration::from_secs(30)));
        health.record_failure();
        health.record_failure();
        let err = Err(DaedraError::BotProtectionDetected);
        let (_name, _) =
            SearchProvider::handle_non_retryable("backend".to_string(), err, Some(health.clone()));
        assert!(!health.is_available());
    }

    #[test]
    fn test_handle_unrecoverable_records_failure() {
        let health = Arc::new(BackendHealth::new(3, Duration::from_secs(30)));
        health.record_failure();
        health.record_failure();
        let err = Err(DaedraError::SearchError("unknown failure".to_string()));
        let (_name, _) = SearchProvider::handle_unrecoverable_error(
            "backend".to_string(),
            err,
            Some(health.clone()),
        );
        assert!(!health.is_available());
    }

    #[test]
    fn test_merge_ranked_results_empty() {
        let merged = SearchProvider::merge_ranked_results(&[], "query", 10);
        assert!(merged.is_empty());
    }

    #[test]
    fn test_merge_ranked_results_single_source() {
        let results: Vec<_> = (0..3)
            .map(|i| test_search_result(&format!("https://only/{}", i), &format!("r{}", i)))
            .collect();
        let by_source = vec![("only".to_string(), results.clone())];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 10);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].url, "https://only/0");
        assert_eq!(merged[1].url, "https://only/1");
        assert_eq!(merged[2].url, "https://only/2");
    }

    #[test]
    fn test_merge_ranked_results_multiple_sources() {
        let a1 = test_search_result("https://a/1", "a1");
        let b1 = test_search_result("https://b/1", "b1");
        let by_source = vec![
            ("a".to_string(), vec![a1.clone()]),
            ("b".to_string(), vec![b1.clone()]),
        ];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 2);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].url, "https://a/1");
        assert_eq!(merged[1].url, "https://b/1");
    }

    #[test]
    fn test_merge_ranked_empty_sources() {
        let merged = SearchProvider::merge_ranked_results(&[], "query", 10);
        assert!(merged.is_empty());
    }

    #[test]
    fn test_merge_ranked_single_source() {
        let results: Vec<_> = (0..3)
            .map(|i| test_search_result(&format!("https://only/{}", i), &format!("r{}", i)))
            .collect();
        let by_source = vec![("only".to_string(), results)];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 10);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].url, "https://only/0");
        assert_eq!(merged[1].url, "https://only/1");
        assert_eq!(merged[2].url, "https://only/2");
    }

    #[test]
    fn test_merge_ranked_uneven_sources() {
        let a: Vec<_> = (0..3)
            .map(|i| test_search_result(&format!("https://a/{}", i), &format!("a{}", i)))
            .collect();
        let b = vec![test_search_result("https://b/0", "b0")];
        let by_source = vec![("a".to_string(), a), ("b".to_string(), b)];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 10);
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0].url, "https://a/0");
        assert_eq!(merged[1].url, "https://b/0");
        assert_eq!(merged[2].url, "https://a/1");
        assert_eq!(merged[3].url, "https://a/2");
    }

    #[test]
    fn test_merge_ranked_all_duplicates() {
        let dup = test_search_result("https://dup", "dup");
        let by_source = vec![
            ("a".to_string(), vec![dup.clone()]),
            ("b".to_string(), vec![dup]),
        ];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 10);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].url, "https://dup");
    }

    #[test]
    fn test_merge_ranked_target_zero() {
        let results = vec![test_search_result("https://x/0", "r0")];
        let by_source = vec![("x".to_string(), results)];
        let merged = SearchProvider::merge_ranked_results(&by_source, "query", 0);
        assert!(merged.is_empty());
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn test_handle_transient_error() {
        let backend = TransientThenOkBackend {
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let health = Arc::new(BackendHealth::new(3, Duration::from_secs(30)));
        let limiters = Arc::new(BackendRateLimiters::new());
        let scraper_default = BackendRateLimiters::default_limiter();
        let args = SearchArgs {
            query: "transient-retry".to_string(),
            options: None,
        };
        let first_err = backend.search(&args).await.unwrap_err();
        assert!(SearchProvider::is_transient(&first_err));

        let (_name, result) = SearchProvider::handle_transient_error(
            &backend,
            &args,
            backend.name().to_string(),
            Err(first_err),
            Some(health.clone()),
            &limiters,
            &scraper_default,
        )
        .await;
        assert!(result.is_ok());
        assert!(!result.unwrap().data.is_empty());
        assert!(health.is_available());
    }

    // ── two-phase aggregate deadline ────────────────────────────────────────

    struct SlowBackend {
        name: &'static str,
        delay: Duration,
        results: usize,
    }

    #[async_trait]
    impl SearchBackend for SlowBackend {
        async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
            tokio::time::sleep(self.delay).await;
            let opts = args.options.clone().unwrap_or_default();
            let data: Vec<crate::types::SearchResult> = (0..self.results)
                .map(|i| test_search_result(&format!("https://slow/{i}"), "slow"))
                .collect();
            Ok(SearchResponse::new(args.query.clone(), data, &opts))
        }

        fn name(&self) -> &str {
            self.name
        }
    }

    fn args_for(q: &str) -> SearchArgs {
        SearchArgs { query: q.to_string(), options: None }
    }

    /// A backend that answers must not be held hostage by one that does not.
    /// Asserts ELAPSED TIME, not mere completion: `timeout(join_all(..))`
    /// would satisfy a completion assertion while still waiting for the
    /// slowest future.
    #[tokio::test]
    async fn slow_backend_does_not_block_fast_answer_past_soft_deadline() {
        let fast = SlowBackend { name: "fast", delay: Duration::from_millis(20), results: 1 };
        let slow = SlowBackend { name: "slow", delay: Duration::from_secs(60), results: 1 };
        let provider = SearchProvider::new(vec![Box::new(fast), Box::new(slow)])
            .with_deadlines(Duration::from_millis(300), Duration::from_secs(5));

        let started = std::time::Instant::now();
        let refs = provider.backend_refs();
        let out = provider.execute_concurrent_queries(&refs, &args_for("x")).await;
        let elapsed = started.elapsed();

        assert!(!out.is_empty(), "the answered backend must be returned");
        assert!(
            elapsed < Duration::from_secs(2),
            "must return near the soft deadline, not wait for the 60s backend (took {elapsed:?})"
        );
    }

    /// With nothing answering, the call must return at the hard deadline, not
    /// hang. The hard deadline is set BELOW BACKEND_DEADLINE so this phase is
    /// reachable; in production the per-backend cap resolves first and the
    /// hard timer is a backstop.
    #[tokio::test]
    async fn all_slow_backends_return_at_hard_deadline() {
        let a = SlowBackend { name: "slow-a", delay: Duration::from_secs(120), results: 1 };
        let b = SlowBackend { name: "slow-b", delay: Duration::from_secs(120), results: 1 };
        let provider = SearchProvider::new(vec![Box::new(a), Box::new(b)])
            .with_deadlines(Duration::from_millis(200), Duration::from_millis(700));

        let started = std::time::Instant::now();
        let refs = provider.backend_refs();
        let out = provider.execute_concurrent_queries(&refs, &args_for("x")).await;
        let elapsed = started.elapsed();

        // Nothing SUCCEEDED. The collector still reports the abandoned
        // backends as error entries, which is deliberate: a backend that
        // disappears from the result set reads as "it answered nothing".
        assert!(
            out.iter().all(|(_, r)| r.is_err()),
            "nothing had answered yet; expected only abandonment errors"
        );
        assert!(
            elapsed >= Duration::from_millis(650) && elapsed < Duration::from_secs(3),
            "must return AT the hard deadline, not earlier and not hang (took {elapsed:?})"
        );
    }

    /// A backend that blows its per-backend deadline must be CHARGED a
    /// circuit-breaker failure, not silently abandoned.
    ///
    /// This is the part of the deadline path that has behavioural
    /// consequences beyond latency: an endpoint that keeps timing out has to
    /// eventually stop being asked. Without the charge, a dead backend costs
    /// BACKEND_DEADLINE on every single search forever.
    ///
    /// The assertion is on breaker state (`is_available()` after enough
    /// charges), not on log output, because breaker state is what the next
    /// search actually reads.
    #[tokio::test]
    async fn backend_exceeding_deadline_is_charged_a_circuit_failure() {
        // Delay far past BACKEND_DEADLINE (5s) so the per-backend timeout path
        // is the one that fires.
        let hung = SlowBackend {
            name: "hung",
            delay: Duration::from_secs(120),
            results: 1,
        };
        let provider = SearchProvider::new(vec![Box::new(hung)])
            .with_deadlines(Duration::from_millis(100), Duration::from_millis(400));

        let refs = provider.backend_refs();
        let out = provider.execute_concurrent_queries(&refs, &args_for("x")).await;

        // The backend was abandoned, and it surfaced as an error entry rather
        // than vanishing (a silently dropped backend looks like "it answered
        // nothing", which is a different and misleading claim).
        assert_eq!(out.len(), 1, "an abandoned backend must still be reported");
        assert!(out[0].1.is_err(), "deadline abandonment must be an error");

        // Assert the CHARGE, not threshold-crossing: the breaker opens at 3
        // and a single abandoned search charges fewer than that, so
        // `is_available()` would report "closed" even though the charge
        // landed.
        let breaker = provider.breaker_for("hung").expect("breaker exists");
        assert!(
            breaker.consecutive_failures() >= 1,
            "the abandoned backend must be charged a failure (got {})",
            breaker.consecutive_failures()
        );
    }

