//! Page fetching and content extraction implementation.
//!
//! This module provides functionality to fetch web pages and extract
//! their content as Markdown.

use crate::types::{DaedraError, DaedraResult, PageContent, PageLink, VisitPageArgs};
use backoff::{ExponentialBackoff, future::retry};
use dom_smoothie::Readability;
use lazy_static::lazy_static;
use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use std::collections::HashSet;
use std::time::Duration;
use tracing::{error, info, instrument, warn};
use url::Url;

/// Default user agent for requests
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// Request timeout
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum content size (10MB)
const MAX_CONTENT_SIZE: usize = 10 * 1024 * 1024;

lazy_static! {
    // Content selectors in order of preference
    static ref CONTENT_SELECTORS: Vec<Selector> = vec![
        Selector::parse("main").unwrap(),
        Selector::parse("article").unwrap(),
        Selector::parse("[role='main']").unwrap(),
        Selector::parse("#content").unwrap(),
        Selector::parse(".content").unwrap(),
        Selector::parse(".main").unwrap(),
        Selector::parse(".post").unwrap(),
        Selector::parse(".article").unwrap(),
        Selector::parse(".entry-content").unwrap(),
        Selector::parse(".post-content").unwrap(),
    ];

    // Elements to remove from content
    static ref REMOVE_SELECTORS: Vec<Selector> = vec![
        Selector::parse("script").unwrap(),
        Selector::parse("style").unwrap(),
        Selector::parse("noscript").unwrap(),
        Selector::parse("header").unwrap(),
        Selector::parse("footer").unwrap(),
        Selector::parse("nav").unwrap(),
        Selector::parse("[role='navigation']").unwrap(),
        Selector::parse("aside").unwrap(),
        Selector::parse(".sidebar").unwrap(),
        Selector::parse("[role='complementary']").unwrap(),
        Selector::parse(".nav").unwrap(),
        Selector::parse(".menu").unwrap(),
        Selector::parse(".header").unwrap(),
        Selector::parse(".footer").unwrap(),
        Selector::parse(".advertisement").unwrap(),
        Selector::parse(".ads").unwrap(),
        Selector::parse(".ad").unwrap(),
        Selector::parse(".cookie-notice").unwrap(),
        Selector::parse(".cookie-banner").unwrap(),
        Selector::parse(".popup").unwrap(),
        Selector::parse(".modal").unwrap(),
        Selector::parse("[class*='cookie']").unwrap(),
        Selector::parse("[class*='banner']").unwrap(),
        Selector::parse("[class*='social']").unwrap(),
        Selector::parse("[class*='share']").unwrap(),
        Selector::parse("[class*='comment']").unwrap(),
    ];

    // Title selector
    static ref TITLE_SELECTOR: Selector = Selector::parse("title").unwrap();

    // Link selector
    static ref LINK_SELECTOR: Selector = Selector::parse("a[href]").unwrap();

    // Heading selectors
    static ref H1_SELECTOR: Selector = Selector::parse("h1").unwrap();
    static ref H2_SELECTOR: Selector = Selector::parse("h2").unwrap();
    static ref H3_SELECTOR: Selector = Selector::parse("h3").unwrap();
    static ref H4_SELECTOR: Selector = Selector::parse("h4").unwrap();
    static ref H5_SELECTOR: Selector = Selector::parse("h5").unwrap();
    static ref H6_SELECTOR: Selector = Selector::parse("h6").unwrap();

    // Paragraph selector
    static ref P_SELECTOR: Selector = Selector::parse("p").unwrap();

    // List selectors
    static ref UL_SELECTOR: Selector = Selector::parse("ul").unwrap();
    static ref OL_SELECTOR: Selector = Selector::parse("ol").unwrap();
    static ref LI_SELECTOR: Selector = Selector::parse("li").unwrap();

    // Code selectors
    static ref PRE_SELECTOR: Selector = Selector::parse("pre").unwrap();
    static ref CODE_SELECTOR: Selector = Selector::parse("code").unwrap();

    // Image selector
    static ref IMG_SELECTOR: Selector = Selector::parse("img").unwrap();

    // Blockquote selector
    static ref BLOCKQUOTE_SELECTOR: Selector = Selector::parse("blockquote").unwrap();

    // Bot protection indicators
    static ref BOT_PROTECTION_SELECTORS: Vec<Selector> = vec![
        Selector::parse("#challenge-running").unwrap(),
        Selector::parse("#cf-challenge-running").unwrap(),
        Selector::parse("#px-captcha").unwrap(),
        Selector::parse("#ddos-protection").unwrap(),
        Selector::parse("#waf-challenge-html").unwrap(),
        Selector::parse(".cf-browser-verification").unwrap(),
    ];
}

