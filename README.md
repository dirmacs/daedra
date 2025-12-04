# 🔍 Daedra

[![Crates.io](https://img.shields.io/crates/v/daedra.svg)](https://crates.io/crates/daedra)
[![Documentation](https://docs.rs/daedra/badge.svg)](https://docs.rs/daedra)
[![CI](https://github.com/dirmacs/daedra/actions/workflows/ci.yml/badge.svg)](https://github.com/dirmacs/daedra/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

**Daedra** is a high-performance web search and research [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) server written in Rust. It provides web search and page fetching capabilities that can be used with AI assistants like Claude.

## Features

- 🔎 **Web Search**: Search the web using DuckDuckGo with customizable options
- 📄 **Page Fetching**: Extract and convert web page content to Markdown
- 🚀 **High Performance**: Built in Rust with async I/O and connection pooling
- 💾 **Caching**: Built-in response caching for improved performance
- 🔌 **Dual Transport**: Support for both STDIO and HTTP (SSE) transports
- 📦 **Library & CLI**: Use as a Rust library or standalone command-line tool

## Installation

### From crates.io

```bash
cargo install daedra
```

### From source

```bash
git clone https://github.com/dirmacs/daedra.git
cd daedra
cargo install --path .
```

### Using Cargo

Add to your `Cargo.toml`:

```toml
[dependencies]
daedra = "0.1"
```

## Quick Start

### As an MCP Server

#### STDIO Transport (for Claude Desktop)

Add to your Claude Desktop configuration (`claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "daedra": {
      "command": "daedra",
      "args": ["serve", "--transport", "stdio"]
    }
  }
}
```

#### SSE Transport (HTTP)

```bash
daedra serve --transport sse --port 3000 --host 127.0.0.1
```

### As a CLI Tool

#### Search the web

```bash
# Basic search
daedra search "rust programming"

# With options
daedra search "rust async" --num-results 20 --region us-en --safe-search moderate

# Output as JSON
daedra search "rust web frameworks" --format json
```

#### Fetch a webpage

```bash
# Fetch and extract content
daedra fetch https://rust-lang.org

# Fetch with a specific selector
daedra fetch https://example.com --selector "article.main"

# Output as JSON
daedra fetch https://example.com --format json
```

#### Server information

```bash
daedra info
```

#### Configuration check

```bash
daedra check
```

### As a Rust Library

```rust
use daedra::{DaedraServer, ServerConfig, TransportType};
use daedra::tools::{search, fetch};
use daedra::types::{SearchArgs, SearchOptions, VisitPageArgs};

// Start an MCP server
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ServerConfig::default();
    let server = DaedraServer::new(config)?;
    server.run(TransportType::Stdio).await?;
    Ok(())
}

// Or use tools directly
async fn search_example() -> anyhow::Result<()> {
    let args = SearchArgs {
        query: "rust programming".to_string(),
        options: Some(SearchOptions {
            num_results: 10,
            region: "wt-wt".to_string(),
            ..Default::default()
        }),
    };

    let results = search::perform_search(&args).await?;
    println!("Found {} results", results.data.len());

    for result in results.data {
        println!("- {} ({})", result.title, result.url);
    }

    Ok(())
}

async fn fetch_example() -> anyhow::Result<()> {
    let args = VisitPageArgs {
        url: "https://rust-lang.org".to_string(),
        selector: None,
        include_images: false,
    };

    let content = fetch::fetch_page(&args).await?;
    println!("Title: {}", content.title);
    println!("Word count: {}", content.word_count);

    Ok(())
}
```

## MCP Tools

### `search_duckduckgo`

Search the web using DuckDuckGo.

**Input Schema:**

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

**Options:**
- `region`: Search region (e.g., `us-en`, `wt-wt` for worldwide)
- `safe_search`: `OFF`, `MODERATE`, or `STRICT`
- `num_results`: Number of results (1-50)
- `time_range`: Time filter (`d`=day, `w`=week, `m`=month, `y`=year)

### `visit_page`

Fetch and extract content from a web page.

**Input Schema:**

```json
{
  "url": "https://example.com",
  "selector": "article.main",
  "include_images": false
}
```

**Options:**
- `url`: URL to fetch (required)
- `selector`: CSS selector for specific content (optional)
- `include_images`: Include image references (default: false)

## Configuration

### Environment Variables

- `RUST_LOG`: Set logging level (`debug`, `info`, `warn`, `error`)

### CLI Options

```
daedra serve [OPTIONS]

Options:
  -t, --transport <TRANSPORT>  Transport type [default: stdio] [possible values: stdio, sse]
  -p, --port <PORT>            Port for SSE transport [default: 3000]
      --host <HOST>            Host to bind to [default: 127.0.0.1]
      --no-cache               Disable result caching
      --cache-ttl <SECONDS>    Cache TTL in seconds [default: 300]
  -v, --verbose                Enable verbose output
  -f, --format <FORMAT>        Output format [default: pretty] [possible values: pretty, json, json-compact]
      --no-color               Disable colored output
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        CLI Binary                           │
│  (clap argument parsing, colored output, TUI)               │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                     Library (daedra)                         │
├─────────────────────────────────────────────────────────────┤
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐       │
│  │   Server     │  │    Tools     │  │    Cache     │       │
│  │  (rmcp MCP)  │  │ (search/     │  │   (moka)     │       │
│  │              │  │  fetch)      │  │              │       │
│  └──────────────┘  └──────────────┘  └──────────────┘       │
├─────────────────────────────────────────────────────────────┤
│  Transport Layer: STDIO | SSE (HTTP)                         │
└─────────────────────────────────────────────────────────────┘
```

## Performance

Daedra is designed for high performance:

- **Async I/O**: Built on Tokio for efficient async operations
- **Connection Pooling**: HTTP connections are pooled and reused
- **Caching**: Results are cached to avoid redundant requests
- **Concurrent Processing**: Parallel search execution support
- **Efficient Parsing**: Fast HTML parsing with the `scraper` crate

## Development

### Prerequisites

- Rust 1.75 or later
- Cargo

### Building

```bash
# Debug build
cargo build

# Release build
cargo build --release
```

### Testing

```bash
# Run all tests
cargo test

# Run unit tests only
cargo test --lib

# Run integration tests (requires network)
cargo test -- integration

# Run with logging
RUST_LOG=debug cargo test
```

### Benchmarks

```bash
cargo bench
```

### Documentation

```bash
# Generate and open documentation
cargo doc --open
```

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

### Code Style

This project uses:
- `rustfmt` for formatting
- `clippy` for linting

Run before committing:

```bash
cargo fmt
cargo clippy -- -D warnings
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Related Projects

- [rmcp](https://github.com/anthropics/rmcp) - Rust MCP SDK
- [mcp-duckduckresearch](https://github.com/bkataru-workshop/mcp-duckduckresearch) - TypeScript inspiration
- [DIRMACS](https://dirmacs.com) - Parent organization

## Acknowledgments

- [Anthropic](https://anthropic.com) for the Model Context Protocol
- [DuckDuckGo](https://duckduckgo.com) for the search service
- The Rust community for excellent crates

---

Made with ❤️ by [DIRMACS Global Services](https://dirmacs.com)
