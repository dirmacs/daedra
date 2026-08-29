//! Bot-tolerant machine-format search backends.
//!
//! Search engines fight HTML scraping with anti-bot challenges, but they
//! serve their own machine-readable formats (RSS, JSON APIs) to integrations
//! without challenge pages. These backends consume those formats directly:
//! same engines, same datacenter IPs, no keys, no CAPTCHAs.
//!
//! - [`BingRssBackend`] — Bing's `format=rss` output (general web results)
//! - [`GoogleNewsBackend`] — Google News RSS (news coverage)
//! - [`HnAlgoliaBackend`] — the Hacker News Algolia API (tech discussions)

use super::backend::SearchBackend;
use crate::types::{
    ContentType, DaedraError, DaedraResult, ResultMetadata, SearchArgs, SearchResponse,
    SearchResult,
};
use async_trait::async_trait;
use quick_xml::Reader;
use quick_xml::events::Event;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tracing::info;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

fn search_client() -> Client {
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .gzip(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .expect("Failed to build HTTP client")
}

fn result(
    title: impl Into<String>,
    url: impl Into<String>,
    description: impl Into<String>,
    source: &str,
) -> SearchResult {
    SearchResult {
        title: title.into().trim().to_string(),
        url: url.into().trim().to_string(),
        description: description.into().trim().to_string(),
        metadata: ResultMetadata {
            content_type: ContentType::Other,
            source: source.to_string(),
            favicon: None,
            published_date: None,
        },
    }
}

/// Parse RSS `<item>` entries (title/link/description) out of an XML feed.
/// Tag-local to feeds; namespace prefixes are tolerated by matching on the
/// local tag name.
fn parse_rss_items(xml: &str) -> Vec<(String, String, String)> {
    // Do NOT trim text: entity refs split text chunks ("Rust ", "&amp;", "
    // Memory") and per-chunk trimming would eat the surrounding spaces.
    let mut reader = Reader::from_str(xml);

    let mut items = Vec::new();
    let mut in_item = false;
    let mut current_tag = String::new();
    let mut buf_title = String::new();
    let mut buf_link = String::new();
    let mut buf_desc = String::new();

    loop {
        let event = reader.read_event();
        match &event {
            Ok(Event::Start(e)) => {
                let name = e.local_name().into_inner().to_string();
                if name == "item" {
                    in_item = true;
                    buf_title.clear();
                    buf_link.clear();
                    buf_desc.clear();
                } else if in_item {
                    current_tag = name;
                }
            },
            Ok(Event::Text(t)) if in_item && !current_tag.is_empty() => {
                let text = quick_xml::escape::unescape_with(
                    t,
                    quick_xml::escape::resolve_predefined_entity,
                )
                .unwrap_or_default();
                match current_tag.as_str() {
                    "title" => buf_title.push_str(&text),
                    "link" => buf_link.push_str(&text),
                    "description" => buf_desc.push_str(&text),
                    _ => {},
                }
            },
            Ok(Event::CData(c)) if in_item && !current_tag.is_empty() => {
                let text = c.to_string();
                match current_tag.as_str() {
                    "title" => buf_title.push_str(&text),
                    "link" => buf_link.push_str(&text),
                    "description" => buf_desc.push_str(&text),
                    _ => {},
                }
            },
            Ok(Event::GeneralRef(r)) if in_item && !current_tag.is_empty() => {
                let text = quick_xml::escape::resolve_predefined_entity(r)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("&{};", r.as_ref()));
                match current_tag.as_str() {
                    "title" => buf_title.push_str(&text),
                    "link" => buf_link.push_str(&text),
                    "description" => buf_desc.push_str(&text),
                    _ => {},
                }
            },
            Ok(Event::End(e)) => {
                let name = e.local_name().into_inner().to_string();
                if name == "item" && in_item {
                    in_item = false;
                    if !buf_link.is_empty() {
                        items.push((buf_title.clone(), buf_link.clone(), buf_desc.clone()));
                    }
                } else if in_item {
                    current_tag.clear();
                }
            },
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {},
        }
    }
    items
}

