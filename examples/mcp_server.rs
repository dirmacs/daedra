//! MCP Server example for Daedra
//!
//! This example demonstrates how to start a Daedra MCP server programmatically.
//!
//! Run with: cargo run --example mcp_server
//!
//! For STDIO transport (default), set USE_SSE=0 or leave unset.
//! For SSE transport, set USE_SSE=1.
//!
//! Note: When using STDIO transport, logs are automatically routed to stderr
//! to prevent corruption of the JSON-RPC stream on stdout.

use daedra::cache::CacheConfig;
use daedra::server::{DaedraServer, ServerConfig, TransportType};
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Choose transport based on environment
    let use_sse = std::env::var("USE_SSE").is_ok();

    // Initialize logging
    // For STDIO transport, we write to stderr to avoid corrupting the JSON-RPC stream
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false);

    if use_sse {
        // SSE transport: logs can go to stdout
        subscriber.init();
        println!("Starting SSE server on http://127.0.0.1:3000");
    } else {
        // STDIO transport: logs MUST go to stderr
        subscriber.with_writer(std::io::stderr).init();
        eprintln!("Starting STDIO server (for MCP clients)");
        eprintln!("Note: Logs are written to stderr to keep stdout clean for JSON-RPC");
    }

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

    // Choose transport
    let transport = if use_sse {
        TransportType::Sse {
            port: 3000,
            host: [127, 0, 0, 1],
        }
    } else {
        TransportType::Stdio
    };

    // Run the server
    server.run(transport).await?;

    Ok(())
}