/// Suspicious page titles that indicate bot protection
const SUSPICIOUS_TITLES: &[&str] = &[
    "security check",
    "ddos protection",
    "please wait",
    "just a moment",
    "attention required",
    "access denied",
    "blocked",
    "captcha",
    "verify you are human",
];

/// Raw content returned from an HTTP fetch
enum FetchedContent {
    Html(String),
    Pdf(String),
    Binary { mime: String, size: usize },
}

/// Returns true for hrefs that should be skipped (#, javascript:, mailto:, tel:).
fn is_skippable_href(href: &str) -> bool {
    href.starts_with('#')
        || href.starts_with("javascript:")
        || href.starts_with("mailto:")
        || href.starts_with("tel:")
}

/// Resolve a relative href against a base URL, returning None for skippable or unresolvable.
fn resolve_href(base: &Url, href: &str) -> Option<Url> {
    if is_skippable_href(href) {
        return None;
    }
    base.join(href).ok()
}

/// Clean up link text: collapse whitespace, skip empty/short text.
fn normalize_link_text(element: &ElementRef<'_>) -> Option<String> {
    let text: String = element
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() || text.len() <= 2 {
        None
    } else {
        Some(text)
    }
}

/// HTTP client for fetching pages
#[derive(Clone)]
pub struct FetchClient {
    client: Client,
}

impl FetchClient {
    /// Create a new fetch client
    pub fn new() -> DaedraResult<Self> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .gzip(true)
            .brotli(true)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(DaedraError::HttpError)?;

