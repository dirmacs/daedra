//! TDD regression tests for GitHub issues #6, #7, #8.
//!
//! ## Workflow
//!
//! **Phase 1 (characterization)** — tests prefixed `characterization_` document *current*
//! buggy behavior. They must **pass on `main` today**.
//!
//! **Phase 2 (fixed)** — tests prefixed `fixed_` assert correct behavior. They must
//! **fail on `main` today** and **pass after the fix**.
//!
//! When behavior is correct, delete or `#[ignore]` the characterization tests and keep
//! the `fixed_` tests.
//!
//! ## Fixtures
//!
//! ```bash
//! mkdir -p tests/fixtures
//! curl -sL -A 'Mozilla/5.0' \
//!   'https://www.celiachia.it/ristorazione-e-celiachia-unindagine-rivela-5-gap-fra-locali-aderenti-al-network-afc-e-locali-non-afc/' \
//!   -o tests/fixtures/celiachia.html
//! ```
//!
//! `tests/fixtures/minimal.pdf` is a tiny inline PDF (see `fixtures/README` or generate
//! in test setup).

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use daedra::{
    tools::{
        backend::{SearchBackend, SearchProvider},
        fetch::FetchClient,
    },
    types::{
        ContentType, DaedraError, DaedraResult, ResultMetadata, SearchArgs, SearchOptions,
        SearchResponse, SearchResult, VisitPageArgs,
    },
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

mod helpers {
    use super::*;

    pub const CELIACHIA_FIXTURE: &str = include_str!("fixtures/celiachia.html");
    pub const MINIMAL_PDF: &[u8] = include_bytes!("fixtures/minimal.pdf");

    /// Phrase from the real article body (`.theme-post-content`), not related-post cards.
    pub const CELIACHIA_ARTICLE_MARKER: &str = "indagine 2023 su";

    pub const CELIACHIA_LIVE_URL: &str = "https://www.celiachia.it/ristorazione-e-celiachia-unindagine-rivela-5-gap-fra-locali-aderenti-al-network-afc-e-locali-non-afc/";

    pub const ISSUE_7_QUERY: &str = "my search query";

    /// Live PDF used in issue #8 reports (arXiv abstract PDF).
    pub const SAMPLE_PDF_URL: &str = "https://arxiv.org/pdf/1706.03762.pdf";

    pub fn celiachia_search_args() -> SearchArgs {
        SearchArgs {
            query: ISSUE_7_QUERY.to_string(),
            options: Some(SearchOptions {
                num_results: 10,
                region: "it-it".to_string(),
                ..Default::default()
            }),
        }
    }

    pub fn sample_result() -> SearchResult {
        SearchResult {
            title: "Mock result".to_string(),
            url: "https://example.com/doc".to_string(),
            description: "fixture".to_string(),
            metadata: ResultMetadata {
                content_type: ContentType::Article,
                source: "example.com".to_string(),
                favicon: None,
                published_date: None,
            },
        }
    }

    pub fn empty_response(query: &str) -> SearchResponse {
        SearchResponse::new(query.to_string(), vec![], &SearchOptions::default())
    }

    pub fn one_result_response(query: &str) -> SearchResponse {
        SearchResponse::new(
            query.to_string(),
            vec![sample_result()],
            &SearchOptions::default(),
        )
    }

    /// Fraction of bytes that are NUL or ASCII control chars (except \\n, \\r, \\t).
    pub fn non_printable_ratio(s: &str) -> f64 {
        let bytes = s.as_bytes();
        if bytes.is_empty() {
            return 0.0;
        }
        let bad = bytes
            .iter()
            .filter(|&&b| {
                b == 0
                    || (b < 0x20 && b != b'\n' && b != b'\r' && b != b'\t')
                    || b == 0x7f
            })
            .count();
        bad as f64 / bytes.len() as f64
    }

    pub fn looks_like_markdown_article(s: &str) -> bool {
        !s.is_empty()
            && !s.as_bytes().contains(&0)
            && non_printable_ratio(s) < 0.05
            && s.chars().any(|c| c.is_alphabetic())
    }
}

use helpers::*;

// ---------------------------------------------------------------------------
// Issue #6 — celiachia.it: wrong `<article>` → word_count < 50
// ---------------------------------------------------------------------------

mod issue_6 {
    use super::*;

    async fn fetch_celiachia_fixture() -> daedra::types::PageContent {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(CELIACHIA_FIXTURE)
                    .insert_header("content-type", "text/html; charset=utf-8"),
            )
            .mount(&server)
            .await;

        let client = FetchClient::new().expect("client");
        let args = VisitPageArgs {
            url: server.uri(),
            selector: None,
            include_images: false,
        };
        client.fetch(&args).await.expect("fetch fixture")
    }

    /// Phase 1: documents issue #6 — first `<article>` is related posts (~29 words).
    /// IGNORED: fix #6 (dom_smoothie) now extracts the full article body.
    #[tokio::test]
    #[ignore = "bug #6 fixed: dom_smoothie Readability extraction now returns full article"]
    async fn characterization_celiachia_word_count_below_threshold() {
        let page = fetch_celiachia_fixture().await;
        assert!(
            page.word_count < 50,
            "issue #6 characterization: expected low word_count from wrong article \
             element, got {} words; title={:?}",
            page.word_count,
            page.title
        );
        assert!(
            !page.content.contains(CELIACHIA_ARTICLE_MARKER),
            "buggy extraction should miss main article marker {:?}",
            CELIACHIA_ARTICLE_MARKER
        );
    }

    /// Phase 2: passes after selector scoring / readability fix in `fetch.rs`.
    #[tokio::test]
    async fn fixed_celiachia_extracts_full_article_body() {
        let page = fetch_celiachia_fixture().await;
        assert!(
            page.word_count >= 50,
            "issue #6 fix: expected substantial article body, got {} words",
            page.word_count
        );
        assert!(
            page.content.contains(CELIACHIA_ARTICLE_MARKER),
            "expected main article phrase {:?} in content",
            CELIACHIA_ARTICLE_MARKER
        );
        assert!(
            page.content.to_lowercase().contains("alimentazione fuori casa"),
            "expected AFC article vocabulary in extracted markdown"
        );
    }

    /// Live network repro for issue #6 (optional CI).
    #[tokio::test]
    #[ignore = "network: live celiachia.it fetch"]
    async fn characterization_celiachia_live_url_low_word_count() {
        let client = FetchClient::new().expect("client");
        let args = VisitPageArgs {
            url: CELIACHIA_LIVE_URL.to_string(),
            selector: None,
            include_images: false,
        };
        let page = client.fetch(&args).await.expect("live fetch");
        assert!(page.word_count < 50, "live issue #6: got {} words", page.word_count);
    }

    #[tokio::test]
    #[ignore = "network: live celiachia.it fetch"]
    async fn fixed_celiachia_live_url_full_article() {
        let client = FetchClient::new().expect("client");
        let args = VisitPageArgs {
            url: CELIACHIA_LIVE_URL.to_string(),
            selector: None,
            include_images: false,
        };
        let page = client.fetch(&args).await.expect("live fetch");
        assert!(page.word_count >= 50);
        assert!(page.content.contains(CELIACHIA_ARTICLE_MARKER));
    }
}

