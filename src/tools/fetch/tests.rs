use super::client::{
    FetchClient, FetchedContent, classify_response_status, extract_with_readability,
    has_bot_protection_element, has_suspicious_title, is_retryable_status, is_skippable_href,
    is_valid_url, normalize_link_text, resolve_href, text_from_selector, title_from_url,
    word_count,
};
use super::extract::{
    classify_by_fallback, classify_by_inference, classify_fetched_content, classify_inferred_mime,
    extract_document_content, extract_pdf_content, is_container_mime, is_office_content_type,
    is_textual_content_type,
};
use super::markdown::{clean_markdown, clean_title, html_to_markdown, strip_img_tags};
use super::mime::{
    MAX_CONTENT_SIZE, check_body_size, is_binary_mime, is_known_binary_content_type,
    normalize_content_type, validate_url,
};
use crate::types::{DaedraError, DaedraResult, PageContent, SearchResponse};
use scraper::Html;
use url::Url;

/// Same selector `client.rs` uses for page titles. Parsed here rather than
/// re-exported from the production lazy_static block, which stays private.
fn TITLE_SELECTOR() -> scraper::Selector {
    scraper::Selector::parse("title").unwrap()
}

const CELIACHIA_FIXTURE: &str = include_str!("../../../tests/fixtures/celiachia.html");
const CELIACHIA_ARTICLE_MARKER: &str = "indagine 2023 su";

#[test]
#[ignore = "bug #6 fixed: dom_smoothie now extracts full article"]
fn characterization_issue_6_celiachia_extract_content_low_word_count() {
    let client = FetchClient::default();
    let content = client
        .extract_content_from_html_for_tests(CELIACHIA_FIXTURE, None, false)
        .expect("extract");
    let words = content.split_whitespace().count();
    assert!(
        words < 50,
        "issue #6 characterization: expected <50 words, got {words}"
    );
    assert!(!content.contains(CELIACHIA_ARTICLE_MARKER));
}

#[test]
fn fixed_issue_6_celiachia_extract_content_full_article() {
    let client = FetchClient::default();
    let content = client
        .extract_content_from_html_for_tests(CELIACHIA_FIXTURE, None, false)
        .expect("extract");
    let words = content.split_whitespace().count();
    assert!(
        words >= 50,
        "issue #6 fix: expected >=50 words, got {words}"
    );
    assert!(content.contains(CELIACHIA_ARTICLE_MARKER));
}

#[test]
fn test_is_valid_url() {
    assert!(is_valid_url("https://example.com"));
    assert!(is_valid_url("http://example.com"));
    assert!(!is_valid_url("ftp://example.com"));
    assert!(!is_valid_url("javascript:alert(1)"));
    assert!(!is_valid_url("not a url"));
}

#[test]
fn test_clean_title() {
    assert_eq!(clean_title("Page Title | Site Name"), "Page Title");
    assert_eq!(clean_title("Page Title - Site Name"), "Page Title");
    assert_eq!(clean_title("Simple Title"), "Simple Title");
}

#[test]
fn test_clean_markdown() {
    let input = "# Title\n\n\n\nParagraph\n\n\n\n\n\nAnother paragraph";
    let expected = "# Title\n\nParagraph\n\nAnother paragraph";
    assert_eq!(clean_markdown(input), expected);
}

#[test]
fn test_clean_markdown_excessive_blanks() {
    let input = "Line one\n\n\n\n\nLine two";
    assert_eq!(clean_markdown(input), "Line one\n\nLine two");
}

#[test]
fn test_clean_markdown_strips_list_markers() {
    let input = "Content\n-\n*\n+\nMore";
    assert_eq!(clean_markdown(input), "Content\nMore");
}

#[test]
fn test_clean_markdown_preserves_content() {
    let input = "# Heading\n\nParagraph with **bold** text.";
    assert_eq!(
        clean_markdown(input),
        "# Heading\n\nParagraph with **bold** text."
    );
}

#[test]
fn test_clean_markdown_empty_input() {
    assert_eq!(clean_markdown(""), "");
}

#[test]
fn test_clean_markdown_only_blanks() {
    assert_eq!(clean_markdown("\n\n\n\n"), "");
}