        Ok(Self { client })
    }

    /// Fetch and extract content from a URL
    #[instrument(skip(self), fields(url = %args.url))]
    pub async fn fetch(&self, args: &VisitPageArgs) -> DaedraResult<PageContent> {
        info!(url = %args.url, "Fetching page");

        let parsed_url = validate_url(&args.url)?;
        let fetched = self.fetch_with_retry(&args.url).await?;

        match fetched {
            FetchedContent::Html(html) => {
                self.build_page_from_html(&html, &args.url, &parsed_url, args.selector.as_deref())
            }
            FetchedContent::Pdf(text) => Ok(FetchClient::build_page_from_pdf(&text, &args.url)),
            FetchedContent::Binary { mime, size } => Err(DaedraError::ExtractionError(format!(
                "Unsupported content type: {mime} ({size} bytes)"
            ))),
        }
    }

    fn build_page_from_html(
        &self,
        html: &str,
        url: &str,
        base_url: &Url,
        selector: Option<&str>,
    ) -> DaedraResult<PageContent> {
        let document = Html::parse_document(html);

        self.check_bot_protection(&document)?;

        let title = self.extract_title(&document);
        let content = self.extract_content(html, &document, url, selector)?;

        let word_count = word_count(&content);

        let links = if word_count >= 50 {
            Some(self.extract_links(&document, base_url))
        } else {
            None
        };

        info!(
            url = %url,
            title = %title,
            word_count = word_count,
            "Page fetched successfully"
        );

        Ok(PageContent {
            url: url.to_string(),
            title,
            content,
            timestamp: chrono::Utc::now().to_rfc3339(),
            word_count,
            links,
        })
    }

    fn build_page_from_pdf(text: &str, url: &str) -> PageContent {
        let content = text.trim().to_string();
        let word_count = word_count(&content);
        let title = title_from_url(url);

        info!(
            url = %url,
            title = %title,
            word_count = word_count,
            "PDF fetched successfully"
        );

        PageContent {
            url: url.to_string(),
            title,
            content,
            timestamp: chrono::Utc::now().to_rfc3339(),
            word_count,
            links: None,
        }
    }

    /// Fetch page content with retry logic
    async fn fetch_with_retry(&self, url: &str) -> DaedraResult<FetchedContent> {
        let backoff = ExponentialBackoff {
            max_elapsed_time: Some(Duration::from_secs(60)),
            ..Default::default()
        };

        let client = self.client.clone();
        let url = url.to_string();

        retry(backoff, || async {
            let response = client.get(&url).send().await.map_err(|e| {
                warn!(error = %e, url = %url, "Fetch request failed, retrying...");
                backoff::Error::transient(DaedraError::HttpError(e))
            })?;

            classify_response_status(response.status(), &url)?;

            if let Some(content_length) = response.content_length()
                && content_length as usize > MAX_CONTENT_SIZE
            {
                return Err(backoff::Error::permanent(DaedraError::FetchError(
                    "Content too large".to_string(),
                )));
            }

            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            let ct = normalize_content_type(&content_type);

            if ct.contains("application/pdf") {
                let bytes = response.bytes().await.map_err(|e| {
                    error!(error = %e, url = %url, "Failed to read response body");
                    backoff::Error::permanent(DaedraError::HttpError(e))
                })?;
                check_body_size(bytes.len())?;
                return Ok(extract_pdf_content(&bytes)?);
            }

            if is_known_binary_content_type(&ct) {
                let bytes = response.bytes().await.map_err(|e| {
                    error!(error = %e, url = %url, "Failed to read response body");
                    backoff::Error::permanent(DaedraError::HttpError(e))
                })?;
                check_body_size(bytes.len())?;
                return Ok(FetchedContent::Binary {
                    mime: ct,
                    size: bytes.len(),
                });
            }

            let bytes = response.bytes().await.map_err(|e| {
                error!(error = %e, url = %url, "Failed to read response body");
                backoff::Error::permanent(DaedraError::HttpError(e))
            })?;
            check_body_size(bytes.len())?;

            classify_fetched_content(&content_type, &bytes).map_err(|e| backoff::Error::permanent(e))
        })
        .await
    }

    /// Check for bot protection indicators
    fn check_bot_protection(&self, document: &Html) -> DaedraResult<()> {
        if has_bot_protection_element(document) || has_suspicious_title(document) {
            return Err(DaedraError::BotProtectionDetected);
        }
        Ok(())
    }

    /// Extract page title
    fn extract_title(&self, document: &Html) -> String {
        text_from_selector(document, &TITLE_SELECTOR)
            .or_else(|| text_from_selector(document, &H1_SELECTOR))
            .unwrap_or_else(|| "Untitled".to_string())
    }

    fn select_content_html(
        &self,
        html: &str,
        document: &Html,
        url: &str,
        selector: Option<&str>,
    ) -> DaedraResult<String> {
        if let Some(sel) = selector {
            Ok(self
                .select_html_fragment(document, sel)?
                .unwrap_or_else(|| self.select_body_html(document)))
        } else if let Some(readability_html) = extract_with_readability(html, url) {
            Ok(readability_html)
        } else {
            let fragment = self
                .select_first_content_selector(document)
                .unwrap_or_else(|| self.select_body_html(document));
            let preview = clean_markdown(&html_to_markdown(&fragment));
            if word_count(&preview) < 10 {
                Ok(self.select_body_html(document))
            } else {
                Ok(fragment)
            }
        }
    }

    /// Extract and convert content to Markdown
    fn extract_content(
        &self,
        html: &str,
        document: &Html,
        url: &str,
        selector: Option<&str>,
    ) -> DaedraResult<String> {
        let content_html = self.select_content_html(html, document, url, selector)?;
        let markdown = html_to_markdown(&content_html);
        let cleaned = clean_markdown(&markdown);

        if word_count(&cleaned) < 10 {
            warn!("Extracted content is very short");
        }

        Ok(cleaned)
    }

    fn select_html_fragment(&self, document: &Html, sel: &str) -> DaedraResult<Option<String>> {
        let custom_selector = Selector::parse(sel).map_err(|_| {
            DaedraError::InvalidArguments(format!("Invalid CSS selector: {}", sel))
        })?;

        Ok(document
            .select(&custom_selector)
            .next()
            .map(|el| el.html()))
    }

    fn select_first_content_selector(&self, document: &Html) -> Option<String> {
        for selector in CONTENT_SELECTORS.iter() {
            if let Some(element) = document.select(selector).next() {
                return Some(element.html());
            }
        }
        None
    }

    fn select_body_html(&self, document: &Html) -> String {
        document
            .select(&Selector::parse("body").unwrap())
            .next()
            .map(|el| el.html())
            .unwrap_or_default()
    }

    /// Extract links from the page
    fn extract_links(&self, document: &Html, base_url: &Url) -> Vec<PageLink> {
        let mut links = Vec::new();
        let mut seen_urls = HashSet::new();

        for element in document.select(&LINK_SELECTOR) {
            let Some(href) = element.value().attr("href") else {
                continue;
            };
            let Some(resolved) = resolve_href(base_url, href) else {
                continue;
            };
            if !seen_urls.insert(resolved.to_string()) {
                continue;
            }
            let Some(text) = normalize_link_text(&element) else {
                continue;
            };
            links.push(PageLink {
                text,
                url: resolved.to_string(),
            });
        }

        links.truncate(50);
        links
    }
}