fn html_unescape(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

/// Remove markup tags and collapse whitespace. Google News embeds raw HTML
/// (`<a href=...><font ...>`) in its RSS descriptions; agents should never
/// see it.
fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {},
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Google News recency operator for a cutoff in seconds (`1h`, `1d`, `7d`,
/// `1m`, `1y`). Scales up to the smallest operator that covers the window.
fn humanize_when(secs: u64) -> String {
    const HOUR: u64 = 60 * 60;
    const DAY: u64 = 24 * HOUR;
    if secs <= HOUR {
        "1h".to_string()
    } else if secs <= DAY {
        "1d".to_string()
    } else if secs <= 7 * DAY {
        "7d".to_string()
    } else if secs <= 30 * DAY {
        "1m".to_string()
    } else {
        "1y".to_string()
    }
}

fn build_rss_response(
    args: &SearchArgs,
    opts: &crate::types::SearchOptions,
    items: Vec<(String, String, String)>,
    source: &str,
) -> SearchResponse {
    let results: Vec<SearchResult> = items
        .into_iter()
        .take(opts.num_results)
        .map(|(t, l, d)| {
            result(
                strip_markup(&html_unescape(&t)),
                html_unescape(&l),
                strip_markup(&html_unescape(&d)),
                source,
            )
        })
        .collect();
    SearchResponse::new(args.query.clone(), results, opts)
}

// ---------------------------------------------------------------------------
// Bing RSS — `format=rss` on the regular search endpoint
// ---------------------------------------------------------------------------

/// Bing search via its machine-readable RSS output — the same index as the
/// HTML SERP, served to integrations without challenge pages.
pub struct BingRssBackend {
    client: Client,
    base_url: String,
}

impl BingRssBackend {
    /// Create a new Bing RSS backend instance.
    pub fn new() -> Self {
        Self::with_base_url("https://www.bing.com/search".to_string())
    }

    /// Create a backend pointed at a custom search endpoint (tests).
    pub fn with_base_url(base_url: String) -> Self {
        Self {
            client: search_client(),
            base_url,
        }
    }
}

impl Default for BingRssBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for BingRssBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();
        let resp = self
            .client
            .get(&self.base_url)
            .query(&[
                ("q", args.query.as_str()),
                ("format", "rss"),
                ("count", &opts.num_results.to_string()),
            ])
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        if !resp.status().is_success() {
            return Err(DaedraError::SearchError(format!(
                "Bing RSS status {}",
                resp.status()
            )));
        }
        let xml = resp.text().await.map_err(DaedraError::HttpError)?;
        let items = parse_rss_items(&xml);
        info!(
            backend = "bing-rss",
            results = items.len(),
            "Bing RSS search complete"
        );
        Ok(build_rss_response(args, &opts, items, "bing-rss"))
    }

    fn name(&self) -> &str {
        "bing-rss"
    }
}

// ---------------------------------------------------------------------------
// Google News RSS
// ---------------------------------------------------------------------------

/// Google News search via its RSS feed — news coverage without challenge
/// pages. Links are Google News redirects to the publisher article.
pub struct GoogleNewsBackend {
    client: Client,
    base_url: String,
}

impl GoogleNewsBackend {
    /// Create a new Google News RSS backend instance.
    pub fn new() -> Self {
        Self::with_base_url("https://news.google.com/rss/search".to_string())
    }

    /// Create a backend pointed at a custom search endpoint (tests).
    pub fn with_base_url(base_url: String) -> Self {
        Self {
            client: search_client(),
            base_url,
        }
    }
}

impl Default for GoogleNewsBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for GoogleNewsBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();
        let (gl, hl) = crate::types::region_to_gl_hl(&opts.region);
        let hl = hl.unwrap_or_else(|| "en".to_string());
        let gl = gl.unwrap_or_else(|| "US".to_string());

        // Google News takes a recency operator inside the query itself.
        let query = match opts
            .time_range
            .as_deref()
            .and_then(crate::types::time_range_secs)
        {
            Some(secs) => format!("{} when:{}", args.query, humanize_when(secs)),
            None => args.query.clone(),
        };

        let resp = self
            .client
            .get(&self.base_url)
            .query(&[
                ("q", query.as_str()),
                ("hl", &format!("{}-{}", hl, gl.to_uppercase())),
                ("gl", &gl.to_uppercase()),
                ("ceid", &format!("{}:{}", gl.to_uppercase(), hl)),
            ])
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        if !resp.status().is_success() {
            return Err(DaedraError::SearchError(format!(
                "Google News RSS status {}",
                resp.status()
            )));
        }
        let xml = resp.text().await.map_err(DaedraError::HttpError)?;
        let items = parse_rss_items(&xml);
        info!(
            backend = "gnews",
            results = items.len(),
            "Google News RSS search complete"
        );
        Ok(build_rss_response(args, &opts, items, "gnews"))
    }

    fn name(&self) -> &str {
        "gnews"
    }
}