#[test]
fn test_html_to_markdown() {
    let html = "<h1>Title</h1><p>Paragraph with <strong>bold</strong> text.</p>";
    let markdown = html_to_markdown(html);
    assert!(markdown.contains("Title"));
    assert!(markdown.contains("Paragraph"));
    assert!(markdown.contains("bold"));
}

#[test]
fn test_classify_fetched_content_html() {
    let bytes = b"<html><body><p>Hello</p></body></html>";
    let result = classify_fetched_content("text/html", bytes).unwrap();
    assert!(matches!(result, FetchedContent::Html(_)));
}

#[test]
fn test_classify_fetched_content_pdf() {
    let bytes = include_bytes!("../../../tests/fixtures/minimal.pdf");
    let result = classify_fetched_content("application/pdf", bytes).unwrap();
    assert!(matches!(result, FetchedContent::Pdf(_)));
}

#[test]
fn test_classify_fetched_content_binary() {
    let bytes: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46];
    let result = classify_fetched_content("image/jpeg", bytes).unwrap();
    assert!(matches!(result, FetchedContent::Binary { .. }));
}

#[test]
fn test_classify_fetched_content_fallback_utf8() {
    let bytes = b"plain text without magic bytes";
    let result = classify_fetched_content("", bytes).unwrap();
    assert!(matches!(result, FetchedContent::Html(_)));
}

#[test]
fn test_classify_fetched_content_fallback_binary() {
    let bytes: &[u8] = &[0x80, 0x81, 0x82, 0x83];
    let result = classify_fetched_content("", bytes).unwrap();
    assert!(matches!(result, FetchedContent::Binary { .. }));
}

#[test]
fn test_normalize_content_type() {
    assert_eq!(
        normalize_content_type("text/html; charset=utf-8"),
        "text/html"
    );
}

#[test]
fn test_is_known_binary_content_type() {
    assert!(is_known_binary_content_type("image/png"));
    assert!(!is_known_binary_content_type("text/html"));
    assert!(is_known_binary_content_type("application/zip"));
}

#[test]
fn test_check_body_size_ok() {
    assert!(check_body_size(100).is_ok());
}

#[test]
fn test_check_body_size_too_large() {
    assert!(check_body_size(MAX_CONTENT_SIZE + 1).is_err());
}

#[test]
fn test_extract_links() {
    let html = r#"<html><body>
            <a href="https://example.com/one">First Link</a>
            <a href="/two">Second Link</a>
            <a href="https://example.com/three">Third Link</a>
        </body></html>"#;
    let document = Html::parse_document(html);
    let base = Url::parse("https://example.com/page").unwrap();
    let client = FetchClient::default();
    let links = client.extract_links(&document, &base);
    assert_eq!(links.len(), 3);
    assert_eq!(links[0].text, "First Link");
    assert_eq!(links[0].url, "https://example.com/one");
    assert_eq!(links[1].text, "Second Link");
    assert_eq!(links[1].url, "https://example.com/two");
    assert_eq!(links[2].text, "Third Link");
    assert_eq!(links[2].url, "https://example.com/three");
}

#[test]
fn test_extract_links_skips_javascript() {
    let html = r#"<html><body>
            <a href="javascript:void(0)">JS</a>
            <a href="mailto:test@example.com">Mail</a>
            <a href="https://example.com/ok">OK Link</a>
        </body></html>"#;
    let document = Html::parse_document(html);
    let base = Url::parse("https://example.com").unwrap();
    let client = FetchClient::default();
    let links = client.extract_links(&document, &base);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, "https://example.com/ok");
}

#[test]
fn test_extract_links_deduplicates() {
    let html = r#"<html><body>
            <a href="https://example.com/same">First</a>
            <a href="https://example.com/same">Second</a>
        </body></html>"#;
    let document = Html::parse_document(html);
    let base = Url::parse("https://example.com").unwrap();
    let client = FetchClient::default();
    let links = client.extract_links(&document, &base);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, "https://example.com/same");
}

#[test]
fn test_clean_markdown_removes_excessive_blanks() {
    let input = "# Title\n\n\n\nParagraph\n\n\n\n\nAnother paragraph";
    let expected = "# Title\n\nParagraph\n\nAnother paragraph";
    assert_eq!(clean_markdown(input), expected);
}

