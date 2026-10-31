//! MCP server: JSON-RPC framing, tool definitions, request handling, and the
//! stdio and SSE transports.
//!
//! - [`jsonrpc`] — wire types for the JSON-RPC envelope
//! - [`tools`]   — the tool descriptors advertised over MCP
//! - [`handler`] — [`DaedraHandler`], which dispatches a tool call
//! - [`stdio`]   — line-oriented stdio transport
//! - [`server`]  — [`DaedraServer`], the SSE/HTTP transport
//! - [`transport`] — transport selection config

mod handler;
mod jsonrpc;
mod server;
mod stdio;
mod tools;
mod transport;

pub use handler::DaedraHandler;
pub use jsonrpc::{JsonRpcError, JsonRpcRequest, JsonRpcResponse, is_notification};
pub use server::DaedraServer;
pub use tools::McpTool;
pub use transport::{ServerConfig, TransportType};

pub use transport::MCP_PROTOCOL_VERSION;

#[cfg(test)]
mod tests;