impl Default for FetchClient {
    fn default() -> Self {
        Self::new().expect("Failed to create default fetch client")
    }
}


fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn validate_url(url: &str) -> DaedraResult<Url> {
    let parsed_url = Url::parse(url).map_err(DaedraError::UrlParseError)?;

    if !matches!(parsed_url.scheme(), "http" | "https") {
        return Err(DaedraError::InvalidArguments(
            "Only HTTP(S) URLs are supported".to_string(),
        ));
    }

    Ok(parsed_url)
}

fn is_retryable_status(status: u16) -> bool {
    status == 429
}

fn classify_response_status(
    status: reqwest::StatusCode,
    url: &str,
) -> Result<(), backoff::Error<DaedraError>> {
    if status.is_success() {
        return Ok(());
    }

    warn!(status = %status, url = %url, "Fetch returned non-success status");

    if is_retryable_status(status.as_u16()) {
        return Err(backoff::Error::transient(DaedraError::RateLimitExceeded));
    }

    if status.as_u16() == 403 {
        return Err(backoff::Error::permanent(DaedraError::BotProtectionDetected));
    }

    Err(backoff::Error::permanent(DaedraError::FetchError(format!(
        "HTTP {}",
        status
    ))))
}

fn normalize_content_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_lowercase()
}

const BINARY_CONTENT_PREFIXES: &[&str] = &[
    "image/",
    "video/",
    "audio/",
    "application/vnd.openxmlformats-",
];

const BINARY_CONTENT_EXACT: &[&str] = &[
    "application/zip",
    "application/gzip",
    "application/x-tar",
    "application/octet-stream",
    "application/vnd.ms-excel",
];

fn is_known_binary_content_type(content_type: &str) -> bool {
    let ct = normalize_content_type(content_type);
    BINARY_CONTENT_PREFIXES
        .iter()
        .any(|prefix| ct.starts_with(prefix))
        || BINARY_CONTENT_EXACT.iter().any(|exact| ct == *exact)
}

fn is_binary_mime(mime: &str) -> bool {
    is_known_binary_content_type(mime)
        || mime == "application/pdf"
        || mime.starts_with("application/vnd.")
}

fn bytes_to_utf8_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn check_body_size(size: usize) -> DaedraResult<()> {
    if size > MAX_CONTENT_SIZE {
        return Err(DaedraError::FetchError("Content too large".to_string()));
    }
    Ok(())
}

fn extract_pdf_content(bytes: &[u8]) -> DaedraResult<FetchedContent> {
    let text = pdf_extract::extract_text_from_mem(bytes)
        .map_err(|e| DaedraError::ExtractionError(e.to_string()))?;
    Ok(FetchedContent::Pdf(text))
}

fn has_bot_protection_element(document: &Html) -> bool {
    BOT_PROTECTION_SELECTORS
        .iter()
        .any(|s| document.select(s).next().is_some())
}

fn has_suspicious_title(document: &Html) -> bool {
    document
        .select(&TITLE_SELECTOR)
        .next()
        .map_or(false, |el| {
            let title = el.text().collect::<String>().to_lowercase();
            SUSPICIOUS_TITLES.iter().any(|s| title.contains(s))
        })
}

fn text_from_selector(document: &Html, selector: &Selector) -> Option<String> {
    document
        .select(selector)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|t| !t.is_empty())
        .map(|t| clean_title(&t))
}

fn classify_inferred_mime(mime: &str, bytes: &[u8]) -> Option<FetchedContent> {
    match mime {
        "application/pdf" => extract_pdf_content(bytes).ok(),
        "text/html" | "application/xhtml+xml" => Some(FetchedContent::Html(bytes_to_utf8_string(bytes))),
        m if is_binary_mime(m) => Some(FetchedContent::Binary {
            mime: m.to_string(),
            size: bytes.len(),
        }),
        m if m.starts_with("text/") => Some(FetchedContent::Html(bytes_to_utf8_string(bytes))),
        _ => None,
    }
}

fn classify_by_inference(kind: &infer::Type, bytes: &[u8]) -> Option<FetchedContent> {
    classify_inferred_mime(kind.mime_type(), bytes)
}