#[test]
fn test_word_count() {
    assert_eq!(word_count("one two three"), 3);
    assert_eq!(word_count("  spaced   words  "), 2);
    assert_eq!(word_count(""), 0);
}

#[test]
fn test_title_from_url() {
    assert_eq!(
        title_from_url("https://example.com/docs/guide.pdf"),
        "guide.pdf"
    );
    assert_eq!(
        title_from_url("https://example.com/"),
        "https://example.com/"
    );
}

#[test]
fn test_is_skippable_href_hash() {
    assert!(is_skippable_href("#section"));
}

#[test]
fn test_is_skippable_href_javascript() {
    assert!(is_skippable_href("javascript:void(0)"));
}

#[test]
fn test_is_skippable_href_mailto() {
    assert!(is_skippable_href("mailto:a@b.com"));
}

#[test]
fn test_is_skippable_href_tel() {
    assert!(is_skippable_href("tel:+123"));
}

#[test]
fn test_is_skippable_href_full_url() {
    assert!(!is_skippable_href("https://example.com"));
}

#[test]
fn test_resolve_href() {
    let base = Url::parse("https://example.com/page").unwrap();
    assert_eq!(
        resolve_href(&base, "/other").map(|u| u.to_string()),
        Some("https://example.com/other".to_string())
    );
    assert_eq!(
        resolve_href(&base, "relative").map(|u| u.to_string()),
        Some("https://example.com/relative".to_string())
    );
    assert_eq!(resolve_href(&base, "#top"), None);
    assert_eq!(resolve_href(&base, "javascript:alert(1)"), None);
}

#[test]
fn test_extract_with_readability() {
    let words: String = (0..60)
        .map(|i| format!("word{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let html = format!(
        "<html><head><title>Article</title></head><body><article><p>{words}</p></article></body></html>"
    );
    assert!(extract_with_readability(&html, "https://example.com/article").is_some());
    assert!(
        extract_with_readability("<html><body>Hi</body></html>", "https://example.com").is_none()
    );
}

#[test]
fn test_classify_by_inference_pdf() {
    let bytes = include_bytes!("../../../tests/fixtures/minimal.pdf");
    let kind = infer::get(bytes).expect("pdf magic");
    let result = classify_by_inference(&kind, bytes);
    assert!(matches!(result, Some(FetchedContent::Pdf(_))));
}

#[test]
fn test_classify_by_inference_html() {
    let bytes = b"<html><body><p>Hello</p></body></html>";
    let kind = infer::get(bytes).expect("html infer match");
    let result = classify_by_inference(&kind, bytes);
    assert!(matches!(result, Some(FetchedContent::Html(_))));
}

#[test]
fn test_classify_by_inference_binary() {
    let bytes: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46];
    let kind = infer::get(bytes).expect("jpeg magic");
    let result = classify_by_inference(&kind, bytes);
    assert!(matches!(result, Some(FetchedContent::Binary { .. })));
}

#[test]
fn test_classify_by_fallback_html_content_type() {
    let bytes = b"not inferable content";
    let result = classify_by_fallback("text/html", bytes).unwrap();
    assert!(matches!(result, FetchedContent::Html(_)));
}

#[test]
fn test_classify_by_fallback_utf8_text() {
    let bytes = b"plain text without magic bytes";
    let result = classify_by_fallback("", bytes).unwrap();
    assert!(matches!(result, FetchedContent::Html(_)));
}

#[test]
fn test_classify_by_fallback_binary() {
    let bytes: &[u8] = &[0x80, 0x81, 0x82, 0x83];
    let result = classify_by_fallback("", bytes).unwrap();
    assert!(matches!(result, FetchedContent::Binary { .. }));
}

#[test]
fn test_build_page_from_pdf() {
    let page = FetchClient::build_page_from_pdf("  hello world  ", "https://example.com/doc.pdf");
    assert_eq!(page.url, "https://example.com/doc.pdf");
    assert_eq!(page.title, "doc.pdf");
    assert_eq!(page.content, "hello world");
    assert_eq!(page.word_count, 2);
    assert!(page.links.is_none());
}

#[test]
fn test_validate_url_valid() {
    let url = validate_url("https://example.com").unwrap();
    assert_eq!(url.as_str(), "https://example.com/");
}

