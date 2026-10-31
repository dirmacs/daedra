//! Transport selection and server configuration.

use crate::cache::CacheConfig;

/// MCP protocol version this server speaks.
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
