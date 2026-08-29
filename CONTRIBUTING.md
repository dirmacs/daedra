# Contributing to Daedra

Thank you for your interest in Daedra. This document tells you how to report bugs, suggest changes, and submit code.

## Code of conduct

Be respectful and constructive in all project interactions.

## Report a bug

Before you create a bug report, read the existing issues. Do not create a duplicate. Include this information in your report:

- A clear and descriptive title
- The exact steps that reproduce the problem
- Specific examples: code snippets, commands, or both
- The behavior you observed, and the behavior you expected
- Your environment: operating system, Rust version, and related details

## Suggest an enhancement

Enhancement suggestions live in GitHub issues. Include this information:

- A clear and descriptive title
- A detailed description of the enhancement
- The reason the enhancement is useful
- Alternatives that you considered

## Submit a pull request

1. Fork the repository. Create your branch from `main`.
2. Write your code. Follow the code standards in this document.
3. Add tests for each new function.
4. Run `cargo test`. All tests must pass.
5. Run `cargo clippy -- -D warnings`. It must exit with code 0.
6. Run `cargo fmt`.
7. Update the documentation if your change needs it.
8. Submit the pull request.

## Development setup

Prerequisites:

- Rust 1.98 or later
- Cargo (ships with Rust)
- Git

Get started:

```bash
# Clone your fork
git clone https://github.com/YOUR_USERNAME/daedra.git
cd daedra

# Add the upstream remote
git remote add upstream https://github.com/dirmacs/daedra.git

# Create a branch for your changes
git checkout -b feature/your-feature-name

# Build the project
cargo build

# Run tests
cargo test

# Run the CLI
cargo run -- --help
```

### Run tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run one test group
cargo test search_tests

# Run integration tests (needs network access)
cargo test -- integration

# Run benchmarks
cargo bench
```

### Code style

The project uses `rustfmt` and `clippy`:

```bash
# Format code
cargo fmt

# Check the formatting
cargo fmt -- --check

# Run clippy
cargo clippy -- -D warnings

# Run clippy with all features
cargo clippy --all-features -- -D warnings
```

### Documentation

- Write a documentation comment for every public API item.
- Use `///` for item documentation.
- Use `//!` for module documentation.
- Add examples where they help the reader.

```rust
/// Performs a web search using DuckDuckGo.
///
/// # Arguments
///
/// * `args` - Search arguments including query and options
///
/// # Returns
///
/// A `SearchResponse` containing the results
///
/// # Example
///
/// ```rust,no_run
/// use daedra::tools::search::{perform_search, SearchArgs};
///
/// let args = SearchArgs {
///     query: "rust".to_string(),
///     options: None,
/// };
/// let results = perform_search(&args).await?;
/// ```
pub async fn perform_search(args: &SearchArgs) -> DaedraResult<SearchResponse> {
    // ...
}
```

### Commit messages

- Write commit titles in the imperative mood: "Add feature", not "Added feature".
- Keep the first line at 72 characters or less.
- Reference issues and pull requests when relevant.

Example:

```
Add caching support for search results

- Implement moka-based cache with TTL
- Add cache configuration options
- Add tests for cache operations

Fixes #123
```

### Branch names

- `feature/description` — new features
- `fix/description` — bug fixes
- `docs/description` — documentation changes
- `refactor/description` — code refactoring
- `test/description` — test additions and changes

## Project structure

```
daedra/
├── src/
│   ├── lib.rs          # Library root, public API
│   ├── main.rs         # CLI binary
│   ├── server.rs       # MCP server implementation
│   ├── types.rs        # Type definitions
│   ├── cache.rs        # Caching implementation
│   └── tools/
│       ├── mod.rs      # Tools module
│       ├── backend.rs  # SearchProvider, fallback chain, circuit breakers
│       ├── rss.rs      # Machine-format backends (Bing RSS, Google News, HN)
│       ├── search.rs   # DuckDuckGo HTML search
│       ├── fetch.rs    # Page fetching implementation
│       └── soft_block.rs # Zero-result page classifier
├── tests/
│   ├── integration_tests.rs      # General integration tests
│   └── stdio_transport_tests.rs  # STDIO transport & MCP protocol tests
├── benches/
│   └── search_benchmark.rs
├── examples/
│   ├── basic_usage.rs
│   ├── mcp_server.rs
│   └── caching.rs
└── .github/
    └── workflows/
        ├── ci.yml
        └── publish.yml
```

## Review process

1. A maintainer reviews your pull request.
2. The maintainer requests changes or asks questions if the code needs them.
3. The maintainer merges the pull request after approval.
4. Your name joins the contributors list.

## Release process

Maintainers manage releases. The process is:

1. Update the version in `Cargo.toml`.
2. Update `CHANGELOG.md`.
3. Create a release tag.
4. CI publishes to crates.io automatically.

## Questions

Open an issue with your question. A maintainer will answer.
