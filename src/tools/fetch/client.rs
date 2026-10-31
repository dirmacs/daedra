//! HTTP fetch client, retry loop, and the public fetch entry points.

use crate::types::{DaedraError, DaedraResult, PageContent, PageLink, VisitPageArgs};
use dom_smoothie::Readability;
use lazy_static::lazy_static;
use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tracing::{error, info, instrument, warn};
use url::Url;

use super::extract::{
    classify_fetched_content, extract_pdf_content, is_container_mime, is_office_content_type,
    is_textual_content_type,
};
use super::markdown::{clean_markdown, clean_title, html_to_markdown, strip_img_tags};
use super::mime::{
    check_body_size, is_known_binary_content_type, normalize_content_type, validate_url,
};

pub(super) enum RetryErr {
    Transient(DaedraError),
    Permanent(DaedraError),
}

impl From<DaedraError> for RetryErr {
    fn from(e: DaedraError) -> Self {
        RetryErr::Permanent(e)
    }
}

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
pub(super) enum FetchedContent {
    Html(String),
    Pdf(String),
    /// Office/ ebook documents (docx, doc, odt, rtf, epub, pptx, xlsx,
    /// csv) converted to Markdown by `anydoc`.
    Document {
        mime: String,
        markdown: String,
    },
    /// Textual non-HTML content (SVG, XML, JSON...) delivered verbatim.
    Text {
        mime: String,
        body: String,
    },
    Binary {
        mime: String,
        size: usize,
    },
}

/// Returns true for hrefs that should be skipped (#, javascript:, mailto:, tel:).
pub(super) fn is_skippable_href(href: &str) -> bool {
    href.starts_with('#')
        || href.starts_with("javascript:")
        || href.starts_with("mailto:")
        || href.starts_with("tel:")
}

/// Resolve a relative href against a base URL, returning None for skippable or unresolvable.
pub(super) fn resolve_href(base: &Url, href: &str) -> Option<Url> {
    if is_skippable_href(href) {
        return None;
    }
    base.join(href).ok()
}

