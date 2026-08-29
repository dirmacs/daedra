<p align="center">
  <img src="docs/img/daedra-logo.svg" width="128" alt="daedra">
</p>

<h1 align="center">Daedra</h1>

<p align="center">
  Self-contained web search MCP server. Rust. 13 backends. Works from any IP.<br>
  Single binary. Automatic backend fallback. Zero configuration for basic search.
</p>

<p align="center">

[![Crates.io](https://img.shields.io/crates/v/daedra.svg)](https://crates.io/crates/daedra)
[![CI](https://github.com/dirmacs/daedra/actions/workflows/ci.yml/badge.svg)](https://github.com/dirmacs/daedra/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

</p>

Daedra is a self-contained web search [MCP](https://modelcontextprotocol.io/) server in Rust. It gives search and page-fetch tools to AI agents. It works from any IP address: datacenter, VPS, or residential. Basic search needs no API keys.

## Why Daedra

Major search engines block datacenter and VPS IP addresses with CAPTCHAs. Daedra solves this with a multi-backend fallback chain. The chain tries one backend after the next until a backend returns results:

```
Serper (API) → Tavily (API) → Bing RSS → Bing → Google News → Hacker News → Google → Wikipedia → StackOverflow → GitHub → Wiby → DDG Instant → DuckDuckGo HTML
```

Three backend groups exist. API backends need a key. Scraper backends read HTML and sometimes meet a CAPTCHA. Machine-format backends read the RSS and JSON feeds that the engines publish for integrations. The machine-format backends work from any IP with no key.

Per-backend circuit breakers and per-backend rate limits keep the chain stable under load. The chain retries only transient errors. Bot protection and rate-limit errors fail fast, so the next backend starts at once.

## Features

- 13 search backends with automatic fallback (see the table below)
- Circuit breaker (`BackendHealth`): opens after repeated failures, with a 30 second cooldown
- Per-backend rate limits via `governor`, with separate quotas for API, knowledge, and scraper backends
- Classified retry: the chain retries only transient errors
- Readability extraction: `dom_smoothie` extracts the article body from HTML pages
- PDF support: `infer` detects the MIME type, `pdf-extract` extracts the text
- Content classification: `FetchedContent` (`Html` / `Pdf` / `Binary`) on every fetch
- URL classification: `src/url_classification.rs` maps search result URLs to content types
- MCP tools: `web_search`, `visit_page`, `crawl_site`, and the `search_duckduckgo` alias

## Install

```bash
cargo install daedra
```

## Search backends

| Backend | Type | API Key | Works from VPS? |
|---------|------|---------|----------------|
| Serper.dev | Google JSON API | `SERPER_API_KEY` | Yes |
| Tavily | AI-optimized API | `TAVILY_API_KEY` | Yes |
| **Bing RSS** | `format=rss` machine output | None | **Always** |
| Bing | HTML scraping | None | Sometimes (CAPTCHA risk) |
| **Google News** | News RSS feed | None | **Always** |
| **Hacker News** | Algolia JSON API | None | **Always** |
| **Google** | HTML scraping | None | Rarely (CAPTCHA risk) |
| **Wikipedia** | OpenSearch API | None | **Always** |
| **StackExchange** | Public API | None | **Always** |
| **GitHub** | Public API | None / `GITHUB_TOKEN` | **Always** |
| **Wiby** | Indie web search | None | **Always** |
| **DDG Instant** | Knowledge graph API | None | **Always** |
| DuckDuckGo | HTML scraping | None | Rarely (blocked since mid-2025) |

The provider tries the backends in order. The first backend that returns results wins.

## Usage

### MCP server (for Claude, Cursor, pawan, and similar agents)

```json
{
  "mcpServers": {
    "daedra": {
      "command": "daedra",
      "args": ["serve", "--transport", "stdio", "--quiet"]
    }
  }
}
```

### CLI

```bash
# Search
daedra search "rust async runtime" --num-results 5

# Fetch a webpage as Markdown (HTML via Readability, PDF via pdf-extract)
daedra fetch https://rust-lang.org

# Check backend health
daedra check

# Server info
daedra info
```

### As a Rust library

```rust
use daedra::tools::SearchProvider;
use daedra::types::SearchArgs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = SearchProvider::auto();
    let args = SearchArgs {
        query: "rust programming".to_string(),
        options: None,
    };
    let results = provider.search(&args).await?;
    for r in &results.data {
        println!("{} — {}", r.title, r.url);
    }
    Ok(())
}
```

## MCP tools

### `web_search`

Search the web with automatic backend fallback.

```json
{
  "query": "search terms",
  "options": {
    "region": "wt-wt",
    "safe_search": "MODERATE",
    "num_results": 10,
    "time_range": "w"
  }
}
```

`search_duckduckgo` is an alias for `web_search`. It exists for backward compatibility.

### `visit_page`

Fetch a page and extract the content as Markdown. HTML pages use `dom_smoothie` Readability extraction. The `infer` crate detects PDFs, and `pdf-extract` extracts the text.

```json
{
  "url": "https://example.com",
  "selector": "article.main",
  "include_images": false
}
```

### `crawl_site`

Crawl a site from a root URL and return Markdown for each page. The crawler reads the sitemap when one exists. Otherwise it follows links.

## Architecture

```
Daedra
├── SearchProvider (fallback chain, circuit breakers, keyed rate limits)
│   ├── SerperBackend / TavilyBackend (API, optional keys)
│   ├── BingRssBackend / GoogleNewsBackend / HnAlgoliaBackend (machine formats)
│   ├── BingBackend / GoogleBackend (HTML scraping)
│   ├── WikipediaBackend / StackExchangeBackend / GitHubBackend
│   ├── WibyBackend / DdgInstantBackend
│   └── SearchClient (DuckDuckGo HTML, last resort)
├── FetchClient (FetchedContent: Html / Pdf / Binary → Markdown)
│   ├── dom_smoothie (Readability), infer (MIME), pdf-extract (PDF)
├── soft_block (classifies zero-result scraper pages: genuine empty or bot block)
├── url_classification (search result URL → ContentType)
├── SearchCache (moka async cache)
├── MCP Server (DaedraHandler: handle_web_search, handle_visit_page, handle_crawl_site)
│   ├── STDIO transport (JSON-RPC)
│   └── SSE transport (Axum HTTP)
└── CLI (Commands::run, CheckReporter)
```

## Key dependencies

| Crate | Role |
|-------|------|
| `dom_smoothie` 0.17 | Readability article extraction |
| `infer` 0.19 | MIME detection on fetched bytes |
| `pdf-extract` 0.12 | PDF text extraction |
| `quick-xml` 0.42 | RSS parsing for the machine-format backends |
| `governor` 0.10 | Per-backend keyed rate limiting |

## Configuration

```bash
# Optional API keys (improve result quality)
export SERPER_API_KEY=...     # Google results via Serper
export TAVILY_API_KEY=...     # AI-optimized search
export GITHUB_TOKEN=...       # Higher GitHub API rate limit

# Logging
export RUST_LOG=daedra=info
```

## Ecosystem

| Project | What |
|---------|------|
| [pawan](https://github.com/dirmacs/pawan) | CLI coding agent that uses daedra for web search via MCP |
| [ares](https://github.com/dirmacs/ares) | Agentic retrieval-enhanced server |
| [eruka](https://eruka.dirmacs.com) | Context intelligence engine |

Built by [DIRMACS](https://dirmacs.com).

## License

MIT
