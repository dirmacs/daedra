//! Page fetching and content extraction implementation.
//!
//! This module provides functionality to fetch web pages and extract
//! their content as Markdown.

use crate::types::{DaedraError, DaedraResult, PageContent, PageLink, VisitPageArgs};
use backoff::{ExponentialBackoff, future::retry};
use dom_smoothie::Readability;
use lazy_static::lazy_static;
use reqwest::Client;
use scraper::{Html, Selector};
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

        // Validate URL
        let parsed_url = Url::parse(&args.url).map_err(DaedraError::UrlParseError)?;

        if !matches!(parsed_url.scheme(), "http" | "https") {
            return Err(DaedraError::InvalidArguments(
                "Only HTTP(S) URLs are supported".to_string(),
            ));
        }

        let fetched = self.fetch_with_retry(&args.url).await?;
        let timestamp = chrono::Utc::now().to_rfc3339();

        match fetched {
            FetchedContent::Html(html) => {
                let document = Html::parse_document(&html);

                self.check_bot_protection(&document)?;

                let title = self.extract_title(&document);
                let content = self.extract_content(
                    &html,
                    &document,
                    &args.url,
                    args.selector.as_deref(),
                )?;

                let word_count = word_count(&content);

                let links = if word_count >= 50 {
                    Some(self.extract_links(&document, &parsed_url))
                } else {
                    None
                };

                info!(
                    url = %args.url,
                    title = %title,
                    word_count = word_count,
                    "Page fetched successfully"
                );

                Ok(PageContent {
                    url: args.url.clone(),
                    title,
                    content,
                    timestamp,
                    word_count,
                    links,
                })
            }
            FetchedContent::Pdf(text) => {
                let content = text.trim().to_string();
                let word_count = word_count(&content);
                let title = title_from_url(&args.url);

                info!(
                    url = %args.url,
                    title = %title,
                    word_count = word_count,
                    "PDF fetched successfully"
                );

                Ok(PageContent {
                    url: args.url.clone(),
                    title,
                    content,
                    timestamp,
                    word_count,
                    links: None,
                })
            }
            FetchedContent::Binary { mime, size } => Err(DaedraError::ExtractionError(format!(
                "Unsupported content type: {mime} ({size} bytes)"
            ))),
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

            let status = response.status();

            if !status.is_success() {
                warn!(status = %status, url = %url, "Fetch returned non-success status");

                if status.as_u16() == 429 {
                    return Err(backoff::Error::transient(DaedraError::RateLimitExceeded));
                }

                if status.as_u16() == 403 {
                    return Err(backoff::Error::permanent(
                        DaedraError::BotProtectionDetected,
                    ));
                }

                return Err(backoff::Error::permanent(DaedraError::FetchError(format!(
                    "HTTP {}",
                    status
                ))));
            }

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
        // Check for bot protection elements
        for selector in BOT_PROTECTION_SELECTORS.iter() {
            if document.select(selector).next().is_some() {
                return Err(DaedraError::BotProtectionDetected);
            }
        }

        // Check for suspicious titles
        if let Some(title_element) = document.select(&TITLE_SELECTOR).next() {
            let title = title_element.text().collect::<String>().to_lowercase();
            for suspicious in SUSPICIOUS_TITLES {
                if title.contains(suspicious) {
                    return Err(DaedraError::BotProtectionDetected);
                }
            }
        }

        Ok(())
    }

    /// Extract page title
    fn extract_title(&self, document: &Html) -> String {
        // Try <title> tag first
        if let Some(title_element) = document.select(&TITLE_SELECTOR).next() {
            let title = title_element.text().collect::<String>().trim().to_string();
            if !title.is_empty() {
                return clean_title(&title);
            }
        }

        // Fall back to first h1
        if let Some(h1_element) = document.select(&H1_SELECTOR).next() {
            let title = h1_element.text().collect::<String>().trim().to_string();
            if !title.is_empty() {
                return clean_title(&title);
            }
        }

        "Untitled".to_string()
    }

    /// Extract and convert content to Markdown
    fn extract_content(
        &self,
        html: &str,
        document: &Html,
        url: &str,
        selector: Option<&str>,
    ) -> DaedraResult<String> {
        let content_html = if let Some(sel) = selector {
            self.select_html_fragment(document, sel)?
                .unwrap_or_else(|| self.select_body_html(document))
        } else if let Some(readability_html) = extract_with_readability(html, url) {
            readability_html
        } else {
            let fragment = self
                .select_first_content_selector(document)
                .unwrap_or_else(|| self.select_body_html(document));
            let preview = clean_markdown(&html_to_markdown(&fragment));
            if word_count(&preview) < 10 {
                self.select_body_html(document)
            } else {
                fragment
            }
        };

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
        let mut seen_urls = std::collections::HashSet::new();

        for element in document.select(&LINK_SELECTOR) {
            let href = match element.value().attr("href") {
                Some(h) => h,
                None => continue,
            };

            // Resolve relative URLs
            let resolved_url = match base_url.join(href) {
                Ok(url) => url.to_string(),
                Err(_) => continue,
            };

            // Skip duplicates, anchors, and non-http(s) URLs
            if seen_urls.contains(&resolved_url)
                || href.starts_with('#')
                || href.starts_with("javascript:")
                || href.starts_with("mailto:")
                || href.starts_with("tel:")
            {
                continue;
            }

            seen_urls.insert(resolved_url.clone());

            let text = element
                .text()
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");

            // Only include links with meaningful text
            if !text.is_empty() && text.len() > 2 {
                links.push(PageLink {
                    text,
                    url: resolved_url,
                });
            }
        }

        // Limit to first 50 links
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

fn normalize_content_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_lowercase()
}

