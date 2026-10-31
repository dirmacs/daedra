//! Request handler: dispatches a JSON-RPC tool call to the right backend.

use tracing::error;
use crate::cache::SearchCache;
use crate::tools::{self, crawl_site, fetch};
use crate::types::{
    CrawlArgs, DaedraResult, PageContent, SearchArgs, SearchResponse, SearchResult,
    VisitPageArgs, crawl_args_schema, search_args_schema, visit_page_args_schema,
};
use crate::{SERVER_NAME, VERSION};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};
use tracing::{debug, info, instrument};

use super::jsonrpc::{
    JsonRpcRequest, JsonRpcResponse, format_page_result, tool_error_response,
    tool_success_response,
};
use super::stdio::parse_tool_call_params;
use super::tools::McpTool;
use super::transport::{MCP_PROTOCOL_VERSION, ServerConfig};

#[derive(Clone)]
pub struct DaedraHandler {
    /// Search cache
    pub(super) cache: SearchCache,

    /// Multi-backend search provider with automatic fallback
    search_provider: Arc<tools::SearchProvider>,

    /// Fetch client
    fetch_client: Arc<fetch::FetchClient>,

    /// Initialization state
    initialized: Arc<RwLock<bool>>,
}

impl DaedraHandler {
    /// Create a new handler
    pub fn new(config: ServerConfig) -> DaedraResult<Self> {
        Ok(Self {
            cache: SearchCache::new(config.cache),
            search_provider: Arc::new(tools::SearchProvider::auto()),
            fetch_client: Arc::new(fetch::FetchClient::new()?),
            initialized: Arc::new(RwLock::new(false)),
        })
    }

