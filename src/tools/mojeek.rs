//! Mojeek search backend — no API key needed.
//!
//! Scrapes Mojeek's HTML results page. Mojeek runs an independent crawler
//! and index, is bot-tolerant from residential IPs, and refuses datacenter
//! IPs outright (plain 403). The fallback chain handles both cases: 403 is
//! a fail-fast error and the circuit breaker sidelines the backend where it
//! is blocked. Honors SafeSearch via Mojeek's `safe` parameter.

use super::backend::SearchBackend;
use super::soft_block::{self, EmptyPage};
use crate::types::{
    ContentType, DaedraError, DaedraResult, ResultMetadata, SafeSearchLevel, SearchArgs,
    SearchResponse, SearchResult,
};
use async_trait::async_trait;
use lazy_static::lazy_static;
use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use std::time::Duration;
use tracing::{info, warn};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const MOJEEK_URL: &str = "https://www.mojeek.com/search";

/// The headers a real browser sends on a top-level navigation. Mojeek's bot
/// wall has two layers: untrusted networks get a plain 403, while trusted
/// networks with a non-browser client (plain reqwest TLS fingerprint) get a
/// silent non-HTML 200 that parses as zero results. The full header set is
/// the cheapest honest way to look like the browser the UA claims to be.
fn browser_default_headers() -> reqwest::header::HeaderMap {
    use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, HeaderMap, HeaderName, HeaderValue};
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        ),
    );
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
    headers.insert(
        HeaderName::from_static("upgrade-insecure-requests"),
        HeaderValue::from_static("1"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-dest"),
        HeaderValue::from_static("document"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-mode"),
        HeaderValue::from_static("navigate"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("none"),
    );
    headers
}

lazy_static! {
    /// Mojeek's organic result items. The current SERP (stylesheet v2.119)
    /// wraps each result in an `<article>`; the `<li>` forms are the legacy
    /// markup kept as fallbacks.
    static ref RESULT_SELECTOR: Selector = Selector::parse(
        ".results-standard > article, ul.results-standard > li, li.results-standard",
    )
    .unwrap();
    /// Result title link (class `ob`) with fallbacks for layout shifts.
    static ref TITLE_SELECTORS: [Selector; 3] = [
        Selector::parse("a.ob").unwrap(),
        Selector::parse("h2 a").unwrap(),
        Selector::parse("a.title").unwrap(),
    ];
    /// Result snippet paragraph (class `s`).
    static ref SNIPPET_SELECTORS: [Selector; 2] = [
        Selector::parse("p.s").unwrap(),
        Selector::parse("p.desc").unwrap(),
    ];
    /// Layout-tolerant organic title link. Mojeek's LLM-results container
    /// still uses `a.ob` even when `.results-standard` is absent.
    static ref OB_LINK: Selector = Selector::parse("a.ob").unwrap();
}

/// Resolve a result href. Organic links are absolute; a tracking wrapper
/// (`/url?url=https%3A%2F%2F...`) unwraps to the destination.
fn unwrap_mojeek_href(href: &str) -> Option<String> {
    let href = href.trim();
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    let abs = if href.starts_with('/') {
        format!("https://www.mojeek.com{href}")
    } else {
        return None;
    };
    let parsed = url::Url::parse(&abs).ok()?;
    if parsed.path() != "/url" {
        return None;
    }
    parsed
        .query_pairs()
        .find(|(k, _)| k == "url")
        .map(|(_, v)| v.into_owned())
        .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
}

/// Compact description of an unparseable page so the next log names the
/// layout instead of hiding it behind "bot protection".
fn page_fingerprint(html: &str) -> String {
    let hay = html.to_lowercase();
    let markers = [
        "results-standard",
        "llm-results",
        "class=\"ob\"",
        "class='ob'",
        "no-results",
        "challenge",
        "captcha",
        "unusual",
        "automated",
        "verify",
        "just a moment",
        "enable javascript",
        "cf-browser",
    ];
    let flags: Vec<&str> = markers
        .iter()
        .copied()
        .filter(|m| hay.contains(m))
        .collect();
    let text: String = html
        .split('<')
        .filter_map(|chunk| chunk.split('>').nth(1))
        .collect::<Vec<_>>()
        .join(" ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let snippet: String = text.chars().take(180).collect();
    format!(
        "len={} flags=[{}] snippet={:?}",
        html.len(),
        flags.join(","),
        snippet
    )
}

fn result_from_parts(url: String, title: String, description: String) -> SearchResult {
    SearchResult {
        title,
        url,
        description,
        metadata: ResultMetadata {
            content_type: ContentType::Other,
            source: "mojeek".to_string(),
            favicon: None,
            published_date: None,
        },
    }
}

fn safe_param(level: SafeSearchLevel) -> Option<&'static str> {
    match level {
        SafeSearchLevel::Off => Some("0"),
        SafeSearchLevel::Moderate => None,
        SafeSearchLevel::Strict => Some("1"),
    }
}

fn extract_mojeek_result(element: &ElementRef) -> Option<SearchResult> {
    let mut link = None;
    for sel in TITLE_SELECTORS.iter() {
        if let Some(a) = element.select(sel).find(|a| {
            a.value()
                .attr("href")
                .is_some_and(|h| unwrap_mojeek_href(h).is_some())
        }) {
            link = Some(a);
            break;
        }
    }
    let link_el = link?;

    let url = unwrap_mojeek_href(link_el.value().attr("href").unwrap_or_default())?;
    let title: String = link_el.text().collect();
    if title.trim().is_empty() {
        return None;
    }

    let description: String = SNIPPET_SELECTORS
        .iter()
        .find_map(|sel| element.select(sel).next())
        .map(|e| e.text().collect())
        .unwrap_or_default();

    Some(result_from_parts(
        url,
        title.trim().to_string(),
        description.trim().to_string(),
    ))
}

fn extract_from_anchor(anchor: ElementRef<'_>) -> Option<SearchResult> {
    let url = unwrap_mojeek_href(anchor.value().attr("href").unwrap_or_default())?;
    let title: String = anchor.text().collect::<String>().trim().to_string();
    if title.is_empty() {
        return None;
    }
    let description = anchor
        .parent()
        .and_then(ElementRef::wrap)
        .and_then(|parent| {
            SNIPPET_SELECTORS
                .iter()
                .find_map(|sel| parent.select(sel).next())
        })
        .map(|e| e.text().collect::<String>().trim().to_string())
        .unwrap_or_default();
    Some(result_from_parts(url, title, description))
}

/// Mojeek HTML scraping backend — independent index, no API key.
pub struct MojeekBackend {
    client: Client,
    base_url: String,
}

impl MojeekBackend {
    /// Create a new Mojeek backend instance.
    pub fn new() -> Self {
        Self::with_base_url(MOJEEK_URL.to_string())
    }

    /// Point at a custom endpoint (tests).
    pub fn with_base_url(base_url: String) -> Self {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .default_headers(browser_default_headers())
            // Keep the session: walls that hand out a clearance cookie via
            // Set-Cookie fail when the client drops it between requests.
            .cookie_store(true)
            .timeout(Duration::from_secs(30))
            .gzip(true)
            .brotli(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .expect("Failed to build HTTP client");
        Self { client, base_url }
    }

    /// Parse a Mojeek SERP page into results (exposed for regression tests).
    pub fn parse_results(&self, html: &str, max_results: usize) -> Vec<SearchResult> {
        let document = Html::parse_document(html);
        let structured: Vec<SearchResult> = document
            .select(&RESULT_SELECTOR)
            .filter_map(|e| extract_mojeek_result(&e))
            .take(max_results)
            .collect();
        if !structured.is_empty() {
            return structured;
        }
        document
            .select(&OB_LINK)
            .filter_map(extract_from_anchor)
            .take(max_results)
            .collect()
    }
}

impl Default for MojeekBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MojeekBackend {
    /// Extract the redirect target of a no-JS `<meta http-equiv="refresh">`
    /// handoff. Relative targets resolve against the configured base URL.
    fn meta_refresh_target(&self, html: &str) -> Option<String> {
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let re = RE.get_or_init(|| {
            regex::Regex::new(
                r#"(?is)<meta[^>]+http-equiv=["']?refresh["']?[^>]*content=["'][^"']*url=([^"'>]+)"#,
            )
            .unwrap()
        });
        let raw = re.captures(html)?.get(1)?.as_str().trim().to_string();
        if raw.is_empty() {
            return None;
        }
        url::Url::parse(&self.base_url)
            .ok()?
            .join(&raw)
            .ok()
            .map(|u| u.to_string())
    }
}

#[async_trait]
impl SearchBackend for MojeekBackend {
    fn name(&self) -> &str {
        "mojeek"
    }

    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();

        // `t` is Mojeek's results-per-page parameter (10/20/30/40). The old
        // request sent `tlen` here, which is the TITLE character limit — it
        // silently truncated every title to `num_results` characters.
        let mut query: Vec<(&str, String)> = vec![("q", args.query.clone())];
        let per_page = if opts.num_results > 30 {
            Some(40)
        } else if opts.num_results > 20 {
            Some(30)
        } else if opts.num_results > 10 {
            Some(20)
        } else {
            None // Mojeek's default is 10
        };
        if let Some(t) = per_page {
            query.push(("t", t.to_string()));
        }
        if let Some(safe) = safe_param(opts.safe_search) {
            query.push(("safe", safe.to_string()));
        }

        let resp = self
            .client
            .get(&self.base_url)
            .query(&query)
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        if !resp.status().is_success() {
            warn!(status = %resp.status(), "Mojeek returned non-200");
            return Err(DaedraError::SearchError(format!(
                "Mojeek status {}",
                resp.status()
            )));
        }

        // A 200 that is not HTML is Mojeek's quiet bot wall for trusted
        // networks: the request reached it, but its client fingerprinting
        // did not clear the automated client. Say so instead of reporting
        // an unparseable zero-result page.
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_lowercase();
        if !content_type.contains("text/html") && !content_type.contains("application/xhtml") {
            warn!(content_type = %content_type, "Mojeek served a non-HTML response");
            return Err(DaedraError::SearchError(format!(
                "Mojeek served {content_type} instead of HTML — its bot wall flagged the \
                 automated client; results are unavailable from this machine"
            )));
        }

        let mut html = resp.text().await.map_err(DaedraError::HttpError)?;

        // A no-JavaScript challenge page hands the browser to the real SERP
        // with a meta refresh. Follow exactly one hop; a loop or a second
        // refresh means the wall stays up and the soft-block path reports it.
        if let Some(target) = self.meta_refresh_target(&html) {
            info!(target = %target, "Following Mojeek meta-refresh handoff");
            let resp2 = self
                .client
                .get(&target)
                .send()
                .await
                .map_err(DaedraError::HttpError)?;
            if resp2.status().is_success() {
                html = resp2.text().await.map_err(DaedraError::HttpError)?;
            }
        }

        let results = self.parse_results(&html, opts.num_results);

        if results.is_empty() {
            match soft_block::classify(
                &html,
                &[
                    "no results found",
                    "did not match",
                    // Mojeek's empty result list carries this class; the
                    // current stylesheet (v2.119) still ships `.no-results`.
                    "no-results",
                    "there are no results",
                    "no results for",
                ],
            ) {
                EmptyPage::GenuineNoResults => {
                    info!("Mojeek genuinely reports no results for this query");
                },
                EmptyPage::SoftBlock => {
                    let fp = page_fingerprint(&html);
                    warn!(%fp, "Mojeek returned 200 with zero results and no no-results marker");
                    let hay = fp.to_lowercase();
                    if [
                        "challenge",
                        "captcha",
                        "automated",
                        "unusual",
                        "just a moment",
                    ]
                    .iter()
                    .any(|m| hay.contains(m))
                    {
                        return Err(DaedraError::BotProtectionDetected);
                    }
                    return Err(DaedraError::SearchError(format!(
                        "Mojeek served HTML that did not parse as results ({fp})"
                    )));
                },
            }
        }

        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"<html><body><ul class="results-standard">
      <li><a class="ob" href="https://tokio.rs/">Tokio - An asynchronous Rust runtime</a>
          <p class="s">A runtime for writing reliable asynchronous applications.</p></li>
      <li><h2><a href="https://docs.rs/tokio">tokio - docs.rs</a></h2>
          <p class="s">A event-driven, non-blocking I/O platform.</p></li>
      <li><a class="ob" href="/search?q=next">Next page</a></li>
    </ul></body></html>"#;

    /// The current Mojeek SERP: `.results-standard` children are `<article>`
    /// elements and the title link wraps an `<h2>`.
    const MODERN_FIXTURE: &str = r#"<html><body><div class="results"><div class="results-standard">
      <article><a class="ob" href="https://tokio.rs/"><h2>Tokio - An asynchronous Rust runtime</h2></a>
          <p class="s">A runtime for writing reliable asynchronous applications.</p>
          <div class="serp-meta"><span>tokio.rs</span></div></article>
      <article><a class="ob" href="https://docs.rs/tokio"><h2>tokio - docs.rs</h2></a>
          <p class="s">A event-driven, non-blocking I/O platform.</p></article>
    </div></div></body></html>"#;

    #[test]
    fn test_parse_mojeek_results() {
        let backend = MojeekBackend::new();
        let results = backend.parse_results(FIXTURE, 10);
        assert_eq!(results.len(), 2, "internal links must be skipped");
        assert_eq!(results[0].url, "https://tokio.rs/");
        assert_eq!(results[0].metadata.source, "mojeek");
        assert!(results[0].description.contains("reliable asynchronous"));
        // Fallback title selector (h2 a) also parses.
        assert_eq!(results[1].url, "https://docs.rs/tokio");
    }

    #[test]
    fn test_parse_modern_article_layout() {
        let backend = MojeekBackend::new();
        let results = backend.parse_results(MODERN_FIXTURE, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "https://tokio.rs/");
        assert_eq!(
            results[0].title,
            "Tokio - An asynchronous Rust runtime".to_string()
        );
        assert!(results[0].description.contains("reliable asynchronous"));
        assert_eq!(results[1].url, "https://docs.rs/tokio");
    }

    #[test]
    fn test_parse_empty_page() {
        let backend = MojeekBackend::new();
        assert!(
            backend
                .parse_results("<html><body><p>no results found</p></body></html>", 10)
                .is_empty()
        );
    }

    #[test]
    fn test_parse_llm_results_layout_without_results_standard() {
        let html = r#"<html><body><div class="container llm-results"><div class="results">
            <a class="ob" href="https://tokio.rs/"><h2>Tokio</h2></a>
            <p class="s">A runtime for writing reliable asynchronous applications.</p>
        </div></div></body></html>"#;
        let results = MojeekBackend::new().parse_results(html, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://tokio.rs/");
        assert!(results[0].title.contains("Tokio"));
        assert!(results[0].description.contains("reliable asynchronous"));
    }

    #[test]
    fn test_unwrap_tracking_href() {
        let html = r#"<html><body><div class="results-standard">
            <article><a class="ob" href="/url?q=&url=https%3A%2F%2Ftokio.rs%2F"><h2>Tokio</h2></a></article>
        </div></body></html>"#;
        let results = MojeekBackend::new().parse_results(html, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://tokio.rs/");
    }

    #[test]
    fn test_page_fingerprint_names_layout() {
        let fp = page_fingerprint(
            r#"<html><body><div class="llm-results">Verifying your browser</div></body></html>"#,
        );
        assert!(fp.contains("llm-results"), "{fp}");
        assert!(fp.contains("Verifying"), "{fp}");
    }
}
