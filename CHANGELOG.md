# Changelog

This file lists the notable changes to this project.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/). This project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-08-29

### Added
- Three general-web backends with no API key: Mojeek (independent index, HTML), Brave (HTML), and Marginalia (public API, answers from any IP). The chain is now 16 backends, 14 without keys.
- Cross-engine corroboration in the merge: the same page found by two engines outranks an otherwise equal single-engine hit. Dedup now keys on the canonical URL (host without www, no trailing slash, tracking parameters stripped), so `example.com/x` and `www.example.com/x/?utm_source=y` are one result.

### Changed
- The unkeyed search story is now stated plainly in README and `check`: Marginalia answers from any IP; Mojeek and Brave serve residential IPs and refuse datacenter ones; the circuit breaker sidelines them where refused.


## [0.3.14] - 2026-08-29

### Added
- `-f json` and `-f json-compact` now work on `info` and `check`. `info` lists the real four MCP tools; `check` prints the same report machine-readably.
- `check` reports missing `SERPER_API_KEY`, `TAVILY_API_KEY`, and `GITHUB_TOKEN`, and says plainly that unkeyed search is not general web search.
- `fetch --timeout SECONDS` sets a per-request timeout. The default path is unchanged.
- SVG, XML, and JSON responses arrive verbatim in a code fence. They no longer fail with "Unsupported content type".

### Fixed
- `serve --transport sse --host localhost` works. The host parser resolves hostnames to IPv4 instead of accepting dotted quads only.

### Docs
- The README no longer claims "first backend that returns results wins". It describes the relevance-ranked merge, names the 13 backends, and states the unkeyed search scope honestly.

## [0.3.13] - 2026-08-29

## [0.3.13] - 2026-08-29

### Fixed
- `crawl` always fetches the root page, even when sitemap or anchor discovery found candidates. A page that links only off-origin now yields a crawl result instead of a silent `0 pages` success.
- `crawl` exits non-zero when it fetched zero pages. The summary still prints first.
- Sitemap indexes (nested `.xml` sitemaps) expand one level during discovery, capped at 20 children.
- The pretty crawl teaser slices by character, not byte, so multi-byte pages can no longer panic.

### Added
- `crawl --depth N` follows same-origin links from fetched pages, up to 5 layers.
- `crawl --delay-ms N` staggers fetch starts (politeness for small sites).
- `robots.txt` is fetched and honored (longest-match rule, `Allow` beats `Disallow`, `*` group). Crawl fails open when robots.txt is absent. `--ignore-robots` opts out. Same fields on the MCP `crawl_site` tool.

## [0.3.12] - 2026-08-29

## [0.3.12] - 2026-08-29

### Changed
- Search results are ranked by relevance to the query before merging. Results that share no query token land after every matched result; a full-phrase title match outranks a word-bag match. When no result matches the query at all, the unrelated feed noise is discarded and the search reports it instead of returning it.
- Google News descriptions are stripped of embedded HTML markup and `nbsp` entities.

### Added
- `--backend` and `--exclude` flags on `search` (plus `backends` and `exclude_backends` in `SearchOptions`) to select or skip backends.
- `--time-range` now filters results on the backends that support it: Serper, Tavily, Hacker News (numeric date filter), and Google News (`when:` operator). Backends without recency support ignore it; invalid letters are rejected.

### Fixed
- `-v`, `-q`, and `RUST_LOG` now apply to every command, not only `serve`. All non-serve output logs to stderr, so JSON output stays clean.
- Empty queries and `--num-results 0` are rejected with a clear error, matching the MCP schema.

## [0.3.11] - 2026-08-29

## [0.3.11] - 2026-08-29

### Fixed
- The docker build copies the bench and example targets into the builder stage. The 0.3.10 image build failed because the manifest declares those targets and the files were missing.

## [0.3.10] - 2026-08-29

