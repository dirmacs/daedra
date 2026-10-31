//! SSE/HTTP transport.

use crate::types::{DaedraError, DaedraResult};
use crate::{SERVER_NAME, VERSION};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{info, instrument};

use super::handler::DaedraHandler;
use super::jsonrpc::{JsonRpcRequest, JsonRpcResponse};
use super::stdio::{process_stdio_line, write_stdio_response};
use super::transport::{ServerConfig, TransportType};

/// SSE/HTTP transport for the MCP server.
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
        let mut stdout = tokio::io::BufWriter::new(tokio::io::stdout());
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(response) = process_stdio_line(&line, &self.handler).await {
                write_stdio_response(response, &mut stdout).await?;
            }
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
