# daedra — Agent Guidelines

## What this is

Daedra is a web search MCP server. It gives search and fetch tools to AI agents via MCP (Model Context Protocol). Thirteen backends run in a fallback chain. Circuit breakers and per-backend rate limits keep search reliable from any IP.

## Rules for agents

- Run `cargo test` before you change any file. All tests must pass.
- The fallback chain is the core value. Do not change the backend order in `SearchProvider::auto()` without a good reason.
- Each backend is independent. A new backend must not change the behavior of other backends.
- The MCP server registers tools in `src/server.rs` (`DaedraHandler::list_tools`). The handlers are `handle_web_search`, `handle_visit_page`, and `handle_crawl_site`.
- Tool names: `web_search` (primary), `search_duckduckgo` (backward-compatibility alias), `visit_page`, `crawl_site`.
- The cache layer is transparent. Backends do not know that the cache exists.
- Do not hardcode paths or API keys in source code. All configuration comes from environment variables or CLI arguments.

## Module map

| Path | Role |
|------|------|
| `src/main.rs` | CLI entry — `Commands::run`, `CheckReporter` for `daedra check` |
| `src/server.rs` | MCP server, transports, `DaedraHandler` tool dispatch |
| `src/lib.rs` | Crate root, re-exports |
| `src/url_classification.rs` | Data-driven URL → `ContentType` rules for search results |
| `src/tools/backend.rs` | `SearchProvider`, `BackendHealth` circuit breakers, `governor` keyed limiters, classified retry, aggregate failure messages |
| `src/tools/rss.rs` | Machine-format backends: Bing RSS, Google News RSS, HN Algolia |
| `src/tools/soft_block.rs` | Zero-result page classifier: genuine no-results or soft block |
| `src/tools/fetch.rs` | `FetchClient`, `FetchedContent` (Html/Pdf/Binary), dom_smoothie, infer, pdf-extract |
| `src/tools/crawl.rs` | Site crawl (sitemap + link following) |
| `src/tools/bing.rs`, `src/tools/google.rs` | HTML scraper backends |
| `src/tools/*.rs` | Other individual search backends |
| `src/cache.rs` | moka async cache layer |
| `src/types.rs` | Shared types; `detect_language` / `detect_topics` use data-driven tables |

## Reliability — do not regress

- Circuit breaker: `BackendHealth` opens after 3 consecutive failures, with a 30 second cooldown per backend name.
- Rate limits: `BackendRateLimiters` — separate keyed quotas for API, knowledge, and scraper backends.
- Classified retry: transient backend errors get one exponential-backoff retry (400 ms to 2 s). Bot protection, 403, CAPTCHA, and rate-limit errors do not retry.
- Aggregate failures: the error message names each failed backend with its error, and each backend that returned a true empty result. Do not merge these two cases into one message.
- Soft blocks: a scraper page with HTTP 200 and zero results is a soft block (an error), not an empty result. The classifier in `src/tools/soft_block.rs` decides.
- Fetch retry: `fetch_with_retry` uses exponential backoff, not a fixed sleep.

## Fetch and extraction

- HTML: dom_smoothie Readability when no CSS selector is given. The selector path uses scraper and htmd.
- PDF: `infer` detects `application/pdf`. `pdf-extract` extracts the text.
- Unknown binary: `FetchedContent::Binary` returns a clear extraction error.
