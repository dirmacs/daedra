<p align="center">
  <img src="docs/img/daedra-logo.svg" width="128" alt="daedra">
</p>

<h1 align="center">Daedra</h1>

<p align="center">
  Self-contained web search MCP server. Rust. 9 backends. Works from any IP.<br>
  Single binary. Automatic backend fallback. Zero configuration for basic search.
</p>

<p align="center">

[![Crates.io](https://img.shields.io/crates/v/daedra.svg)](https://crates.io/crates/daedra)
[![CI](https://github.com/dirmacs/daedra/actions/workflows/ci.yml/badge.svg)](https://github.com/dirmacs/daedra/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**Daedra** (v0.3.0) is a self-contained web search [MCP](https://modelcontextprotocol.io/) server written in Rust. Multiple search backends with automatic fallback. Works from any IP — datacenter, VPS, residential. No API keys required for basic search.

## Why Daedra?

Every major search engine (Google, Bing, DuckDuckGo, Brave) blocks datacenter/VPS IPs with CAPTCHAs since 2025. Daedra solves this with a **multi-backend fallback chain** that automatically finds a backend that works:

```
Serper (API) → Tavily (API) → Bing → Wikipedia → StackOverflow → GitHub → Wiby → DDG Instant → DuckDuckGo HTML
```

Pure Rust. If one backend is blocked or rate-limited, the next one takes over automatically. Per-backend **circuit breakers** and **governor** rate limits keep the chain stable under load; transient failures get a **classified retry** (exponential backoff, not a blind fixed delay).

## Features

- **9 search backends** with automatic fallback (see table below)
- **Circuit breaker** (`BackendHealth`) — opens after repeated failures, 30s cooldown
- **Per-backend keyed rate limiting** via `governor` (API vs knowledge vs scraper tiers)
- **Classified retry** — only transient errors are retried; bot protection and rate limits fail fast
- **Readability extraction** — `dom_smoothie` article body extraction for HTML pages
- **PDF support** — `infer` MIME sniffing + `pdf-extract` text extraction
- **Content classification** — `FetchedContent` enum (`Html` / `Pdf` / `Binary`) on fetch
- **URL classification** — `src/url_classification.rs` maps search result URLs to content types
- **MCP tools** — `web_search`, `visit_page`, `crawl_site` (+ `search_duckduckgo` alias)

## Install

```bash
cargo install daedra
```

## Search backends

| Backend | Type | API Key | Works from VPS? |
|---------|------|---------|----------------|
| Serper.dev | Google JSON API | `SERPER_API_KEY` | Yes |
| Tavily | AI-optimized API | `TAVILY_API_KEY` | Yes |
| Bing | HTML scraping | None | Sometimes (CAPTCHA risk) |
| **Wikipedia** | OpenSearch API | None | **Always** |
| **StackExchange** | Public API | None | **Always** |
| **GitHub** | Public API | None / `GITHUB_TOKEN` | **Always** |
| **Wiby** | Indie web search | None | **Always** |
| **DDG Instant** | Knowledge graph API | None | **Always** |
| DuckDuckGo | HTML scraping | None | Rarely (blocked since mid-2025) |

Backends are tried in order. First one that returns results wins.

## Usage

### MCP Server (for Claude, Cursor, pawan, etc.)

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

## MCP Tools

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

Aliases: `search_duckduckgo` (backward compat)

### `visit_page`

Fetch and extract page content as Markdown. HTML pages use **dom_smoothie** Readability extraction; PDFs are detected via **infer** and text is extracted with **pdf-extract**.

```json
{
  "url": "https://example.com",
  "selector": "article.main",
  "include_images": false
}
```

### `crawl_site`

Crawl a site from a root URL (sitemap or link following), returning Markdown per page.

## Architecture

```
Daedra
├── SearchProvider (fallback chain, circuit breakers, keyed rate limits)
│   ├── SerperBackend / TavilyBackend (API, optional keys)
│   ├── BingBackend (HTML scraping)
│   ├── WikipediaBackend / StackExchangeBackend / GitHubBackend
│   ├── WibyBackend / DdgInstantBackend
│   └── SearchClient (DuckDuckGo HTML, last resort)
├── FetchClient (FetchedContent: Html / Pdf / Binary → Markdown)
│   ├── dom_smoothie (Readability), infer (MIME), pdf-extract (PDF)
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
| `infer` 0.19 | MIME sniffing on fetched bytes |
| `pdf-extract` 0.10 | PDF text extraction |
| `governor` 0.10 | Per-backend keyed rate limiting |

## Configuration

```bash
# Optional API keys (improves result quality)
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
