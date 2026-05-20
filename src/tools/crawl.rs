//! Deep-crawl a website with sitemap discovery + bounded concurrent fetch.
//!
//! This module provides the building blocks for walking a site intelligently:
//!
//! 1. **Sitemap discovery** — fetches `/sitemap.xml` (and common aliases)
//!    and extracts the URL list. Falls back to anchor discovery from the
//!    root page when no sitemap exists.
//! 2. **Bounded concurrent fetch** — pulls a batch of URLs through the
//!    existing [`fetch::visit_page`] pipeline, respecting a user-supplied
//!    concurrency cap via [`tokio::sync::Semaphore`].
//!
//! LLM-based URL ranking is deliberately **not** part of this module. The
//! consumer (ARES, pawan, or any downstream that already has an LLM client)
//! is expected to pre-select which URLs to deep-fetch. daedra's job is to
//! make that selection fast and correct, not to make it smart.
//!
//! This is the "deep" half of the `broad search + deep crawl` MIT stack —
//! see `reference_smartcrawler_vs_daedra.md` for the design rationale.

use crate::tools::fetch::fetch_page;
use crate::types::{
    CrawlArgs, CrawlError, CrawlResult, CrawlSummary, CrawledPage, DaedraError, DaedraResult,
    PageContent, VisitPageArgs,
};
use lazy_static::lazy_static;
use reqwest::Client;
use scraper::{Html, Selector};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::{info, warn};
use url::Url;

/// Default User-Agent string for sitemap/robots fetches.
const USER_AGENT: &str = "Mozilla/5.0 (compatible; daedra-crawl/0.3.0; +https://github.com/dirmacs/daedra)";

/// Hard cap on sitemap response size (10 MB) to bound worst-case parser work.
const SITEMAP_MAX_BYTES: usize = 10 * 1024 * 1024;

/// Default per-request timeout when fetching the sitemap itself.
const SITEMAP_TIMEOUT: Duration = Duration::from_secs(15);

/// Common sitemap paths to probe in order before giving up.
const SITEMAP_CANDIDATES: &[&str] = &[
    "/sitemap.xml",
    "/sitemap_index.xml",
    "/sitemap-index.xml",
    "/wp-sitemap.xml",
];

lazy_static! {
    static ref ANCHOR_SELECTOR: Selector = Selector::parse("a[href]").unwrap();
}

async fn fetch_sitemap_body(client: &Client, url: &Url) -> Option<String> {
    let resp = match client
        .get(url.clone())
        .header("User-Agent", USER_AGENT)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!("sitemap probe {} failed: {}", url, e);
            return None;
        }
    };

    if !resp.status().is_success() {
        return None;
    }

    match resp.text().await {
        Ok(b) if b.len() <= SITEMAP_MAX_BYTES => Some(b),
        Ok(_) => {
            warn!(
                "sitemap {} exceeded {} bytes, skipping",
                url, SITEMAP_MAX_BYTES
            );
            None
        }
        Err(e) => {
            warn!("sitemap {} body read failed: {}", url, e);
            None
        }
    }
}

async fn probe_sitemap_candidate(client: &Client, root: &Url, path: &str) -> Option<Vec<Url>> {
    let url = root.join(path).ok()?;
    let body = fetch_sitemap_body(client, &url).await?;
    let urls = parse_sitemap(&body);
    if urls.is_empty() {
        None
    } else {
        info!("sitemap {} yielded {} URLs", url, urls.len());
        Some(urls)
    }
}

/// Try each well-known sitemap path under `root` and return the first one
/// that parses to a non-empty URL list. Returns `Ok(None)` if every candidate
/// is missing, malformed, or empty (fallback to HTML anchor discovery).
async fn discover_sitemap(client: &Client, root: &Url) -> DaedraResult<Option<Vec<Url>>> {
    for candidate in SITEMAP_CANDIDATES {
        if let Some(urls) = probe_sitemap_candidate(client, root, candidate).await {
            return Ok(Some(urls));
        }
    }

    Ok(None)
}

/// Parse a sitemap XML body into a URL list.
///
/// Accepts both single sitemaps (`<urlset><url><loc>...</loc></url>...`)
/// and sitemap indexes (`<sitemapindex><sitemap><loc>...</loc></sitemap>...`).
/// Index entries are returned as-is; callers that want to recursively expand
/// them must do so themselves — this keeps the parser decoupled from I/O.
///
/// Invalid URLs are dropped silently rather than failing the whole parse,
/// which matches how real-world crawlers handle the messy sitemap ecosystem.
pub fn parse_sitemap(body: &str) -> Vec<Url> {
    let mut out = Vec::new();
    let mut in_loc = false;
    let mut current = String::new();

    // The sitemap XML schema is rigid enough that a tag-aware substring scan
    // outperforms a full XML parser and doesn't pull in xml-rs at the cost
    // of one more heavy dep. We look for `<loc>...</loc>` pairs anywhere in
    // the document, which covers both urlset and sitemapindex shapes.
    let mut rest = body;
    while let Some(open) = rest.find("<loc>") {
        let after_open = &rest[open + "<loc>".len()..];
        let Some(close) = after_open.find("</loc>") else {
            break;
        };
        let loc_text = after_open[..close].trim();
        if let Ok(parsed) = Url::parse(loc_text) {
            if !out.iter().any(|existing: &Url| existing == &parsed) {
                out.push(parsed);
            }
        }
        rest = &after_open[close + "</loc>".len()..];
        // Silence the unused write-only state — `current`/`in_loc` are
        // reserved for a future switch to a proper SAX pass if sitemaps
        // with embedded HTML comments start tripping the naive scan.
        current.clear();
        let _ = in_loc;
        in_loc = false;
    }

    out
}