// ---------------------------------------------------------------------------
// Hacker News (Algolia API)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct AlgoliaResponse {
    hits: Vec<AlgoliaHit>,
}

#[derive(Deserialize)]
struct AlgoliaHit {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    story_title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(rename = "objectID", default)]
    object_id: String,
    #[serde(default)]
    points: Option<u64>,
    #[serde(default)]
    num_comments: Option<u64>,
    #[serde(default)]
    author: Option<String>,
}

impl AlgoliaHit {
    fn effective_title(&self) -> String {
        self.title
            .clone()
            .or_else(|| self.story_title.clone())
            .unwrap_or_else(|| "(untitled)".to_string())
    }

    fn effective_url(&self) -> String {
        self.url
            .clone()
            .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={}", self.object_id))
    }

    fn effective_description(&self) -> String {
        match (self.points, self.num_comments, self.author.as_deref()) {
            (Some(p), Some(c), Some(a)) => format!("{p} points, {c} comments, by {a}"),
            _ => String::new(),
        }
    }
}

/// Hacker News search via the public Algolia API — no key, no challenges,
/// excellent signal for technical queries.
pub struct HnAlgoliaBackend {
    client: Client,
    base_url: String,
}

impl HnAlgoliaBackend {
    /// Create a new HN Algolia backend instance.
    pub fn new() -> Self {
        Self::with_base_url("https://hn.algolia.com/api/v1/search".to_string())
    }

    /// Create a backend pointed at a custom search endpoint (tests).
    pub fn with_base_url(base_url: String) -> Self {
        Self {
            client: search_client(),
            base_url,
        }
    }

    fn parse_response(&self, body: &str) -> DaedraResult<Vec<SearchResult>> {
        let parsed: AlgoliaResponse = serde_json::from_str(body)
            .map_err(|e| DaedraError::SearchError(format!("HN Algolia parse: {e}")))?;
        Ok(parsed
            .hits
            .into_iter()
            .map(|h| {
                result(
                    h.effective_title(),
                    h.effective_url(),
                    h.effective_description(),
                    "hn",
                )
            })
            .collect())
    }
}

impl Default for HnAlgoliaBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for HnAlgoliaBackend {
    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();
        let mut params: Vec<(String, String)> = vec![
            ("query".to_string(), args.query.clone()),
            ("hitsPerPage".to_string(), opts.num_results.to_string()),
        ];
        if let Some(secs) = opts
            .time_range
            .as_deref()
            .and_then(crate::types::time_range_secs)
        {
            let cutoff = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                .saturating_sub(secs);
            params.push((
                "numericFilters".to_string(),
                format!("created_at_i>{cutoff}"),
            ));
        }
        let resp = self
            .client
            .get(&self.base_url)
            .query(&params)
            .send()
            .await
            .map_err(DaedraError::HttpError)?;

        if !resp.status().is_success() {
            return Err(DaedraError::SearchError(format!(
                "HN Algolia status {}",
                resp.status()
            )));
        }
        let body = resp.text().await.map_err(DaedraError::HttpError)?;
        let results = self.parse_response(&body)?;
        info!(
            backend = "hn",
            results = results.len(),
            "HN Algolia search complete"
        );
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }

    fn name(&self) -> &str {
        "hn"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bing_rss_items() {
        let xml = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Bing: q</title>
            <item><title>Rust &amp; Memory</title><link>https://rust-lang.org/</link><description>Fast &amp; safe</description></item>
            <item><title>Second</title><link>https://example.com/2</link><description>desc</description></item>
        </channel></rss>"#;
        let items = parse_rss_items(xml);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].0, "Rust & Memory"); // xml_content already unescapes
        assert_eq!(items[0].1, "https://rust-lang.org/");
    }

    #[test]
    fn html_unescapes_entities() {
        assert_eq!(html_unescape("A &amp; B &lt;tag&gt;"), "A & B <tag>");
    }

    #[test]
    fn hn_hit_falls_back_to_story_fields() {
        let hit: AlgoliaHit = serde_json::from_str(
            r#"{"objectID":"1","story_title":"Ask HN: Rust?","points":42,"num_comments":7,"author":"x"}"#,
        )
        .unwrap();
        assert_eq!(hit.effective_title(), "Ask HN: Rust?");
        assert_eq!(
            hit.effective_url(),
            "https://news.ycombinator.com/item?id=1"
        );
        assert_eq!(hit.effective_description(), "42 points, 7 comments, by x");
    }
}