fn is_known_binary_content_type(content_type: &str) -> bool {
    let ct = normalize_content_type(content_type);
    ct.starts_with("image/")
        || ct.starts_with("video/")
        || ct.starts_with("audio/")
        || ct == "application/zip"
        || ct == "application/gzip"
        || ct == "application/x-tar"
        || ct == "application/octet-stream"
        || ct == "application/vnd.ms-excel"
        || ct.starts_with("application/vnd.openxmlformats-")
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

fn classify_fetched_content(content_type: &str, bytes: &[u8]) -> DaedraResult<FetchedContent> {
    if let Some(kind) = infer::get(bytes) {
        let mime = kind.mime_type();
        if mime == "application/pdf" {
            return extract_pdf_content(bytes);
        }
        if mime == "text/html" || mime == "application/xhtml+xml" {
            return Ok(FetchedContent::Html(bytes_to_utf8_string(bytes)));
        }
        if is_binary_mime(mime) {
            return Ok(FetchedContent::Binary {
                mime: mime.to_string(),
                size: bytes.len(),
            });
        }
        if mime.starts_with("text/") {
            return Ok(FetchedContent::Html(bytes_to_utf8_string(bytes)));
        }
    }

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
    let lines: Vec<&str> = markdown.lines().collect();

    // Remove excessive blank lines
    let mut result = String::new();
    let mut prev_blank = false;

    for line in lines.iter() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            if !prev_blank {
                result.push('\n');
                prev_blank = true;
            }
        } else {
            // Skip lines that are just list markers
            if trimmed == "-" || trimmed == "*" || trimmed == "+" {
                continue;
            }

            result.push_str(trimmed);
            result.push('\n');
            prev_blank = false;
        }
    }

    result.trim().to_string()
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
    fn test_html_to_markdown() {
        let html = "<h1>Title</h1><p>Paragraph with <strong>bold</strong> text.</p>";
        let markdown = html_to_markdown(html);
        assert!(markdown.contains("Title"));
        assert!(markdown.contains("Paragraph"));
        assert!(markdown.contains("bold"));
    }
}