/// Fall back to HTML anchor discovery when no sitemap is available.
/// Fetches `root`, extracts same-origin anchor hrefs, and returns up to
/// `cap` absolute URLs. This is deliberately minimal — for real crawling
/// recursion, the consumer should use the returned URLs as seed input to
/// a subsequent `crawl_site` call.
async fn discover_via_anchors(client: &Client, root: &Url, cap: usize) -> DaedraResult<Vec<Url>> {
    let body = client
        .get(root.clone())
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| DaedraError::FetchError(format!("anchor discovery GET {} failed: {}", root, e)))?
        .text()
        .await
        .map_err(|e| DaedraError::FetchError(format!("anchor discovery body {} failed: {}", root, e)))?;

    let doc = Html::parse_document(&body);
    let mut seen: Vec<Url> = Vec::new();
    for a in doc.select(&ANCHOR_SELECTOR) {
        if let Some(href) = a.value().attr("href") {
            let absolute = match root.join(href) {
                Ok(u) => u,
                Err(_) => continue,
            };
            if absolute.origin() != root.origin() {
                continue;
            }
            if seen.iter().all(|u| u != &absolute) {
                seen.push(absolute);
                if seen.len() >= cap {
                    break;
                }
            }
        }
    }
    Ok(seen)
}

fn clamp_crawl_args(max_pages: usize, concurrency: usize) -> (usize, usize) {
    (max_pages.max(1).min(500), concurrency.max(1).min(16))
}

fn rank_urls_by_path_length(urls: &mut [Url]) {
    urls.sort_by_key(|u| u.path().len());
}

/// Discover crawl candidates: sitemap first, HTML anchors as fallback.
async fn discover_urls(
    client: &Client,
    root: &Url,
    max_pages: usize,
) -> DaedraResult<(Vec<Url>, bool)> {
    match discover_sitemap(client, root).await? {
        Some(urls) => Ok((urls, true)),
        None => {
            let urls = discover_via_anchors(client, root, max_pages * 2).await?;
            Ok((urls, false))
        }
    }
}

/// Spawn semaphore-guarded fetch tasks for each candidate URL.
async fn fetch_candidates_concurrently(
    candidates: Vec<Url>,
    concurrency: usize,
) -> Vec<tokio::task::JoinHandle<Option<(String, DaedraResult<PageContent>)>>> {
    let sem = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::with_capacity(candidates.len());
    for url in candidates {
        let sem = Arc::clone(&sem);
        let args = VisitPageArgs {
            url: url.to_string(),
            selector: None,
            include_images: false,
        };
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.ok()?;
            let result = fetch_page(&args).await;
            Some((args.url, result))
        }));
    }
    handles
}

/// Join fetch tasks and partition results into pages and errors.
async fn collect_crawl_results(
    handles: Vec<tokio::task::JoinHandle<Option<(String, DaedraResult<PageContent>)>>>,
    _requested: usize,
) -> (Vec<CrawledPage>, Vec<CrawlError>) {
    let mut pages: Vec<CrawledPage> = Vec::new();
    let mut errors: Vec<CrawlError> = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Some((url, Ok(page)))) => {
                let links = page
                    .links
                    .unwrap_or_default()
                    .into_iter()
                    .map(|l| l.url)
                    .collect();
                pages.push(CrawledPage {
                    url,
                    title: page.title,
                    markdown: page.content,
                    links,
                });
            }
            Ok(Some((url, Err(e)))) => errors.push(CrawlError {
                url,
                error: e.to_string(),
            }),
            Ok(None) | Err(_) => {
                // semaphore closed or task panic — skip silently
            }
        }
    }
    (pages, errors)
}

