# daedra — Agent Guidelines

## What This Is

Daedra is a web search MCP server. It provides search tools to AI agents via MCP (Model Context Protocol). Multiple backends with automatic fallback ensure search always works.

## For Agents

- Run `cargo test` before any changes
- The search fallback chain is the core value — don't break backend ordering
- Each backend is independent — adding a new one shouldn't affect others
- MCP tools are defined in `src/tools/` — follow the existing pattern
- Caching is transparent — backends don't know about the cache layer
- No hardcoded paths or API keys in source code
