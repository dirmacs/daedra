# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/dirmacs/daedra/compare/v0.1.5...HEAD
[0.1.5]: https://github.com/dirmacs/daedra/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/dirmacs/daedra/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/dirmacs/daedra/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/dirmacs/daedra/compare/v0.1.0...v0.1.2
[0.1.0]: https://github.com/dirmacs/daedra/releases/tag/v0.1.0