    /// Get server information for initialization
    pub fn get_server_info(&self) -> Value {
        json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": VERSION
            }
        })
    }

    /// List available tools
    pub fn list_tools(&self) -> Vec<McpTool> {
        vec![
            McpTool {
                name: "web_search".to_string(),
                description: Some(
                    "Search the web across 14 unkeyed backends (Mwmbl, Brave, Marginalia, Bing RSS, Bing, Google News, Hacker News, Google, Wikipedia, StackOverflow, GitHub, Wiby, DDG Instant, DDG). Results merge by relevance with cross-engine corroboration."
                        .to_string(),
                ),
                input_schema: search_args_schema(),
            },
            McpTool {
                name: "search_duckduckgo".to_string(),
                description: Some(
                    "Alias for web_search (backward compatibility). Search the web across 14 unkeyed backends (Mwmbl, Brave, Marginalia, Bing RSS, Bing, Google News, Hacker News, Google, Wikipedia, StackOverflow, GitHub, Wiby, DDG Instant, DDG). Results merge by relevance with cross-engine corroboration."
                        .to_string(),
                ),
                input_schema: search_args_schema(),
            },
            McpTool {
                name: "visit_page".to_string(),
                description: Some(
                    "Visit a webpage and extract its content as Markdown. Useful for reading articles, documentation, or any web page."
                        .to_string(),
                ),
                input_schema: visit_page_args_schema(),
            },
            McpTool {
                name: "crawl_site".to_string(),
                description: Some(
                    "Crawl a website starting from a root URL. Discovers pages via sitemap.xml or link following, fetches up to max_pages concurrently, and returns Markdown content for each page."
                        .to_string(),
                ),
                input_schema: crawl_args_schema(),
            },
        ]
    }

    /// Execute search tool
    #[instrument(skip(self))]
    pub async fn execute_search(&self, args: SearchArgs) -> DaedraResult<SearchResponse> {
        let options = args.options.clone().unwrap_or_default();

        // Check cache first
        if let Some(cached) = self
            .cache
            .get_search(
                &args.query,
                &options.region,
                &options.safe_search.to_string(),
            )
            .await
        {
            info!(query = %args.query, "Returning cached search results");
            return Ok(cached);
        }

        // Perform search via multi-backend provider (aggregate across backends)
        let mut response = self.search_provider.search(&args).await?;

        self.enrich_sparse_results(&mut response.data, 3).await;

        // Cache the results
        self.cache
            .set_search(
                &args.query,
                &options.region,
                &options.safe_search.to_string(),
                response.clone(),
            )
            .await;

        Ok(response)
    }

    /// Fetch page snippets for sparse top results (description < 100 chars).
    async fn enrich_sparse_results(&self, results: &mut [SearchResult], count: usize) {
        let enrich_count = count.min(results.len());
        if enrich_count == 0 {
            return;
        }

        let fetch_client = self.fetch_client.clone();
        let enrich_semaphore = Arc::new(Semaphore::new(2));
        let futures: Vec<_> = results[..enrich_count]
            .iter()
            .filter(|r| r.description.len() < 100)
            .map(|r| {
                let url = r.url.clone();
                let client = fetch_client.clone();
                let semaphore = enrich_semaphore.clone();
                async move {
                    let _permit = semaphore.acquire_owned().await.unwrap();
                    let args = VisitPageArgs {
                        url: url.clone(),
                        selector: None,
                        include_images: false,
                    };
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        client.fetch(&args),
                    )
                    .await
                    {
                        Ok(Ok(page)) => {
                            let snippet: String = page.content.chars().take(300).collect();
                            Some((url, snippet))
                        },
                        _ => None,
                    }
                }
            })
            .collect();

        let enrichments = futures::future::join_all(futures).await;
        for enrichment in enrichments.into_iter().flatten() {
            if let Some(result) = results.iter_mut().find(|r| r.url == enrichment.0)
                && result.description.len() < 100
            {
                result.description = enrichment.1;
            }
        }
    }

    /// Execute fetch/visit page tool
    #[instrument(skip(self))]
    pub async fn execute_fetch(&self, args: VisitPageArgs) -> DaedraResult<PageContent> {
        // Check cache first
        if let Some(cached) = self
            .cache
            .get_page(&args.url, args.selector.as_deref())
            .await
        {
            info!(url = %args.url, "Returning cached page content");
            return Ok(cached);
        }

        // Fetch page
        let content = self.fetch_client.fetch(&args).await?;

        // Cache the results
        self.cache
            .set_page(&args.url, args.selector.as_deref(), content.clone())
            .await;

        Ok(content)
    }

    /// Handle a JSON-RPC request
    pub async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        debug!(method = %request.method, "Handling request");

        if request.method == "initialize" {
            let mut initialized = self.initialized.write().await;
            *initialized = true;
        }

        self.handle_method(&request.method, request.id, request.params)
            .await
    }

    /// Dispatch a JSON-RPC method to its handler.
    pub(super) async fn handle_method(
        &self,
        method: &str,
        id: Option<Value>,
        params: Option<Value>,
    ) -> JsonRpcResponse {
        match method {
            "initialize" => JsonRpcResponse::success(id, self.get_server_info()),
            "initialized" | "notifications/initialized" => JsonRpcResponse::success(id, json!({})),
            "tools/list" => JsonRpcResponse::success(id, json!({ "tools": self.list_tools() })),
            "tools/call" => match parse_tool_call_params(params, id.clone()) {
                Ok((name, args)) => self.call_tool(id, &name, args).await,
                Err(resp) => resp,
            },
            "ping" => JsonRpcResponse::success(id, json!({})),
            _ => JsonRpcResponse::error(id, -32601, format!("Method not found: {}", method)),
        }
    }

    async fn handle_web_search(&self, id: Option<Value>, arguments: Value) -> JsonRpcResponse {
        let args: SearchArgs = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => {
                return JsonRpcResponse::error(
                    id,
                    -32602,
                    format!("Invalid search arguments: {}", e),
                );
            },
        };

        match self.execute_search(args).await {
            Ok(response) => {
                let text = serde_json::to_string_pretty(&response).unwrap_or_default();
                tool_success_response(id, text)
            },
            Err(e) => {
                error!(error = %e, "Search failed");
                tool_error_response(id, &format!("Search failed: {}", e))
            },
        }
    }

    pub(super) async fn handle_visit_page(&self, id: Option<Value>, arguments: Value) -> JsonRpcResponse {
        let args: VisitPageArgs = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => {
                return JsonRpcResponse::error(
                    id,
                    -32602,
                    format!("Invalid fetch arguments: {}", e),
                );
            },
        };

        if !fetch::is_valid_url(&args.url) {
            return tool_error_response(id, "Invalid URL: must be HTTP or HTTPS");
        }

        match self.execute_fetch(args).await {
            Ok(content) => tool_success_response(id, format_page_result(&content)),
            Err(e) => {
                error!(error = %e, "Fetch failed");
                tool_error_response(id, &format!("Failed to fetch page: {}", e))
            },
        }
    }

    async fn handle_crawl_site(&self, id: Option<Value>, arguments: Value) -> JsonRpcResponse {
        let args: CrawlArgs = match serde_json::from_value(arguments) {
            Ok(a) => a,
            Err(e) => {
                return JsonRpcResponse::error(
                    id,
                    -32602,
                    format!("Invalid crawl arguments: {}", e),
                );
            },
        };

        match crawl_site(args).await {
            Ok(result) => {
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                tool_success_response(id, text)
            },
            Err(e) => {
                error!(error = %e, "Crawl failed");
                tool_error_response(id, &format!("Crawl failed: {}", e))
            },
        }
    }

    /// Call a specific tool
    async fn call_tool(&self, id: Option<Value>, name: &str, arguments: Value) -> JsonRpcResponse {
        info!(tool = %name, "Executing tool");

        match name {
            "web_search" | "search_duckduckgo" => self.handle_web_search(id, arguments).await,
            "visit_page" => self.handle_visit_page(id, arguments).await,
            "crawl_site" => self.handle_crawl_site(id, arguments).await,
            _ => JsonRpcResponse::error(id, -32601, format!("Unknown tool: {}", name)),
        }
    }

    /// Get cache reference
    pub fn cache(&self) -> &SearchCache {
        &self.cache
    }
}

// `JsonRpcResponse` is the function's only error channel; boxing it here would
// just move the size to every call site.
