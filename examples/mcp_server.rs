//! MCP Server example for Daedra
//!
//! Run with: cargo run --example mcp_server

use daedra::cache::CacheConfig;
use daedra::server::{DaedraServer, ServerConfig, TransportType};
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    println!("🔍 Starting Daedra MCP Server\n");

    // Configure the server
    let config = ServerConfig {
        cache: CacheConfig {
            ttl: Duration::from_secs(600), // 10 minute cache
            max_entries: 500,
            enabled: true,
        },
        verbose: true,
        max_concurrent_tools: 5,
    };

    // Create the server
    let server = DaedraServer::new(config)?;

    // Choose transport based on environment or arguments
    let transport = if std::env::var("USE_SSE").is_ok() {
        println!("Starting SSE server on http://127.0.0.1:3000");
        TransportType::Sse {
            port: 3000,
            host: [127, 0, 0, 1],
        }
    } else {
        println!("Starting STDIO server (for MCP clients)");
        TransportType::Stdio
    };

    // Run the server
    server.run(transport).await?;

    Ok(())
}