fn classify_by_fallback(content_type: &str, bytes: &[u8]) -> DaedraResult<FetchedContent> {
    let ct = normalize_content_type(content_type);
    if ct.contains("text/html") {
        return Ok(FetchedContent::Html(bytes_to_utf8_string(bytes)));
    }

    if std::str::from_utf8(bytes).is_ok() {
        return Ok(FetchedContent::Html(bytes_to_utf8_string(bytes)));
    }

    Ok(FetchedContent::Binary {
        mime: if ct.is_empty() {
            "application/octet-stream".to_string()
        } else {
            ct
        },
        size: bytes.len(),
    })
}

fn classify_fetched_content(content_type: &str, bytes: &[u8]) -> DaedraResult<FetchedContent> {
    if let Some(kind) = infer::get(bytes) {
        if let Some(content) = classify_by_inference(&kind, bytes) {
            return Ok(content);
        }
        if kind.mime_type() == "application/pdf" {
            return extract_pdf_content(bytes);
        }
    }

    classify_by_fallback(content_type, bytes)
}

fn extract_with_readability(html: &str, url: &str) -> Option<String> {
    let document_url = if url.is_empty() { None } else { Some(url) };
    let mut readability = Readability::new(html, document_url, None).ok()?;
    let article = readability.parse().ok()?;
    if word_count(&article.text_content) >= 50 {
        Some(article.content.to_string())
    } else {
        None
    }
}

fn title_from_url(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .path_segments()
                .and_then(|segments| segments.filter(|s| !s.is_empty()).last())
                .map(str::to_string)
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| url.to_string())
}

/// Fetch a page and extract its content
///
/// # Arguments
///
/// * `args` - Fetch arguments including URL and optional selector
///
/// # Returns
///
/// Extracted page content as `PageContent`
///
/// # Example
///
/// ```rust,no_run
/// use daedra::{VisitPageArgs, tools::fetch::fetch_page};
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let args = VisitPageArgs {
///         url: "https://example.com".to_string(),
///         selector: None,
///         include_images: false,
///     };
///     let content = fetch_page(&args).await?;
///     println!("Title: {}", content.title);
///     Ok(())
/// }
/// ```
pub async fn fetch_page(args: &VisitPageArgs) -> DaedraResult<PageContent> {
    let client = FetchClient::new()?;
    client.fetch(args).await
}

/// Validate that a URL is safe to fetch
pub fn is_valid_url(url: &str) -> bool {
    match Url::parse(url) {
        Ok(parsed) => matches!(parsed.scheme(), "http" | "https"),
        Err(_) => false,
    }
}

/// Convert HTML to Markdown
fn html_to_markdown(html: &str) -> String {
    // Use htmd crate for conversion
    htmd::convert(html).unwrap_or_else(|_| html.to_string())
}

/// Clean up Markdown content
fn clean_markdown(markdown: &str) -> String {
    markdown
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            trimmed != "-" && trimmed != "*" && trimmed != "+"
        })
        .fold(String::new(), |mut acc, line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                if !acc.ends_with("\n\n") {
                    acc.push('\n');
                }
            } else {
                acc.push_str(trimmed);
                acc.push('\n');
            }
            acc
        })
        .trim()
        .to_string()
}

/// Clean up a page title
fn clean_title(title: &str) -> String {
    // Remove common suffixes
    let title = title
        .split(" | ")
        .next()
        .unwrap_or(title)
        .split(" - ")
        .next()
        .unwrap_or(title)
        .split(" :: ")
        .next()
        .unwrap_or(title)
        .split(" — ")
        .next()
        .unwrap_or(title);

    title.trim().to_string()
}

#[cfg(test)]
impl FetchClient {
    /// Same extraction path as [`FetchClient::fetch`] but without HTTP (integration fixtures).
    pub fn extract_content_from_html_for_tests(
        &self,
        html: &str,
        selector: Option<&str>,
    ) -> DaedraResult<String> {
        let document = Html::parse_document(html);
        self.extract_content(html, &document, "", selector)
    }

    /// Same path as [`FetchClient::build_page_from_html`] without HTTP.
    pub fn build_page_from_html_for_tests(
        &self,
        html: &str,
        url: &str,
        selector: Option<&str>,
    ) -> DaedraResult<PageContent> {
        let parsed_url = validate_url(url)?;
        self.build_page_from_html(html, url, &parsed_url, selector)
    }

