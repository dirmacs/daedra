# daedra

Self-contained web search MCP server. 9 backends, automatic fallback, works from any IP. No API keys required for basic search.

## Build & Test

```bash
cargo build --release
cargo test
cargo clippy -- -D warnings
```

## Architecture

Single crate with modular backends: Serper, Tavily, Bing, Wikipedia, StackExchange, GitHub, DuckDuckGo, and more. MCP server supports both STDIO and SSE transports. Results cached via moka async cache.

## Key Files

- `src/main.rs` — CLI entrypoint + MCP server setup
- `src/server.rs` — Axum HTTP/SSE server
- `src/lib.rs` — Search engine, backend orchestration, fallback chain
- `src/tools/` — MCP tool definitions
- `src/cache.rs` — moka async cache layer

## Conventions

- Git author: `bkataru <baalateja.k@gmail.com>`
- No hardcoded paths — all config via env vars or CLI args
- Async runtime: tokio
- Release profile: LTO, codegen-units=1, strip=true