/// Clean up link text: collapse whitespace, skip empty/short text.
pub(super) fn normalize_link_text(element: &ElementRef<'_>) -> Option<String> {
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
        Self::with_timeout(REQUEST_TIMEOUT)
    }

    /// Create a fetch client with a custom per-request timeout.
    pub fn with_timeout(timeout: Duration) -> DaedraResult<Self> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(timeout)
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
            FetchedContent::Html(html) => self.build_page_from_html(
                &html,
                &args.url,
                &parsed_url,
                args.selector.as_deref(),
                args.include_images,
            ),
            FetchedContent::Pdf(text) => Ok(FetchClient::build_page_from_pdf(&text, &args.url)),
            FetchedContent::Document { mime, markdown } => {
                Ok(Self::build_page_from_document(&markdown, &mime, &args.url))
            },
            FetchedContent::Text { mime, body } => {
                Ok(Self::build_page_from_text(&body, &mime, &args.url))
            },
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
        include_images: bool,
    ) -> DaedraResult<PageContent> {
        let document = Html::parse_document(html);

        self.check_bot_protection(&document)?;

        let title = self.extract_title(&document);
        let content = self.extract_content(html, &document, url, selector, include_images)?;

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
            content_type: Some("html".to_string()),
            links,
        })
    }

    pub(super) fn build_page_from_pdf(text: &str, url: &str) -> PageContent {
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
            content_type: Some("pdf".to_string()),
            links: None,
        }
    }

    /// Convert a document (docx, odt, epub, ...) already rendered to
    /// Markdown by `anydoc` into a page. Content is used verbatim: it is
    /// already Markdown.
    fn build_page_from_document(markdown: &str, mime: &str, url: &str) -> PageContent {
        let trimmed = markdown.trim();
        let title = title_from_url(url);
        let word_count = word_count(trimmed);

        info!(
            url = %url,
            mime = %mime,
            word_count = word_count,
            "Document converted to Markdown"
        );

        PageContent {
            url: url.to_string(),
            title,
            content: trimmed.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            word_count,
            content_type: Some(mime.to_string()),
            links: None,
        }
    }

    /// Wrap textual non-HTML content (SVG, XML, JSON) in a code fence. The
    /// title comes from the URL file name.
    fn build_page_from_text(body: &str, mime: &str, url: &str) -> PageContent {
        let trimmed = body.trim();
        let title = title_from_url(url);
        let content = format!("```{mime}\n{trimmed}\n```");
        let word_count = word_count(&content);

        info!(
            url = %url,
            mime = %mime,
            word_count = word_count,
            "Textual content fetched successfully"
        );

        PageContent {
            url: url.to_string(),
            title,
            content,
            timestamp: chrono::Utc::now().to_rfc3339(),
            word_count,
            content_type: Some(mime.to_string()),
            links: None,
        }
    }

    /// Fetch page content with retry logic
    async fn fetch_with_retry(&self, url: &str) -> DaedraResult<FetchedContent> {
        let client = self.client.clone();
        let url = url.to_string();

        // Inline retry loop: transient errors (network) retry with doubling
        // delays from 400 ms to 2 s inside a 60 s window. Permanent errors
        // return at once. Replaces the unmaintained `backoff` crate.
        let mut delay = Duration::from_millis(400);
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let outcome = async {
                let response = client.get(&url).send().await.map_err(|e| {
                    warn!(error = %e, url = %url, "Fetch request failed, retrying...");
                    RetryErr::Transient(DaedraError::HttpError(e))
                })?;

                classify_response_status(response.status(), &url)?;

                if let Some(content_length) = response.content_length()
                    && content_length as usize > MAX_CONTENT_SIZE
                {
                    return Err(RetryErr::Permanent(DaedraError::FetchError(
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
                        RetryErr::Permanent(DaedraError::HttpError(e))
                    })?;
                    check_body_size(bytes.len())?;
                    return Ok(extract_pdf_content(&bytes)?);
                }

                // SVG/XML/JSON look binary to the mime list but are text we
                // can deliver verbatim; Office documents convert to
                // Markdown. Both are handled by classification instead.
                if is_known_binary_content_type(&ct)
                    && !is_textual_content_type(&ct)
                    && !is_office_content_type(&ct)
                    && !is_container_mime(&ct)
                {
                    let bytes = response.bytes().await.map_err(|e| {
                        error!(error = %e, url = %url, "Failed to read response body");
                        RetryErr::Permanent(DaedraError::HttpError(e))
                    })?;
                    check_body_size(bytes.len())?;
                    return Ok(FetchedContent::Binary {
                        mime: ct,
                        size: bytes.len(),
                    });
                }

                let bytes = response.bytes().await.map_err(|e| {
                    error!(error = %e, url = %url, "Failed to read response body");
                    RetryErr::Permanent(DaedraError::HttpError(e))
                })?;
                check_body_size(bytes.len())?;

                classify_fetched_content(&content_type, &bytes).map_err(RetryErr::Permanent)
            };

            match outcome.await {
                Ok(content) => return Ok(content),
                Err(RetryErr::Permanent(e)) => return Err(e),
                Err(RetryErr::Transient(e)) => {
                    if Instant::now() + delay > deadline {
                        return Err(e);
                    }
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(2));
                },
            }
        }
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
        include_images: bool,
    ) -> DaedraResult<String> {
        let content_html = self.select_content_html(html, document, url, selector)?;
        let content_html = if include_images {
            content_html
        } else {
            strip_img_tags(&content_html)
        };
        let markdown = html_to_markdown(&content_html);
        let cleaned = clean_markdown(&markdown);

        if word_count(&cleaned) < 10 {
            warn!("Extracted content is very short");
        }

        Ok(cleaned)
    }

    fn select_html_fragment(&self, document: &Html, sel: &str) -> DaedraResult<Option<String>> {
        let custom_selector = Selector::parse(sel)
            .map_err(|_| DaedraError::InvalidArguments(format!("Invalid CSS selector: {}", sel)))?;

        Ok(document.select(&custom_selector).next().map(|el| el.html()))
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
    pub(super) fn extract_links(&self, document: &Html, base_url: &Url) -> Vec<PageLink> {
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

pub(super) fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

pub(super) fn is_retryable_status(status: u16) -> bool {
    status == 429
}

pub(super) fn classify_response_status(
    status: reqwest::StatusCode,
    url: &str,
) -> Result<(), RetryErr> {
    if status.is_success() {
        return Ok(());
    }

    warn!(status = %status, url = %url, "Fetch returned non-success status");

    if is_retryable_status(status.as_u16()) {
        return Err(RetryErr::Transient(DaedraError::RateLimitExceeded));
    }

    if status.as_u16() == 403 {
        return Err(RetryErr::Permanent(DaedraError::BotProtectionDetected));
    }

    Err(RetryErr::Permanent(DaedraError::FetchError(format!(
        "HTTP {}",
        status
    ))))
}

pub(super) fn extract_with_readability(html: &str, url: &str) -> Option<String> {
    let document_url = if url.is_empty() { None } else { Some(url) };
    let mut readability = Readability::new(html, document_url, None).ok()?;
    let article = readability.parse().ok()?;
    if word_count(&article.text_content) >= 50 {
        Some(article.content.to_string())
    } else {
        None
    }
}

pub(super) fn title_from_url(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .path_segments()
                .and_then(|segments| segments.rev().find(|s| !s.is_empty()))
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
#[cfg(test)]
impl FetchClient {
    /// Same extraction path as [`FetchClient::fetch`] but without HTTP (integration fixtures).
    pub fn extract_content_from_html_for_tests(
        &self,
        html: &str,
        selector: Option<&str>,
        include_images: bool,
    ) -> DaedraResult<String> {
        let document = Html::parse_document(html);
        self.extract_content(html, &document, "", selector, include_images)
    }

    /// Same path as [`FetchClient::build_page_from_html`] without HTTP.
    pub fn build_page_from_html_for_tests(
        &self,
        html: &str,
        url: &str,
        selector: Option<&str>,
        include_images: bool,
    ) -> DaedraResult<PageContent> {
        let parsed_url = validate_url(url)?;
        self.build_page_from_html(html, url, &parsed_url, selector, include_images)
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

pub(super) fn has_bot_protection_element(document: &Html) -> bool {
    BOT_PROTECTION_SELECTORS
        .iter()
        .any(|s| document.select(s).next().is_some())
}

pub(super) fn has_suspicious_title(document: &Html) -> bool {
    document.select(&TITLE_SELECTOR).next().is_some_and(|el| {
        let title = el.text().collect::<String>().to_lowercase();
        SUSPICIOUS_TITLES.iter().any(|s| title.contains(s))
    })
}

pub(super) fn text_from_selector(document: &Html, selector: &Selector) -> Option<String> {
    document
        .select(selector)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|t| !t.is_empty())
        .map(|t| clean_title(&t))
}

/// Fetch a page and extract its content as Markdown.
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
pub fn html_to_page_content(
    html: &str,
    url: &str,
    include_images: bool,
) -> DaedraResult<PageContent> {
    let client = FetchClient::default();
    client.build_page_from_html(html, url, &validate_url(url)?, None, include_images)
}