// ---------------------------------------------------------------------------
// Issue #7 — third consecutive search: all backends empty
// ---------------------------------------------------------------------------

mod issue_7 {
    use super::*;

    /// Simulates aggregate failure on the Nth `SearchProvider::search` call.
    ///
    /// `SearchProvider` queries every backend concurrently, so per-backend counters
    /// are wrong for this test — we count whole search rounds instead.
    struct FailFromNthSearchBackend {
        search_round: Arc<AtomicUsize>,
        fail_from_search: usize,
    }

    #[async_trait]
    impl SearchBackend for FailFromNthSearchBackend {
        async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
            let round = self.search_round.load(Ordering::SeqCst);
            if round >= self.fail_from_search {
                Ok(empty_response(&args.query))
            } else {
                Ok(one_result_response(&args.query))
            }
        }

        fn name(&self) -> &str {
            "mock-search-round"
        }
    }

    struct RoundCountingProvider {
        inner: SearchProvider,
        search_round: Arc<AtomicUsize>,
    }

    impl RoundCountingProvider {
        fn new(fail_from_search: usize) -> Self {
            let search_round = Arc::new(AtomicUsize::new(0));
            let backend = FailFromNthSearchBackend {
                search_round: Arc::clone(&search_round),
                fail_from_search,
            };
            Self {
                inner: SearchProvider::new(vec![Box::new(backend)]),
                search_round,
            }
        }

        async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
            self.search_round.fetch_add(1, Ordering::SeqCst);
            self.inner.search(args).await
        }
    }

    fn rate_limited_provider(fail_from_search: usize) -> RoundCountingProvider {
        RoundCountingProvider::new(fail_from_search)
    }

    async fn run_three_sequential_searches(
        provider: &RoundCountingProvider,
    ) -> Vec<DaedraResult<SearchResponse>> {
        let args = celiachia_search_args();
        let mut out = Vec::with_capacity(3);
        for _ in 0..3 {
            out.push(provider.search(&args).await);
        }
        out
    }

    /// Phase 1: models aggregate failure when every backend is empty (issue #7 symptom).
    #[tokio::test]
    async fn characterization_third_search_all_backends_empty_error() {
        let provider = rate_limited_provider(3);
        let results = run_three_sequential_searches(&provider).await;
        assert!(results[0].is_ok() && !results[0].as_ref().unwrap().data.is_empty());
        assert!(results[1].is_ok() && !results[1].as_ref().unwrap().data.is_empty());

        match &results[2] {
            Err(DaedraError::SearchError(msg)) => {
                assert!(
                    msg.contains("search backends returned 0 results"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected SearchError, got {other:?}"),
        }
    }

    /// Phase 2: placeholder — enable after retry/throttle/diagnostics fix.
    /// Rename to `fixed_` and implement retry in `SearchProvider::search`.
    #[tokio::test]
    #[ignore = "implement per-backend retry / rate-limit handling (issue #7 fix)"]
    async fn fixed_third_search_succeeds_after_rate_limit() {
        let provider = rate_limited_provider(3);
        let results = run_three_sequential_searches(&provider).await;
        let third = &results[2];
        assert!(
            third.is_ok(),
            "after fix, third search should succeed: {:?}",
            third.as_ref().err()
        );
        assert!(!third.as_ref().unwrap().data.is_empty());
    }

    /// Live repro: 3× sequential searches with real backends (flaky).
    #[tokio::test]
    #[ignore = "network: intermittent rate limits (issue #7)"]
    async fn characterization_live_triple_search_third_may_fail() {
        let provider = SearchProvider::auto();
        let args = celiachia_search_args();
        let mut outcomes = Vec::new();
        for i in 0..3 {
            outcomes.push(provider.search(&args).await);
            if i < 2 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
        if let Err(DaedraError::SearchError(msg)) = &outcomes[2] {
            assert!(msg.contains("All search backends returned 0 results"));
        }
        // If it passes, test is inconclusive — do not fail CI on flaky success.
    }

    #[tokio::test]
    #[ignore = "network: live triple search after issue #7 fix"]
    async fn fixed_live_triple_search_all_succeed() {
        let provider = SearchProvider::auto();
        let args = celiachia_search_args();
        for i in 0..3 {
            let r = provider.search(&args).await;
            assert!(r.is_ok(), "search {} failed: {:?}", i + 1, r.err());
            assert!(!r.unwrap().data.is_empty(), "search {} empty", i + 1);
            if i < 2 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Issue #8 — PDF served as raw bytes through `response.text()`
// ---------------------------------------------------------------------------

mod issue_8 {
    use super::*;

    async fn fetch_pdf_fixture() -> daedra::types::PageContent {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/doc.pdf"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(MINIMAL_PDF)
                    .insert_header("content-type", "application/pdf"),
            )
            .mount(&server)
            .await;

        let client = FetchClient::new().expect("client");
        let args = VisitPageArgs {
            url: format!("{}/doc.pdf", server.uri()),
            selector: None,
            include_images: false,
        };
        client.fetch(&args).await.expect("fetch pdf fixture")
    }

    /// Phase 1: PDF bytes run through HTML→Markdown pipeline as opaque text (not extracted).
    /// IGNORED: fix #8 (infer + pdf-extract) now extracts PDF text properly.
    #[tokio::test]
    #[ignore = "bug #8 fixed: infer MIME sniff + pdf-extract now extracts text from PDFs"]
    async fn characterization_pdf_fixture_non_text_content() {
        let page = fetch_pdf_fixture().await;
        assert!(
            page.content.contains("%PDF") || page.content.contains("endobj"),
            "issue #8 characterization: PDF wire format should appear in content as opaque text \
             (len={}, preview={:?})",
            page.content.len(),
            &page.content.chars().take(80).collect::<String>()
        );
    }

    /// Phase 2: passes after MIME routing + PDF text extraction.
    #[tokio::test]
    async fn fixed_pdf_fixture_extracted_as_text() {
        let page = fetch_pdf_fixture().await;
        assert!(
            looks_like_markdown_article(&page.content),
            "expected readable text, ratio={:.3}",
            non_printable_ratio(&page.content)
        );
        assert!(
            page.content.contains("Hello PDF") || page.content.contains("Hello"),
            "expected extracted text from minimal.pdf"
        );
        assert!(
            !page.content.contains("%PDF"),
            "fixed fetch must not return raw PDF file bytes as markdown"
        );
    }

    #[tokio::test]
    #[ignore = "network: live arXiv PDF fetch"]
    async fn characterization_live_pdf_non_markdown() {
        let client = FetchClient::new().expect("client");
        let args = VisitPageArgs {
            url: SAMPLE_PDF_URL.to_string(),
            selector: None,
            include_images: false,
        };
        let page = client
            .fetch(&args)
            .await
            .expect("live pdf fetch (may return garbage today)");
        assert!(
            page.content.contains("%PDF")
                || non_printable_ratio(&page.content) > 0.05
                || !page.content.to_lowercase().contains("attention"),
            "live PDF should not be a clean extracted article today"
        );
    }

    #[tokio::test]
    #[ignore = "network: live arXiv PDF after issue #8 fix"]
    async fn fixed_live_pdf_readable_markdown() {
        let client = FetchClient::new().expect("client");
        let args = VisitPageArgs {
            url: SAMPLE_PDF_URL.to_string(),
            selector: None,
            include_images: false,
        };
        let page = client.fetch(&args).await.expect("live pdf");
        assert!(looks_like_markdown_article(&page.content));
        assert!(page.word_count >= 10);
    }
}
