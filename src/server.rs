//! MCP server implementation for Daedra.
//!
//! This module provides the core MCP server implementation that handles
//! tool requests and manages communication via STDIO or SSE transports.

use crate::cache::{CacheConfig, SearchCache};
use crate::tools::{fetch, search};
use crate::types::{
    DaedraError, DaedraResult, PageContent, SearchArgs, SearchResponse, VisitPageArgs,
    search_args_schema, visit_page_args_schema,
};
use crate::{SERVER_NAME, VERSION};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument};

/// MCP Protocol version
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Transport type for the MCP server
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportType {
    /// Standard input/output transport
    #[default]
    Stdio,
    /// Server-Sent Events over HTTP
    Sse {
        /// Port to listen on
        port: u16,
        /// Host to bind to
        host: [u8; 4],
    },
}

/// Configuration for the Daedra server
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Cache configuration
    pub cache: CacheConfig,

    /// Whether to enable verbose logging
    pub verbose: bool,

    /// Maximum concurrent tool executions
    pub max_concurrent_tools: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            cache: CacheConfig::default(),
            verbose: false,
            max_concurrent_tools: 10,
        }
    }
}

/// JSON-RPC 2.0 Request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version (always "2.0")
    pub jsonrpc: String,
    /// Request ID (None for notifications)
    pub id: Option<Value>,
    /// Method name
    pub method: String,
    /// Method parameters
    #[serde(default)]
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 Response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version (always "2.0")
    pub jsonrpc: String,
    /// Request ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    /// Success result
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error result
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 Error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Error code
    pub code: i32,
    /// Error message
    pub message: String,
    /// Additional error data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    /// Create a success response
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Create an error response
    pub fn error(id: Option<Value>, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: None,
            }),
        }
    }
}

/// MCP Tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: Option<String>,
    /// JSON Schema for input
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// Tool handler implementation
#[derive(Clone)]
pub struct DaedraHandler {
    /// Search cache
    cache: SearchCache,

    /// Search client
    search_client: Arc<search::SearchClient>,

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
            search_client: Arc::new(search::SearchClient::new()?),
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
                name: "search_duckduckgo".to_string(),
                description: Some(
                    "Search the web using DuckDuckGo. Returns structured search results with metadata."
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

        // Perform search
        let response = self.search_client.search(&args).await?;

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

        match request.method.as_str() {
            "initialize" => {
                let mut initialized = self.initialized.write().await;
                *initialized = true;
                JsonRpcResponse::success(request.id, self.get_server_info())
            },

            "initialized" => {
                // Notification acknowledgment
                JsonRpcResponse::success(request.id, json!({}))
            },

            "tools/list" => {
                let tools = self.list_tools();
                JsonRpcResponse::success(request.id, json!({ "tools": tools }))
            },

            "tools/call" => {
                let params = match request.params {
                    Some(p) => p,
                    None => {
                        return JsonRpcResponse::error(
                            request.id,
                            -32602,
                            "Missing parameters".to_string(),
                        );
                    },
                };

                let tool_name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

                self.call_tool(request.id, tool_name, arguments).await
            },

            "ping" => JsonRpcResponse::success(request.id, json!({})),

            _ => JsonRpcResponse::error(
                request.id,
                -32601,
                format!("Method not found: {}", request.method),
            ),
        }
    }

    /// Call a specific tool
    async fn call_tool(&self, id: Option<Value>, name: &str, arguments: Value) -> JsonRpcResponse {
        info!(tool = %name, "Executing tool");

        match name {
            "search_duckduckgo" => {
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
                        JsonRpcResponse::success(
                            id,
                            json!({
                                "content": [{ "type": "text", "text": text }],
                                "isError": false
                            }),
                        )
                    },
                    Err(e) => {
                        error!(error = %e, "Search failed");
                        JsonRpcResponse::success(
                            id,
                            json!({
                                "content": [{ "type": "text", "text": format!("Search failed: {}", e) }],
                                "isError": true
                            }),
                        )
                    },
                }
            },

