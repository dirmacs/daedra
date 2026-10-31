//! Tool descriptors advertised over MCP.

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