### Added
- Dependabot config (cargo and github-actions, weekly) so dependency bumps arrive before the CI audit gate blocks a release.

### Fixed
- The release pipeline now works end to end:
  - `Dockerfile` builds in a multi-stage build. The old image copied a host-built binary that never exists on a clean checkout, so every docker build failed.
  - The release workflow declares `permissions: contents: write`. Without it, the release-asset upload failed with "Resource not accessible by integration", and the binary-cancel cascade left releases with no assets.
  - The binary-build matrix no longer cancels the other four legs when one leg fails.
  - All workflow toolchains pinned to 1.98.

### Changed
- Removed the unmaintained `backoff` crate (and its `instant` dependency). The fetch and DDG-search retry loops now use inline exponential backoff with the same timing. Unmaintained-crate warnings in `cargo audit` drop from 8 to 3.

## [0.3.9] - 2026-08-29

## [0.3.9] - 2026-08-29

### Changed

- The minimum supported Rust version is now 1.98 (the toolchain that the rest of the DIRMACS Rust stack uses). The `msrv` CI job checks 1.98.
- All documentation is now written in ASD-STE100 Simplified Technical English. Code, identifiers, commands, and file paths did not change.

### Fixed

- The README, CONTRIBUTING.md, AGENTS.md, CLAUDE.md, and the site index said "9 backends" or "10 backends". They now say 13. The key-dependency tables now list `pdf-extract` 0.12 and `quick-xml` 0.42.

## [0.3.8] - 2026-08-29

### Fixed

- quick-xml 0.38 → 0.42. Version 0.38.4 has two advisories (RUSTSEC-2026-0194 and RUSTSEC-2026-0195). The RSS parser now uses the 0.42 API.

## [0.3.7] - 2026-08-29

### Added

- Three bot-tolerant machine-format backends. They read the RSS and JSON feeds that the engines publish for integrations. No keys. No challenge pages. Tests verified them from datacenter IPs.
  - **Bing RSS** (`bing-rss`): the regular Bing index via `format=rss`. The chain tries it before HTML scraping.
  - **Google News** (`gnews`): the Google News RSS feed, with region mapping.
  - **Hacker News** (`hn`): the public HN Algolia JSON API. Strong for technical queries.
- The fallback chain now has 13 backends. Most queries succeed even when SERP scraping is blocked.

## [0.3.6] - 2026-08-29

### Fixed

- Scraper backends (Bing, Google, DuckDuckGo) now classify an HTTP 200 page with zero results. The new `soft_block` classifier decides: a genuine no-results page stays an empty result, an anti-bot or consent page becomes an error. Aggregate failures now name the real cause.
- Each backend knows its own no-results marker. Bing knows "There are no results for". Google knows "did not match any documents". DuckDuckGo knows "No results". A true no-match query still reports empty.
- Google now treats `/sorry/` redirect URLs as CAPTCHA pages.

## [0.3.5] - 2026-08-29

### Fixed

- Aggregate failure errors now separate two cases: backends that failed (with the per-backend error) and backends that returned a true empty result. The old message reported both as "0 results" and hid rate limits and CAPTCHAs.

## [0.3.4] - 2026-08-29

### Fixed

- The manifest now requires pdf-extract 0.12, which depends on lopdf ^0.42. Consumers no longer resolve the vulnerable lopdf 0.38 line.

## [0.3.3] - 2026-08-29

### Added

- The Google HTML search backend (`google`). It sits after Bing in the chain. On a CAPTCHA it fails fast with `BotProtectionDetected`, so the next backend takes over. Closes #9.
- Every backend now honors the SafeSearch option. Bing uses `adlt`. Serper uses `safeSearch`. Google uses `safe`. DuckDuckGo already used `kp`.
- Region mapping for the API and scraper backends. Serper and Google receive `gl` and `hl`. Bing receives `mkt`. The values come from the DDG-style region tag.

### Fixed

