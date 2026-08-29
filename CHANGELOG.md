# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.7] - 2026-08-29

### Added
- Three bot-tolerant machine-format backends — same engines, no challenge pages, no keys, verified from datacenter IPs:
  - **Bing RSS** (`bing-rss`): the regular Bing index via `format=rss`, tried before HTML scraping
  - **Google News** (`gnews`): Google News RSS feed with region mapping
  - **Hacker News** (`hn`): the public HN Algolia JSON API, strong for technical queries
- Fallback chain is now 13 backends; the bot-tolerant tier means most queries succeed even when SERP scraping is blocked

## [0.3.6] - 2026-08-29

### Fixed
- Scraper backends (Bing, Google, DuckDuckGo) now classify HTTP-200 zero-result anti-bot/consent pages as **soft blocks** (errors) instead of legitimate empty results — aggregate failures now name the real cause
- Each backend recognizes its own genuine no-results marker (Bing "There are no results for", Google "did not match any documents", DDG "No results") so true no-match queries still report empty
- Google additionally treats `/sorry/` redirect URLs as CAPTCHA; new `soft_block` module centralizes the classifier

## [0.3.5] - 2026-08-29

### Fixed
- Aggregate search-failure errors now distinguish backends that **failed** (with per-backend error detail) from backends that legitimately returned 0 results — the old message conflated the two, hiding rate limits/CAPTCHAs behind a "0 results" claim

## [0.3.4] - 2026-08-29

### Fixed
- Dependency manifest: pdf-extract 0.12 (patched lopdf ^0.42) so downstream consumers stop resolving the vulnerable lopdf 0.38 train

## [0.3.3] - 2026-08-29

### Added
- Google HTML search backend (`google`) in the fallback chain after Bing, with CAPTCHA fail-fast (`BotProtectionDetected`) so the next backend takes over (closes #9)
- SafeSearch is now honored on every backend: Bing (`adlt`), Serper (`safeSearch`), and Google (`safe`); DuckDuckGo already supported `kp`
- Region mapping for API/scrape backends: Serper/Google receive `gl`/`hl`, Bing receives `mkt` from the DDG-style region tag

### Fixed
- All `cargo clippy --all-targets -D warnings` findings (pre-existing drift: collapsible ifs, borrowed boxes, missing `Default` impls, clamp patterns, deprecated `criterion::black_box`)

## [0.1.6] - 2026-02-01

### Changed
- Updated `mcp_server` example to demonstrate proper stderr logging for STDIO transport
- Updated CONTRIBUTING.md with new test file in project structure

### Documentation
- Added notes about STDIO transport logging behavior to README
- Added `--quiet` flag documentation
- Improved example code with transport-aware logging setup

## [0.1.5] - 2026-02-01

### Fixed
- **stdio transport**: Route all log output to stderr instead of stdout to prevent JSON-RPC stream corruption (#4)
- **stdio transport**: Suppress decorative banner when using stdio transport
- **MCP protocol**: Handle `notifications/initialized` method (with prefix) as a no-op instead of returning "Method not found"

### Added
- New `--quiet` / `-q` flag to disable all logging output (useful for stdio transport)
- Comprehensive stdio transport integration test suite (19 new tests)
  - Protocol compliance tests (stdout purity, no ANSI codes, JSON-RPC structure)
  - MCP handshake tests (initialize, initialized, tools/list, ping)
  - Tool execution tests (search_duckduckgo, visit_page)
  - Error handling tests (malformed JSON, invalid params, unknown methods)

## [0.1.4] - 2026-01-21

### Fixed
- Replaced html2md with htmd to fix Android/Termux builds (html2md had JNI dependencies that caused build failures on Android)

## [0.1.3] - 2025-12-15

### Fixed
- Fixed publish workflow by bumping version (0.1.2 already existed on crates.io)

## [0.1.2] - 2025-12-15

### Added
- Initial release of Daedra MCP server
- Web search using DuckDuckGo
- Page fetching with content extraction to Markdown
- STDIO transport for MCP clients
- SSE (HTTP) transport for web-based clients
- Built-in response caching with configurable TTL
- CLI with colored output and multiple output formats
- Parallel search execution support
- Comprehensive test suite
- Benchmark suite for performance testing
- Docker support
- GitHub Actions CI/CD workflows

### Tools
- `search_duckduckgo`: Search the web using DuckDuckGo
  - Customizable region settings
  - Safe search filtering (Off/Moderate/Strict)
  - Configurable result count (1-50)
  - Time range filtering (day/week/month/year)
  - Content type detection
  - Language detection
  - Topic analysis

- `visit_page`: Fetch and extract webpage content
  - HTML to Markdown conversion
  - CSS selector support for targeted extraction
  - Bot protection detection
  - Link extraction
  - Word count analysis

### CLI Commands
- `serve`: Start the MCP server (STDIO or SSE)
- `search`: Perform a direct web search
- `fetch`: Fetch and display webpage content
- `info`: Show server information
- `check`: Validate configuration and connectivity

## [0.1.0] - 2025-01-XX

### Added
- Initial public release

[Unreleased]: https://github.com/dirmacs/daedra/compare/v0.1.6...HEAD
[0.1.6]: https://github.com/dirmacs/daedra/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/dirmacs/daedra/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/dirmacs/daedra/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/dirmacs/daedra/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/dirmacs/daedra/compare/v0.1.0...v0.1.2
[0.1.0]: https://github.com/dirmacs/daedra/releases/tag/v0.1.0
