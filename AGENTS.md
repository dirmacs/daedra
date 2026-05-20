# daedra — Agent Guidelines

## What This Is

Daedra is a web search MCP server. It provides search and fetch tools to AI agents via MCP (Model Context Protocol). Nine backends with automatic fallback, circuit breakers, and per-backend rate limits keep search reliable from any IP.

## For Agents

- Run `cargo test` before any changes
- The search fallback chain is the core value — don't break backend ordering in `SearchProvider::auto()` without good reason
- Each backend is independent — adding a new one shouldn't affect others
- MCP tools are registered in `src/server.rs` (`DaedraHandler::list_tools`); handlers are `handle_web_search`, `handle_visit_page`, `handle_crawl_site`
- Tool names: `web_search` (primary), `search_duckduckgo` (backward-compat alias), `visit_page`, `crawl_site`
- Caching is transparent — backends don't know about the cache layer
- No hardcoded paths or API keys in source code

## Module map (post-refactor)

| Path | Role |
|------|------|
| `src/main.rs` | CLI entry — `Commands::run`, `CheckReporter` for `daedra check` |
| `src/server.rs` | MCP server, transports, `DaedraHandler` tool dispatch |
| `src/lib.rs` | Crate root, re-exports |
| `src/url_classification.rs` | Data-driven URL → `ContentType` rules for search results |
| `src/tools/backend.rs` | `SearchProvider`, `BackendHealth` circuit breakers, `governor` keyed limiters, classified retry |
| `src/tools/fetch.rs` | `FetchClient`, `FetchedContent` (Html/Pdf/Binary), dom_smoothie, infer, pdf-extract |
| `src/tools/crawl.rs` | Site crawl (sitemap + link following) |
| `src/tools/*.rs` | Individual search backends |
| `src/cache.rs` | moka async cache layer |
| `src/types.rs` | Shared types; `detect_language` / `detect_topics` use data-driven tables |

## Reliability (don't regress)

- **Circuit breaker**: `BackendHealth` opens after 3 consecutive failures, 30s cooldown per backend name
- **Rate limits**: `BackendRateLimiters` — separate keyed quotas for API, knowledge, and scraper backends
- **Classified retry**: transient backend errors get one exponential-backoff retry (400ms–2s); bot protection, 403, CAPTCHA, and rate limits do not retry
- **Fetch retry**: `fetch_with_retry` uses exponential backoff (not a fixed 500ms sleep)

## Fetch / extraction

- HTML: dom_smoothie Readability when no CSS selector; selector path uses scraper + htmd
- PDF: `infer` detects `application/pdf`, `pdf-extract` extracts text
- Unknown binary: `FetchedContent::Binary` returns a clear extraction error