#[test]
fn test_validate_url_invalid_scheme() {
    let err = validate_url("ftp://example.com").unwrap_err();
    assert!(matches!(err, DaedraError::InvalidArguments(_)));
}

#[test]
fn test_validate_url_malformed() {
    assert!(validate_url("not a url").is_err());
}

#[test]
fn test_is_known_binary_content_type_refactored() {
    assert!(is_known_binary_content_type("image/png"));
    assert!(is_known_binary_content_type("video/mp4"));
    assert!(is_known_binary_content_type("audio/mp3"));
    assert!(is_known_binary_content_type("application/zip"));
    assert!(!is_known_binary_content_type("text/html"));
    assert!(!is_known_binary_content_type("application/json"));
}

#[test]
fn test_build_page_from_html_basic() {
    let html = r#"<html><head><title>Test Page</title></head><body>
            <article><p>Hello world from the test article body content here.</p></article>
        </body></html>"#;
    let client = FetchClient::default();
    let page = client
        .build_page_from_html_for_tests(html, "https://example.com/page", None, false)
        .unwrap();
    assert_eq!(page.title, "Test Page");
    assert_eq!(page.url, "https://example.com/page");
    assert!(page.word_count > 0);
    assert!(!page.content.is_empty());
}

#[test]
fn test_build_page_from_html_with_selector() {
    let html = r#"<html><head><title>Site</title></head><body>
            <div id="sidebar">Sidebar noise should not appear in output.</div>
            <div id="main"><p>Main region only content for selector test case.</p></div>
        </body></html>"#;
    let client = FetchClient::default();
    let page = client
        .build_page_from_html_for_tests(html, "https://example.com", Some("#main"), false)
        .unwrap();
    assert!(page.content.contains("Main region only"));
    assert!(!page.content.contains("Sidebar noise"));
}

