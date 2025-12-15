# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/dirmacs/daedra/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/dirmacs/daedra/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/dirmacs/daedra/compare/v0.1.0...v0.1.2
[0.1.0]: https://github.com/dirmacs/daedra/releases/tag/v0.1.0