    /// Exposes bot-protection checks for unit tests.
    pub fn check_bot_protection_for_tests(&self, html: &str) -> DaedraResult<()> {
        let document = Html::parse_document(html);
        self.check_bot_protection(&document)
    }

    /// Exposes content HTML selection for unit tests.
    pub fn select_content_html_for_tests(
        &self,
        html: &str,
        url: &str,
        selector: Option<&str>,
    ) -> DaedraResult<String> {
        let document = Html::parse_document(html);
        self.select_content_html(html, &document, url, selector)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELIACHIA_FIXTURE: &str = include_str!("../../tests/fixtures/celiachia.html");
    const CELIACHIA_ARTICLE_MARKER: &str = "indagine 2023 su";

    #[test]
    #[ignore = "bug #6 fixed: dom_smoothie now extracts full article"]
    fn characterization_issue_6_celiachia_extract_content_low_word_count() {
        let client = FetchClient::default();
        let content = client
            .extract_content_from_html_for_tests(CELIACHIA_FIXTURE, None)
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
            .extract_content_from_html_for_tests(CELIACHIA_FIXTURE, None)
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
        assert_eq!(clean_markdown(input), "# Heading\n\nParagraph with **bold** text.");
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
        let bytes = include_bytes!("../../tests/fixtures/minimal.pdf");
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
        assert_eq!(title_from_url("https://example.com/"), "https://example.com/");
    }

    #[test]
    fn test_is_skippable_href() {
        assert!(is_skippable_href("#section"));
        assert!(is_skippable_href("javascript:void(0)"));
        assert!(is_skippable_href("mailto:a@b.com"));
        assert!(is_skippable_href("tel:+123"));
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
        assert!(extract_with_readability("<html><body>Hi</body></html>", "https://example.com").is_none());
    }

    #[test]
    fn test_classify_by_inference_pdf() {
        let bytes = include_bytes!("../../tests/fixtures/minimal.pdf");
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
            .build_page_from_html_for_tests(html, "https://example.com/page", None)
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
            .build_page_from_html_for_tests(html, "https://example.com", Some("#main"))
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
            .build_page_from_html_for_tests(&html, "https://example.com", None)
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
            .build_page_from_html_for_tests(html, "https://example.com/short", None)
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
            .extract_content_from_html_for_tests(&html, None)
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
            .extract_content_from_html_for_tests(html, Some("#target"))
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
            .extract_content_from_html_for_tests(html, None)
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
            text_from_selector(&document, &TITLE_SELECTOR),
            Some("Test".to_string())
        );
    }

    #[test]
    fn test_text_from_selector_empty() {
        let html = r#"<html><head><title></title></head><body></body></html>"#;
        let document = Html::parse_document(html);
        assert_eq!(text_from_selector(&document, &TITLE_SELECTOR), None);
    }

    #[test]
    fn test_text_from_selector_missing() {
        let html = r#"<html><body><p>No title</p></body></html>"#;
        let document = Html::parse_document(html);
        assert_eq!(text_from_selector(&document, &TITLE_SELECTOR), None);
    }

    #[test]
    fn test_classify_inferred_mime_html() {
        let bytes = b"<!DOCTYPE html><html><body></body></html>";
        let result = classify_inferred_mime("text/html", bytes);
        assert!(matches!(result, Some(FetchedContent::Html(_))));
    }

    #[test]
    fn test_classify_inferred_mime_pdf() {
        let bytes = include_bytes!("../../tests/fixtures/minimal.pdf");
        let result = classify_inferred_mime("application/pdf", bytes);
        assert!(matches!(result, Some(FetchedContent::Pdf(_))));
    }

    #[test]
    fn test_classify_inferred_mime_text() {
        let bytes = b"plain text content";
        let result = classify_inferred_mime("text/plain", bytes);
        assert!(matches!(result, Some(FetchedContent::Html(_))));
    }

    #[test]
    fn test_classify_inferred_mime_binary() {
        let bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let result = classify_inferred_mime("image/png", bytes);
        assert!(matches!(
            result,
            Some(FetchedContent::Binary { mime, .. }) if mime == "image/png"
        ));
    }

    #[test]
    fn test_classify_inferred_mime_octet_stream() {
        let bytes: &[u8] = &[0x00, 0x01, 0x02, 0x03];
        let result = classify_inferred_mime("application/octet-stream", bytes);
        assert!(matches!(
            result,
            Some(FetchedContent::Binary { mime, .. }) if mime == "application/octet-stream"
        ));
    }


}