            "visit_page" => {
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

                // Validate URL
                if !fetch::is_valid_url(&args.url) {
                    return JsonRpcResponse::success(
                        id,
                        json!({
                            "content": [{ "type": "text", "text": "Invalid URL: must be HTTP or HTTPS" }],
                            "isError": true
                        }),
                    );
                }

                match self.execute_fetch(args).await {
                    Ok(content) => {
                        let output = format!(
                            "# {}\n\n**URL:** {}\n**Fetched:** {}\n**Words:** {}\n\n---\n\n{}",
                            content.title,
                            content.url,
                            content.timestamp,
                            content.word_count,
                            content.content
                        );
                        JsonRpcResponse::success(
                            id,
                            json!({
                                "content": [{ "type": "text", "text": output }],
                                "isError": false
                            }),
                        )
                    },
                    Err(e) => {
                        error!(error = %e, "Fetch failed");
                        JsonRpcResponse::success(
                            id,
                            json!({
                                "content": [{ "type": "text", "text": format!("Failed to fetch page: {}", e) }],
                                "isError": true
                            }),
                        )
                    },
                }
            },

            _ => JsonRpcResponse::error(id, -32601, format!("Unknown tool: {}", name)),
        }
    }

    /// Get cache reference
    pub fn cache(&self) -> &SearchCache {
        &self.cache
    }
}

/// Main Daedra MCP server
pub struct DaedraServer {
    handler: DaedraHandler,
    #[allow(dead_code)]
    config: ServerConfig,
}

impl DaedraServer {
    /// Create a new Daedra server with the given configuration
    pub fn new(config: ServerConfig) -> DaedraResult<Self> {
        let handler = DaedraHandler::new(config.clone())?;
        Ok(Self { handler, config })
    }

    /// Create a new server with default configuration
    pub fn with_defaults() -> DaedraResult<Self> {
        Self::new(ServerConfig::default())
    }

    /// Run the server with the specified transport
    #[instrument(skip(self))]
    pub async fn run(self, transport: TransportType) -> DaedraResult<()> {
        info!(
            server = SERVER_NAME,
            version = VERSION,
            "Starting Daedra MCP server"
        );

        match transport {
            TransportType::Stdio => self.run_stdio().await,
            TransportType::Sse { port, host } => self.run_sse(host, port).await,
        }
    }

    /// Run the server with STDIO transport
    async fn run_stdio(self) -> DaedraResult<()> {
        info!("Starting STDIO transport");

        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        // Process JSON-RPC messages line by line
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }

            debug!(request = %line, "Received request");

