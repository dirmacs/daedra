# daedra

Self-contained web search MCP server. 13 backends with automatic fallback. Works from any IP. No API keys are required for basic search.

## Build and test

```bash
cargo build --release
cargo test
cargo clippy -- -D warnings
```

## Architecture

Single crate with modular backends: Serper, Tavily, Bing RSS, Bing, Google News, Hacker News, Google, Wikipedia, StackExchange, GitHub, Wiby, DDG Instant, DuckDuckGo HTML. `SearchProvider` runs the fallback chain. Per-backend circuit breakers (`BackendHealth`), governor keyed rate limits, and classified retry (transient errors only) keep the chain stable.

`FetchClient` classifies responses as `FetchedContent` (Html / Pdf / Binary). HTML uses dom_smoothie Readability extraction. PDFs use infer + pdf-extract. Binary types fail with a typed error.

The MCP server (`DaedraHandler` in `server.rs`) exposes `web_search`, `visit_page`, and `crawl_site`. `search_duckduckgo` is a backward-compatibility alias for `web_search`. Transports: STDIO and SSE (Axum). The moka async cache layer caches results.

## Key files

- `src/main.rs` — CLI entrypoint (`Commands::run`, `CheckReporter` for health checks)
- `src/server.rs` — MCP server, Axum HTTP/SSE, tool handler methods
- `src/lib.rs` — Crate root and re-exports
- `src/url_classification.rs` — Search result URL → content type (data-driven rules)
- `src/tools/backend.rs` — SearchProvider, circuit breakers, rate limiters, fallback chain, aggregate failure messages
- `src/tools/rss.rs` — Machine-format backends: Bing RSS, Google News, HN Algolia
- `src/tools/soft_block.rs` — Zero-result page classifier for scraper backends
- `src/tools/fetch.rs` — Page fetch, Readability, PDF, MIME classification
- `src/tools/crawl.rs` — Multi-page crawl tool
- `src/tools/` — Individual backend implementations
- `src/cache.rs` — moka async cache layer
- `src/types.rs` — Shared schemas. Language and topic detection tables.

## Conventions

- Git author: `bkataru <baalateja.k@gmail.com>`
- No hardcoded paths. All configuration comes from environment variables or CLI arguments.
- Async runtime: tokio
- Release profile: LTO, codegen-units=1, strip=true
- Extraction deps: `dom_smoothie` 0.17, `infer` 0.19, `pdf-extract` 0.12. RSS parsing: `quick-xml` 0.42. Rate limiting: `governor` 0.10.
- MSRV: Rust 1.98 (same toolchain as the rest of the DIRMACS Rust stack)
