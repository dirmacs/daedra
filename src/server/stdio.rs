//! Line-oriented stdio transport for MCP.

use serde_json::json;
use tracing::debug;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{error, info};

use super::handler::DaedraHandler;
use super::jsonrpc::{
    JsonRpcRequest, JsonRpcResponse, is_notification, tool_error_response, tool_success_response,
};

pub(super) fn parse_tool_call_params(
    params: Option<Value>,
    id: Option<Value>,
) -> Result<(String, Value), JsonRpcResponse> {
    let params = match params {
        Some(p) => p,
        None => {
            return Err(JsonRpcResponse::error(
                id,
                -32602,
                "Missing parameters".to_string(),
            ));
        },
    };
    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    Ok((tool_name, arguments))
}




/// Parse and handle one STDIO line; returns a response only for non-notification requests.
pub(super) async fn process_stdio_line(line: &str, handler: &DaedraHandler) -> Option<JsonRpcResponse> {
    if line.trim().is_empty() {
        return None;
    }

    debug!(request = %line, "Received request");

    let request: JsonRpcRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            return Some(JsonRpcResponse::error(
                None,
                -32700,
                format!("Parse error: {}", e),
            ));
        },
    };

    let response = handler.handle_request(request.clone()).await;
    if is_notification(&request) {
        None
    } else {
        Some(response)
    }
}

/// Serialize a JSON-RPC response and write it to STDIO (with trailing newline).
pub(super) async fn write_stdio_response(
    response: JsonRpcResponse,
    stdout: &mut tokio::io::BufWriter<tokio::io::Stdout>,
) -> std::io::Result<()> {
    let response_str = serde_json::to_string(&response).unwrap();
    debug!(response = %response_str, "Sending response");
    stdout.write_all(response_str.as_bytes()).await?;
    stdout
        .write_all(
            b"
",
        )
        .await?;
    stdout.flush().await
}