- All `cargo clippy --all-targets -D warnings` findings. The drift predated this release: collapsible ifs, borrowed boxes, missing `Default` impls, clamp patterns, and the deprecated `criterion::black_box`.

## [0.1.6] - 2026-02-01

### Changed

- The `mcp_server` example now shows correct stderr logging for the STDIO transport.
- CONTRIBUTING.md now lists the new test file in the project structure.

### Documentation

- The README now documents the STDIO transport logging behavior.
- The README now documents the `--quiet` flag.
- The example code shows a transport-aware logging setup.

## [0.1.5] - 2026-02-01

### Fixed

- STDIO transport: all log output now goes to stderr, not stdout. This prevents JSON-RPC stream corruption (#4).
- STDIO transport: the server no longer prints the decorative banner on the STDIO transport.
- MCP protocol: the server handles `notifications/initialized` (with prefix) as a no-op. It no longer returns "Method not found".

### Added

- The new `--quiet` / `-q` flag disables all logging output. Useful for the STDIO transport.
- An integration test suite for the STDIO transport, with 19 new tests:
  - Protocol compliance tests (stdout purity, no ANSI codes, JSON-RPC structure)
  - MCP handshake tests (initialize, initialized, tools/list, ping)
  - Tool execution tests (search_duckduckgo, visit_page)
  - Error handling tests (malformed JSON, invalid params, unknown methods)

## [0.1.4] - 2026-01-21

### Fixed

- html2md → htmd. html2md had JNI dependencies, and those dependencies broke Android/Termux builds.

## [0.1.3] - 2025-12-15

### Fixed

- The publish workflow failed because version 0.1.2 already existed on crates.io. The version bump fixed the workflow.

## [0.1.2] - 2025-12-15

### Added

- First release of the Daedra MCP server:
  - Web search with DuckDuckGo
  - Page fetching with content extraction to Markdown
  - STDIO transport for MCP clients
  - SSE (HTTP) transport for web clients
  - Response caching with a configurable TTL
  - CLI with colored output and multiple output formats
  - Parallel search execution
  - Test suite
  - Benchmark suite
  - Docker support
  - GitHub Actions CI/CD workflows

### Tools

- `search_duckduckgo`: search the web with DuckDuckGo.
  - Region settings
  - Safe search levels (Off / Moderate / Strict)
  - Result count from 1 to 50
  - Time range filter (day / week / month / year)
  - Content type detection
  - Language detection
  - Topic analysis

- `visit_page`: fetch a page and extract its content.
  - HTML to Markdown conversion
  - CSS selector for targeted extraction
  - Bot protection detection
  - Link extraction
  - Word count

### CLI commands

- `serve`: start the MCP server (STDIO or SSE)
- `search`: run a direct web search
- `fetch`: fetch and show page content
- `info`: show server information
- `check`: test the configuration and the backend connectivity

## [0.1.0] - 2025-01-XX

### Added

- First public release.

[Unreleased]: https://github.com/dirmacs/daedra/compare/v0.3.8...HEAD
[0.3.8]: https://github.com/dirmacs/daedra/compare/v0.3.7...v0.3.8
[0.3.7]: https://github.com/dirmacs/daedra/compare/v0.3.6...v0.3.7
[0.3.6]: https://github.com/dirmacs/daedra/compare/v0.3.5...v0.3.6
[0.3.5]: https://github.com/dirmacs/daedra/compare/v0.3.4...v0.3.5
[0.3.4]: https://github.com/dirmacs/daedra/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/dirmacs/daedra/compare/v0.1.6...v0.3.3
[0.1.6]: https://github.com/dirmacs/daedra/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/dirmacs/daedra/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/dirmacs/daedra/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/dirmacs/daedra/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/dirmacs/daedra/compare/v0.1.0...v0.1.2
[0.1.0]: https://github.com/dirmacs/daedra/releases/tag/v0.1.0
