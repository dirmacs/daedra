<p align="center">
  <img src="docs/img/daedra-logo.svg" width="128" alt="daedra">
</p>

<h1 align="center">Daedra</h1>

<p align="center">
  Self-contained web search MCP server. Rust. 7 backends. Works from any IP.<br>
  No API keys required. No Docker. No Python.
</p>

<p align="center">

[![Crates.io](https://img.shields.io/crates/v/daedra.svg)](https://crates.io/crates/daedra)
[![CI](https://github.com/dirmacs/daedra/actions/workflows/ci.yml/badge.svg)](https://github.com/dirmacs/daedra/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
![Tests](https://img.shields.io/badge/tests-39-brightgreen.svg)
![Backends](https://img.shields.io/badge/search_backends-7-blue.svg)

**Daedra** is a self-contained web search [MCP](https://modelcontextprotocol.io/) server written in Rust. 7 search backends with automatic fallback. Works from any IP — datacenter, VPS, residential. No API keys required for basic search.

## Why Daedra?

Every major search engine (Google, Bing, DuckDuckGo, Brave) blocks datacenter/VPS IPs with CAPTCHAs since 2025. Daedra solves this with a **multi-backend fallback chain** that automatically finds a backend that works:

```
Serper (API) → Tavily (API) → Bing → Wikipedia → StackOverflow → GitHub → DuckDuckGo
```

No Docker. No Python. No SearXNG. Pure Rust. Daedra IS the search infrastructure.

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

# Fetch a webpage as Markdown
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

Fetch and extract web page content as Markdown.

```json
{
  "url": "https://example.com",
  "selector": "article.main",
  "include_images": false
}
```

## Architecture

```
Daedra
├── SearchProvider (fallback chain)
│   ├── SerperBackend      (Google via API)
│   ├── TavilyBackend      (AI-optimized API)
│   ├── BingBackend         (HTML scraping)
│   ├── WikipediaBackend    (OpenSearch API)
│   ├── StackExchangeBackend (Public API)
│   ├── GitHubBackend       (Public API)
│   └── SearchClient        (DuckDuckGo HTML)
├── FetchClient (HTML → Markdown)
├── SearchCache (moka async cache)
├── MCP Server
│   ├── STDIO transport (JSON-RPC)
│   └── SSE transport (Axum HTTP)
└── CLI (clap)
```

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
| [eruka](https://github.com/dirmacs/eruka) | Context intelligence engine |

Built by [DIRMACS](https://dirmacs.com).

## License

MIT