/// Walk a site deeply, returning extracted page content for each URL.
///
/// The caller supplies a URL and a page budget. daedra finds the URLs
/// (sitemap first, HTML anchors second), fetches them under a concurrency
/// semaphore, converts each to markdown via the existing `visit_page`
/// pipeline, and returns a structured result with per-URL success/error
/// buckets.
pub async fn crawl_site(args: CrawlArgs) -> DaedraResult<CrawlResult> {
    let root = Url::parse(&args.root_url)
        .map_err(|e| DaedraError::InvalidArguments(format!("invalid root_url: {}", e)))?;

    let (max_pages, concurrency) = clamp_crawl_args(args.max_pages, args.concurrency);

    let client = Client::builder()
        .user_agent(USER_AGENT)
        .timeout(SITEMAP_TIMEOUT)
        .gzip(true)
        .brotli(true)
        .build()
        .map_err(|e| DaedraError::FetchError(format!("http client build: {}", e)))?;

    let (mut candidates, sitemap_found) = discover_urls(&client, &root, max_pages).await?;
    rank_urls_by_path_length(&mut candidates);
    candidates.truncate(max_pages);

    info!(
        root = %root,
        sitemap_found,
        candidates = candidates.len(),
        concurrency,
        "crawl_site starting"
    );

    let handles = fetch_candidates_concurrently(candidates, concurrency).await;
    let (pages, errors) = collect_crawl_results(handles, max_pages).await;

    Ok(CrawlResult {
        root_url: root.to_string(),
        sitemap_found,
        summary: CrawlSummary {
            requested: max_pages,
            fetched: pages.len(),
            failed: errors.len(),
        },
        pages,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sitemap_handles_urlset() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url>
    <loc>https://example.com/</loc>
    <lastmod>2026-01-01</lastmod>
  </url>
  <url>
    <loc>https://example.com/about</loc>
  </url>
  <url>
    <loc>https://example.com/docs/intro</loc>
  </url>
</urlset>"#;
        let urls = parse_sitemap(xml);
        assert_eq!(urls.len(), 3, "expected 3 unique URLs from urlset");
        assert_eq!(urls[0].as_str(), "https://example.com/");
        assert_eq!(urls[2].path(), "/docs/intro");
    }

    #[test]
    fn parse_sitemap_handles_sitemapindex() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <sitemap><loc>https://example.com/sitemap-1.xml</loc></sitemap>
  <sitemap><loc>https://example.com/sitemap-2.xml</loc></sitemap>
</sitemapindex>"#;
        let urls = parse_sitemap(xml);
        assert_eq!(urls.len(), 2, "sitemap index should return its nested loc entries");
        assert!(urls[0].path().ends_with("sitemap-1.xml"));
    }

    #[test]
    fn parse_sitemap_drops_invalid_urls() {
        let xml = r#"<urlset>
            <url><loc>not-a-url</loc></url>
            <url><loc>https://example.com/ok</loc></url>
            <url><loc>   </loc></url>
        </urlset>"#;
        let urls = parse_sitemap(xml);
        assert_eq!(urls.len(), 1, "only the one valid URL should survive");
        assert_eq!(urls[0].as_str(), "https://example.com/ok");
    }

    #[test]
    fn parse_sitemap_deduplicates() {
        let xml = r#"<urlset>
            <url><loc>https://example.com/a</loc></url>
            <url><loc>https://example.com/a</loc></url>
            <url><loc>https://example.com/b</loc></url>
        </urlset>"#;
        let urls = parse_sitemap(xml);
        assert_eq!(urls.len(), 2, "duplicates should collapse");
    }

    #[test]
    fn parse_sitemap_empty_returns_empty_vec() {
        assert!(parse_sitemap("").is_empty());
        assert!(parse_sitemap("<?xml version=\"1.0\"?><urlset></urlset>").is_empty());
    }
    #[test]
    fn test_clamp_crawl_args_min() {
        assert_eq!(clamp_crawl_args(0, 0), (1, 1));
    }

    #[test]
    fn test_clamp_crawl_args_max() {
        assert_eq!(clamp_crawl_args(1000, 100), (500, 16));
    }

    #[test]
    fn test_clamp_crawl_args_passthrough() {
        assert_eq!(clamp_crawl_args(10, 4), (10, 4));
    }

    #[test]
    fn test_rank_urls_by_path_length() {
        let mut urls = vec![
            Url::parse("https://example.com/b/c/d").unwrap(),
            Url::parse("https://example.com/a").unwrap(),
            Url::parse("https://example.com/e/f").unwrap(),
        ];
        rank_urls_by_path_length(&mut urls);
        let paths: Vec<_> = urls.iter().map(|u| u.path().len()).collect();
        assert_eq!(paths, [2, 4, 6]);
        assert_eq!(urls[0].path(), "/a");
        assert_eq!(urls[1].path(), "/e/f");
        assert_eq!(urls[2].path(), "/b/c/d");
    }

    #[test]
    fn test_parse_sitemap_unclosed_loc() {
        assert!(parse_sitemap("<loc>no closing tag").is_empty());
    }

    #[test]
    fn test_parse_sitemap_mixed_valid_invalid() {
        let xml = r#"<urlset>
            <url><loc>not-a-url</loc></url>
            <url><loc>https://example.com/ok</loc></url>
            <url><loc>   </loc></url>
        </urlset>"#;
        let urls = parse_sitemap(xml);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].as_str(), "https://example.com/ok");
    }

    #[test]
    fn test_parse_sitemap_xml_with_comments() {
        let xml = r#"<?xml version="1.0"?>
<urlset>
  <url><loc>https://example.com/first</loc></url>
  <!-- comment between loc tags -->
  <url><loc>https://example.com/second</loc></url>
</urlset>"#;
        let urls = parse_sitemap(xml);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].as_str(), "https://example.com/first");
        assert_eq!(urls[1].as_str(), "https://example.com/second");
    }

}
