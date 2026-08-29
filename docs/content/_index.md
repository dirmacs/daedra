+++
title = "Daedra"
description = "Self-contained web search MCP server. 16 backends, Readability extraction, PDF support, circuit breakers."
template = "index.html"
+++

Daedra is an MCP server in Rust for web search and page extraction. Sixteen backends run each query, and results merge by relevance, so search works from any IP address. Basic search needs no API keys; with no keys the backends are the knowledge and feed sources (wiki, HN, StackOverflow, GitHub, RSS), not general web search.

See the project [README](https://github.com/dirmacs/daedra) for full documentation.