#[test]
fn test_build_page_from_html_with_links() {
    let words: String = (0..55)
        .map(|i| format!("word{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let html = format!(
        r#"<html><head><title>Links Page</title></head><body>
            <article><p>{words}</p>
            <a href="https://example.com/one">First Link</a>
            <a href="https://example.com/two">Second Link</a>
            </article></body></html>"#
    );
    let client = FetchClient::default();
    let page = client
        .build_page_from_html_for_tests(&html, "https://example.com", None, false)
        .unwrap();
    assert!(page.word_count >= 50);
    let links = page.links.expect("expected links for long pages");
    assert!(!links.is_empty());
    assert!(links.iter().any(|l| l.url.contains("example.com/one")));
}

#[test]
fn test_build_page_from_html_short_no_links() {
    let html = r#"<html><head><title>Short</title></head><body>
            <p>Brief page.</p>
            <a href="https://example.com/link">Link</a>
        </body></html>"#;
    let client = FetchClient::default();
    let page = client
        .build_page_from_html_for_tests(html, "https://example.com/short", None, false)
        .unwrap();
    assert!(page.word_count < 50);
    assert!(page.links.is_none());
}

#[test]
fn test_check_bot_protection_clean() {
    let html = r#"<html><head><title>Normal Page</title></head><body><p>Hello</p></body></html>"#;
    let client = FetchClient::default();
    assert!(client.check_bot_protection_for_tests(html).is_ok());
}

#[test]
fn test_check_bot_protection_cf_challenge() {
    let html = r#"<html><head><title>Checking</title></head><body>
            <div id="cf-challenge-running"></div>
        </body></html>"#;
    let client = FetchClient::default();
    let err = client.check_bot_protection_for_tests(html).unwrap_err();
    assert!(matches!(err, DaedraError::BotProtectionDetected));
}

#[test]
fn test_check_bot_protection_suspicious_title() {
    let html = r#"<html><head><title>Just a moment...</title></head><body></body></html>"#;
    let client = FetchClient::default();
    let err = client.check_bot_protection_for_tests(html).unwrap_err();
    assert!(matches!(err, DaedraError::BotProtectionDetected));
}

#[test]
fn test_extract_content_with_readability() {
    let words: String = (0..60)
        .map(|i| format!("word{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let html = format!(
        "<html><head><title>Article</title></head><body><article><p>{words}</p></article></body></html>"
    );
    let client = FetchClient::default();
    let content = client
        .extract_content_from_html_for_tests(&html, None, false)
        .unwrap();
    assert!(word_count(&content) >= 50);
    assert!(content.contains("word0"));
}

#[test]
fn test_extract_content_with_selector() {
    let html = r#"<html><body>
            <div class="noise">Ignored sidebar text here.</div>
            <div id="target"><p>Selected fragment only.</p></div>
        </body></html>"#;
    let client = FetchClient::default();
    let content = client
        .extract_content_from_html_for_tests(html, Some("#target"), false)
        .unwrap();
    assert!(content.contains("Selected fragment only"));
    assert!(!content.contains("Ignored sidebar"));
}

#[test]
fn test_extract_content_fallback_to_body() {
    let html = r#"<html><body>
            <div class="wrapper"><p>Body fallback content without article or main tags.</p></div>
        </body></html>"#;
    let client = FetchClient::default();
    let content = client
        .extract_content_from_html_for_tests(html, None, false)
        .unwrap();
    assert!(content.contains("Body fallback content"));
}

#[test]
fn test_select_content_html_with_selector() {
    let html = r#"<html><body>
            <div class="noise">Ignored sidebar text here.</div>
            <div id="target"><p>Selected fragment only.</p></div>
        </body></html>"#;
    let client = FetchClient::default();
    let content = client
        .select_content_html_for_tests(html, "https://example.com", Some("#target"))
        .unwrap();
    assert!(content.contains("Selected fragment only"));
    assert!(!content.contains("Ignored sidebar"));
}

#[test]
fn test_select_content_html_with_readability() {
    let words: String = (0..60)
        .map(|i| format!("word{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let html = format!(
        "<html><head><title>Article</title></head><body><article><p>{words}</p></article></body></html>"
    );
    let client = FetchClient::default();
    let content = client
        .select_content_html_for_tests(&html, "https://example.com/article", None)
        .unwrap();
    assert!(content.contains("word0"));
    assert!(content.contains("<p>"));
}

#[test]
fn test_select_content_html_fallback_short() {
    let html = r#"<html><body><div class="x"></div></body></html>"#;
    let client = FetchClient::default();
    let content = client
        .select_content_html_for_tests(html, "https://example.com", None)
        .unwrap();
    assert!(content.contains("body") || content.contains("<div"));
}

#[test]
fn test_select_content_html_fallback_long() {
    let html = r#"<html><body>
            <div class="wrapper"><p>Body fallback content without article or main tags but enough words here.</p></div>
        </body></html>"#;
    let client = FetchClient::default();
    let content = client
        .select_content_html_for_tests(html, "https://example.com", None)
        .unwrap();
    assert!(content.contains("Body fallback content"));
    assert!(content.contains("wrapper"));
}

#[test]
fn test_has_bot_protection_element_detected() {
    let html = r#"<html><body><div id="cf-challenge-running"></div></body></html>"#;
    let document = Html::parse_document(html);
    assert!(has_bot_protection_element(&document));
}

#[test]
fn test_has_bot_protection_element_clean() {
    let html = r#"<html><head><title>Normal Page</title></head><body><p>Hello</p></body></html>"#;
    let document = Html::parse_document(html);
    assert!(!has_bot_protection_element(&document));
}

#[test]
fn test_has_suspicious_title_detected() {
    let html = r#"<html><head><title>Just a moment...</title></head><body></body></html>"#;
    let document = Html::parse_document(html);
    assert!(has_suspicious_title(&document));
}

#[test]
fn test_has_suspicious_title_clean() {
    let html = r#"<html><head><title>My Page</title></head><body></body></html>"#;
    let document = Html::parse_document(html);
    assert!(!has_suspicious_title(&document));
}

#[test]
fn test_has_suspicious_title_no_title() {
    let html = r#"<html><body><p>No title element</p></body></html>"#;
    let document = Html::parse_document(html);
    assert!(!has_suspicious_title(&document));
}

#[test]
fn test_text_from_selector_title() {
    let html = r#"<html><head><title>Test</title></head><body></body></html>"#;
    let document = Html::parse_document(html);
    assert_eq!(
        text_from_selector(&document, &TITLE_SELECTOR()),
        Some("Test".to_string())
    );
}

#[test]
fn test_text_from_selector_empty() {
    let html = r#"<html><head><title></title></head><body></body></html>"#;
    let document = Html::parse_document(html);
    assert_eq!(text_from_selector(&document, &TITLE_SELECTOR()), None);
}

#[test]
fn test_text_from_selector_missing() {
    let html = r#"<html><body><p>No title</p></body></html>"#;
    let document = Html::parse_document(html);
    assert_eq!(text_from_selector(&document, &TITLE_SELECTOR()), None);
}

#[test]
fn test_classify_inferred_mime_text_html() {
    let bytes = b"<!DOCTYPE html><html><body></body></html>";
    let result = classify_inferred_mime("text/html", bytes);
    assert!(matches!(result, Some(FetchedContent::Html(_))));
}

#[test]
fn test_classify_inferred_mime_application_pdf() {
    let bytes = include_bytes!("../../../tests/fixtures/minimal.pdf");
    let result = classify_inferred_mime("application/pdf", bytes);
    assert!(matches!(result, Some(FetchedContent::Pdf(_))));
}

#[test]
fn test_classify_inferred_mime_text_plain() {
    let bytes = b"plain text content";
    let result = classify_inferred_mime("text/plain", bytes);
    assert!(matches!(result, Some(FetchedContent::Html(_))));
}

#[test]
fn test_classify_inferred_mime_text_csv() {
    let bytes = b"name,value\na,1";
    let result = classify_inferred_mime("text/csv", bytes);
    // CSV now converts to Markdown via anydoc, not the HTML path.
    assert!(matches!(result, Some(FetchedContent::Document { mime, .. }) if mime.contains("csv")));
}

#[test]
fn test_classify_inferred_mime_docx_is_document_not_text() {
    // The docx MIME contains the substring "xml"; the office rule must
    // win so the zip container is not handed back as text.
    let docx_mime = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    let result = classify_inferred_mime(docx_mime, b"PK\x03\x04 not really a docx");
    // The office rule must win over the textual-xml rule: a broken
    // document errors out, it never comes back as raw "text".
    assert!(!matches!(result, Some(FetchedContent::Text { .. })));
    assert!(is_office_content_type(&normalize_content_type(docx_mime)));
}

#[test]
fn test_extract_document_content_rejects_garbage() {
    assert!(extract_document_content(b"PK\x03\x04garbage", "application/epub+zip").is_err());
}

#[test]
fn test_classify_inferred_mime_text_xml() {
    let bytes = b"<?xml version=\"1.0\"?><root/>";
    let result = classify_inferred_mime("text/xml", bytes);
    // XML now arrives verbatim (Text), not through HTML extraction.
    assert!(matches!(result, Some(FetchedContent::Text { .. })));
}

#[test]
fn test_classify_inferred_mime_svg_is_text() {
    let bytes = b"<svg xmlns='http://www.w3.org/2000/svg'/>";
    let result = classify_inferred_mime("image/svg+xml", bytes);
    assert!(matches!(result, Some(FetchedContent::Text { mime, .. }) if mime.contains("svg")));
}

#[test]
fn test_classify_inferred_mime_image_png() {
    let bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let result = classify_inferred_mime("image/png", bytes);
    assert!(matches!(
        result,
        Some(FetchedContent::Binary { mime, .. }) if mime == "image/png"
    ));
}

#[test]
fn test_classify_inferred_mime_application_zip() {
    let bytes: &[u8] = &[0x50, 0x4B, 0x03, 0x04];
    let result = classify_inferred_mime("application/zip", bytes);
    assert!(matches!(
        result,
        Some(FetchedContent::Binary { mime, .. }) if mime == "application/zip"
    ));
}

#[test]
fn test_classify_inferred_mime_application_octet_stream() {
    let bytes: &[u8] = &[0x00, 0x01, 0x02, 0x03];
    let result = classify_inferred_mime("application/octet-stream", bytes);
    assert!(matches!(
        result,
        Some(FetchedContent::Binary { mime, .. }) if mime == "application/octet-stream"
    ));
}

#[test]
fn test_classify_inferred_mime_audio_mpeg() {
    let bytes: &[u8] = &[0xFF, 0xFB, 0x90, 0x00];
    let result = classify_inferred_mime("audio/mpeg", bytes);
    assert!(matches!(
        result,
        Some(FetchedContent::Binary { mime, .. }) if mime == "audio/mpeg"
    ));
}
