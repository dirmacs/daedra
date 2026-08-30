+++
title = "Daedra"
description = "Self-contained web search MCP server. 14 unkeyed backends, Readability extraction, PDF and Office document support, circuit breakers."
template = "index.html"
+++

Daedra is an MCP server in Rust for web search and page extraction. Fourteen unkeyed backends run each query, and results merge by relevance, so search works from any IP address. Mwmbl and Marginalia are the general indexes. HTML scrapers may meet a CAPTCHA.

See the project [README](https://github.com/dirmacs/daedra) for full documentation.
