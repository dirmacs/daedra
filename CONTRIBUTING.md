# Contributing to Daedra

First off, thank you for considering contributing to Daedra! It's people like you that make Daedra such a great tool.

## Code of Conduct

This project and everyone participating in it is governed by our commitment to maintaining a welcoming, inclusive environment. Please be respectful and constructive in all interactions.

## How Can I Contribute?

### Reporting Bugs

Before creating bug reports, please check the existing issues to avoid duplicates. When you create a bug report, include as many details as possible:

- **Use a clear and descriptive title**
- **Describe the exact steps to reproduce the problem**
- **Provide specific examples** (code snippets, commands, etc.)
- **Describe the behavior you observed and what you expected**
- **Include your environment details** (OS, Rust version, etc.)

### Suggesting Enhancements

Enhancement suggestions are tracked as GitHub issues. When creating an enhancement suggestion:

- **Use a clear and descriptive title**
- **Provide a detailed description of the suggested enhancement**
- **Explain why this enhancement would be useful**
- **List any alternatives you've considered**

### Pull Requests

1. **Fork the repository** and create your branch from `main`
2. **Write your code** following our coding standards
3. **Add tests** for any new functionality
4. **Ensure all tests pass** (`cargo test`)
5. **Run linting** (`cargo clippy -- -D warnings`)
6. **Format your code** (`cargo fmt`)
7. **Update documentation** if needed
8. **Submit your pull request**

## Development Setup

### Prerequisites

- Rust 1.75 or later
- Cargo (comes with Rust)
- Git

### Getting Started

```bash
# Clone your fork
git clone https://github.com/YOUR_USERNAME/daedra.git
cd daedra

# Add upstream remote
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

### Running Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific tests
cargo test search_tests

# Run integration tests (requires network)
cargo test -- integration

# Run benchmarks
cargo bench
```

### Code Style

We use `rustfmt` and `clippy` to maintain consistent code style:

```bash
# Format code
cargo fmt

# Check formatting
cargo fmt -- --check

# Run clippy
cargo clippy -- -D warnings

# Run clippy with all features
cargo clippy --all-features -- -D warnings
```

### Documentation

- All public APIs must have documentation comments
- Use `///` for item documentation
- Use `//!` for module-level documentation
- Include examples in documentation where helpful

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

### Commit Messages

- Use the present tense ("Add feature" not "Added feature")
- Use the imperative mood ("Move cursor to..." not "Moves cursor to...")
- Limit the first line to 72 characters or less
- Reference issues and pull requests when relevant

Example:
```
Add caching support for search results

- Implement moka-based cache with TTL
- Add cache configuration options
- Add tests for cache operations

Fixes #123
```

### Branch Naming

- `feature/description` - New features
- `fix/description` - Bug fixes
- `docs/description` - Documentation changes
- `refactor/description` - Code refactoring
- `test/description` - Test additions/changes

## Project Structure

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
│       ├── search.rs   # Search implementation
│       └── fetch.rs    # Page fetching implementation
├── tests/
│   └── integration_tests.rs
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

## Review Process

1. A maintainer will review your PR
2. They may request changes or ask questions
3. Once approved, your PR will be merged
4. You'll be added to the contributors list!

## Release Process

Releases are managed by maintainers. The process is:

1. Update version in `Cargo.toml`
2. Update `CHANGELOG.md`
3. Create a release tag
4. CI automatically publishes to crates.io

## Questions?

Feel free to open an issue with your question or reach out to the maintainers.

Thank you for contributing! 🎉
