//! JSON Schemas advertised to MCP clients.

/// JSON Schema for search arguments (used for MCP tool definition)
pub fn search_args_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "The search query string"
            },
            "options": {
                "type": "object",
                "description": "Optional search configuration",
                "properties": {
                    "region": {
                        "type": "string",
                        "description": "Region for search results (e.g., 'us-en', 'wt-wt' for worldwide)",
                        "default": "wt-wt"
                    },
                    "safe_search": {
                        "type": "string",
                        "enum": ["OFF", "MODERATE", "STRICT"],
                        "description": "Safe search filtering level",
                        "default": "MODERATE"
                    },
                    "num_results": {
                        "type": "integer",
                        "description": "Maximum number of results to return",
                        "default": 10,
                        "minimum": 1,
                        "maximum": 50
                    },
                    "time_range": {
                        "type": "string",
                        "description": "Time range filter (d=day, w=week, m=month, y=year)"
                    }
                }
            }
        },
        "required": ["query"]
    })
}

/// JSON Schema for visit_page arguments
pub fn visit_page_args_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "url": {
                "type": "string",
                "format": "uri",
                "description": "URL of the page to visit"
            },
            "selector": {
                "type": "string",
                "description": "Optional CSS selector to target specific content"
            },
            "include_images": {
                "type": "boolean",
                "description": "Whether to include image references in the response",
                "default": false
            }
        },
        "required": ["url"]
    })
}

/// Returns the JSON Schema for the crawl_site tool arguments.
pub fn crawl_args_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "root_url": {
                "type": "string",
                "format": "uri",
                "description": "Root URL of the site to crawl (sitemap or homepage)"
            },
            "max_pages": {
                "type": "integer",
                "description": "Maximum number of pages to fetch (default: 25)",
                "default": 25
            },
            "concurrency": {
                "type": "integer",
                "description": "Maximum concurrent fetches (default: 4)",
                "default": 4
            },
            "depth": {
                "type": "integer",
                "description": "Link layers to follow past discovery (1 = discovered pages only, max 5)",
                "default": 1
            },
            "delay_ms": {
                "type": "integer",
                "description": "Delay between page fetch starts in milliseconds (politeness)",
                "default": 0
            },
            "ignore_robots": {
                "type": "boolean",
                "description": "Ignore robots.txt exclusions for this crawl",
                "default": false
            }
        },
        "required": ["root_url"]
    })
}
