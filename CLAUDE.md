# daedra

Self-contained web search MCP server. 9 backends, automatic fallback, works from any IP. No API keys required for basic search.

## Build & Test

```bash
cargo build --release
cargo test
cargo clippy -- -D warnings
```

## Architecture

Single crate with modular backends: Serper, Tavily, Bing, Wikipedia, StackExchange, GitHub, Wiby, DDG Instant, DuckDuckGo HTML. `SearchProvider` runs the fallback chain with per-backend circuit breakers (`BackendHealth`), governor keyed rate limits, and classified retry on transient errors only.

`FetchClient` classifies responses as `FetchedContent` (Html / Pdf / Binary): HTML uses dom_smoothie Readability extraction, PDFs use infer + pdf-extract, binary types fail with a typed error.

MCP server (`DaedraHandler` in `server.rs`) exposes `web_search`, `visit_page`, `crawl_site`; `search_duckduckgo` is a backward-compat alias for `web_search`. STDIO and SSE transports. Results cached via moka async cache.

## Key Files

- `src/main.rs` — CLI entrypoint (`Commands::run`, `CheckReporter` for health checks)
- `src/server.rs` — MCP server, Axum HTTP/SSE, tool handler methods
- `src/lib.rs` — Crate root and re-exports
- `src/url_classification.rs` — Search result URL → content type (data-driven rules)
- `src/tools/backend.rs` — SearchProvider, circuit breakers, rate limiters, fallback chain
- `src/tools/fetch.rs` — Page fetch, Readability, PDF, MIME classification
- `src/tools/crawl_site.rs` — Multi-page crawl tool
- `src/tools/` — Individual backend implementations
- `src/cache.rs` — moka async cache layer
- `src/types.rs` — Shared schemas; language/topic detection tables

## Conventions

- Git author: `bkataru <baalateja.k@gmail.com>`
- No hardcoded paths — all config via env vars or CLI args
- Async runtime: tokio
- Release profile: LTO, codegen-units=1, strip=true
- New extraction deps: `dom_smoothie` 0.17, `infer` 0.19, `pdf-extract` 0.10; rate limiting via `governor` 0.10