            // Parse the request
            let request: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    let error_response =
                        JsonRpcResponse::error(None, -32700, format!("Parse error: {}", e));
                    let response_str = serde_json::to_string(&error_response).unwrap();
                    stdout.write_all(response_str.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                    stdout.flush().await?;
                    continue;
                },
            };

            // Handle the request
            let response = self.handler.handle_request(request).await;

            // Send the response
            let response_str = serde_json::to_string(&response).unwrap();
            debug!(response = %response_str, "Sending response");
            stdout.write_all(response_str.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }

        info!("STDIO server stopped");
        Ok(())
    }

    /// Run the server with SSE transport
    async fn run_sse(self, host: [u8; 4], port: u16) -> DaedraResult<()> {
        use axum::{
            Json, Router,
            extract::State,
            response::sse::{Event, Sse},
            routing::{get, post},
        };
        use futures::stream::{self, Stream};
        use std::convert::Infallible;
        use tower_http::cors::CorsLayer;

        info!(host = ?host, port = port, "Starting SSE transport");

        let handler = Arc::new(self.handler);

        // Health check endpoint
        async fn health() -> &'static str {
            "OK"
        }

        // SSE endpoint for server-to-client messages
        async fn sse_handler() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
            let stream = stream::once(async { Ok(Event::default().data("connected")) });
            Sse::new(stream)
        }

        // JSON-RPC endpoint
        async fn rpc_handler(
            State(handler): State<Arc<DaedraHandler>>,
            Json(request): Json<JsonRpcRequest>,
        ) -> Json<JsonRpcResponse> {
            let response = handler.handle_request(request).await;
            Json(response)
        }

        // Build the router
        let app = Router::new()
            .route("/health", get(health))
            .route("/sse", get(sse_handler))
            .route("/rpc", post(rpc_handler))
            .layer(CorsLayer::permissive())
            .with_state(handler);

        let addr = std::net::SocketAddr::from((host, port));
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
            DaedraError::ServerError(format!(
                "Failed to bind to {}:{}: {}",
                host.iter()
                    .map(|b| b.to_string())
                    .collect::<Vec<_>>()
                    .join("."),
                port,
                e
            ))
        })?;

        info!(
            "SSE server listening on http://{}:{}",
            host.iter()
                .map(|b| b.to_string())
                .collect::<Vec<_>>()
                .join("."),
            port
        );

        axum::serve(listener, app)
            .await
            .map_err(|e| DaedraError::ServerError(format!("Server error: {}", e)))?;

        Ok(())
    }

    /// Get the server's cache statistics
    pub fn cache_stats(&self) -> crate::cache::CacheStats {
        self.handler.cache.stats()
    }

    /// Clear the server's cache
    pub async fn clear_cache(&self) {
        self.handler.cache.clear().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert!(!config.verbose);
        assert_eq!(config.max_concurrent_tools, 10);
    }

    #[test]
    fn test_transport_type_default() {
        assert_eq!(TransportType::default(), TransportType::Stdio);
    }

    #[tokio::test]
    async fn test_handler_creation() {
        let config = ServerConfig::default();
        let handler = DaedraHandler::new(config);
        assert!(handler.is_ok());
    }

    #[test]
    fn test_list_tools() {
        let config = ServerConfig::default();
        let handler = DaedraHandler::new(config).unwrap();
        let tools = handler.list_tools();

        assert_eq!(tools.len(), 2);
        assert!(tools.iter().any(|t| t.name == "search_duckduckgo"));
        assert!(tools.iter().any(|t| t.name == "visit_page"));
    }

    #[test]
    fn test_json_rpc_response_success() {
        let response = JsonRpcResponse::success(Some(json!(1)), json!({"status": "ok"}));
        assert_eq!(response.jsonrpc, "2.0");
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[test]
    fn test_json_rpc_response_error() {
        let response =
            JsonRpcResponse::error(Some(json!(1)), -32600, "Invalid request".to_string());
        assert_eq!(response.jsonrpc, "2.0");
        assert!(response.result.is_none());
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32600);
    }

    #[tokio::test]
    async fn test_handle_ping() {
        let config = ServerConfig::default();
        let handler = DaedraHandler::new(config).unwrap();

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "ping".to_string(),
            params: None,
        };

        let response = handler.handle_request(request).await;
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[tokio::test]
    async fn test_handle_initialize() {
        let config = ServerConfig::default();
        let handler = DaedraHandler::new(config).unwrap();

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "initialize".to_string(),
            params: None,
        };

        let response = handler.handle_request(request).await;
        assert!(response.result.is_some());

        let result = response.result.unwrap();
        assert_eq!(result["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], SERVER_NAME);
    }

    #[tokio::test]
    async fn test_handle_tools_list() {
        let config = ServerConfig::default();
        let handler = DaedraHandler::new(config).unwrap();

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "tools/list".to_string(),
            params: None,
        };

        let response = handler.handle_request(request).await;
        assert!(response.result.is_some());

        let result = response.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
    }

    #[tokio::test]
    async fn test_handle_unknown_method() {
        let config = ServerConfig::default();
        let handler = DaedraHandler::new(config).unwrap();

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "unknown/method".to_string(),
            params: None,
        };

        let response = handler.handle_request(request).await;
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }
}
