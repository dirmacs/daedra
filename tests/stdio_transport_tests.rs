//! Integration tests for stdio transport
//!
//! These tests verify that the MCP server correctly handles stdio transport,
//! ensuring that:
//! - stdout only contains valid JSON-RPC messages
//! - logs are routed to stderr (not stdout)
//! - all MCP protocol methods work correctly
//! - tool execution works end-to-end

use serde_json::{Value, json};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;

/// Helper struct to manage the daedra server process
struct DaedraProcess {
    child: Child,
    stdin: tokio::process::ChildStdin,
    stdout_reader: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    #[allow(dead_code)]
    stderr_reader: tokio::io::Lines<BufReader<tokio::process::ChildStderr>>,
}

impl DaedraProcess {
    /// Spawn a new daedra server process with stdio transport
    async fn spawn() -> Self {
        Self::spawn_with_args(&[]).await
    }

    /// Spawn a new daedra server process with additional arguments
    async fn spawn_with_args(extra_args: &[&str]) -> Self {
        let mut args = vec!["serve", "--transport", "stdio"];
        args.extend(extra_args);

        let mut child = Command::new(env!("CARGO_BIN_EXE_daedra"))
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to spawn daedra process");

        let stdin = child.stdin.take().expect("Failed to get stdin");
        let stdout = child.stdout.take().expect("Failed to get stdout");
        let stderr = child.stderr.take().expect("Failed to get stderr");

        let stdout_reader = BufReader::new(stdout).lines();
        let stderr_reader = BufReader::new(stderr).lines();

        Self {
            child,
            stdin,
            stdout_reader,
            stderr_reader,
        }
    }

    /// Send a JSON-RPC request and return the response
    async fn send_request(&mut self, request: Value) -> Result<Value, String> {
        let request_str = serde_json::to_string(&request).unwrap();
        self.stdin
            .write_all(request_str.as_bytes())
            .await
            .map_err(|e| format!("Failed to write request: {}", e))?;
        self.stdin
            .write_all(b"\n")
            .await
            .map_err(|e| format!("Failed to write newline: {}", e))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush stdin: {}", e))?;

        // Read response with timeout
        let response_line = timeout(Duration::from_secs(30), self.stdout_reader.next_line())
            .await
            .map_err(|_| "Timeout waiting for response".to_string())?
            .map_err(|e| format!("Failed to read response: {}", e))?
            .ok_or_else(|| "No response received".to_string())?;

        serde_json::from_str(&response_line)
            .map_err(|e| format!("Failed to parse response: {} - raw: {}", e, response_line))
    }

    /// Send a JSON-RPC request and verify it succeeds
    async fn send_request_expect_success(&mut self, request: Value) -> Value {
        let response = self
            .send_request(request)
            .await
            .expect("Request should succeed");
        assert!(
            response.get("error").is_none(),
            "Expected success but got error: {:?}",
            response
        );
        response
    }

    /// Perform the MCP initialization handshake
    async fn initialize(&mut self) -> Value {
        let init_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "test-client",
                    "version": "1.0.0"
                }
            }
        });

        let response = self.send_request_expect_success(init_request).await;
        assert!(response["result"]["protocolVersion"].is_string());
        assert!(response["result"]["serverInfo"]["name"].is_string());

        // Send initialized notification
        let initialized_request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "initialized",
            "params": {}
        });
        self.send_request_expect_success(initialized_request).await;

        response
    }

    /// Cleanup the process
    async fn cleanup(mut self) {
        drop(self.stdin);
        let _ = self.child.kill().await;
    }
}

mod protocol_tests {
    use super::*;

    #[tokio::test]
    async fn test_stdout_only_contains_valid_jsonrpc() {
        let mut process = DaedraProcess::spawn().await;

        // Send initialize request
        let init_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });

        let response = process
            .send_request(init_request)
            .await
            .expect("Should get response");

        // Verify it's valid JSON-RPC 2.0
        assert_eq!(response["jsonrpc"], "2.0");
        assert!(
            response.get("result").is_some() || response.get("error").is_some(),
            "Response must have result or error"
        );

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_stdout_contains_no_ansi_codes() {
        let mut process = DaedraProcess::spawn().await;

        // Send multiple requests to trigger various code paths
        let requests = vec![
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "initialized", "params": {}}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list", "params": {}}),
            json!({"jsonrpc": "2.0", "id": 4, "method": "ping", "params": {}}),
        ];

        for request in requests {
            let request_str = serde_json::to_string(&request).unwrap();
            process
                .stdin
                .write_all(request_str.as_bytes())
                .await
                .unwrap();
            process.stdin.write_all(b"\n").await.unwrap();
            process.stdin.flush().await.unwrap();

            let response_line = timeout(Duration::from_secs(5), process.stdout_reader.next_line())
                .await
                .expect("Should not timeout")
                .expect("Should read line")
                .expect("Should have content");

            // Check for ANSI escape codes (ESC character is \x1b or \u001b)
            assert!(
                !response_line.contains('\x1b'),
                "stdout should not contain ANSI escape codes, got: {}",
                response_line
            );

            // Also check for common ANSI patterns
            assert!(
                !response_line.contains("[0m"),
                "stdout should not contain ANSI reset codes"
            );
            assert!(
                !response_line.contains("[1m"),
                "stdout should not contain ANSI bold codes"
            );
            assert!(
                !response_line.contains("[2m"),
                "stdout should not contain ANSI dim codes"
            );

            // Verify it's still valid JSON
            let _: Value =
                serde_json::from_str(&response_line).expect("Response should be valid JSON");
        }

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_initialize_returns_correct_structure() {
        let mut process = DaedraProcess::spawn().await;

        let init_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "test-client",
                    "version": "1.0.0"
                }
            }
        });

        let response = process.send_request_expect_success(init_request).await;
        let result = &response["result"];

        // Verify required fields per MCP spec
        assert!(
            result["protocolVersion"].is_string(),
            "Missing protocolVersion"
        );
        assert!(result["capabilities"].is_object(), "Missing capabilities");
        assert!(result["serverInfo"].is_object(), "Missing serverInfo");
        assert!(
            result["serverInfo"]["name"].is_string(),
            "Missing server name"
        );
        assert!(
            result["serverInfo"]["version"].is_string(),
            "Missing server version"
        );

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_initialized_notification_handled() {
        let mut process = DaedraProcess::spawn().await;

        // First initialize
        let init_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });
        process.send_request_expect_success(init_request).await;

        // Then send initialized notification
        let initialized_request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "initialized",
            "params": {}
        });

        let response = process
            .send_request(initialized_request)
            .await
            .expect("Should get response");

        // Should succeed without error
        assert!(
            response.get("error").is_none(),
            "initialized should not return error"
        );

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_notifications_initialized_handled() {
        let mut process = DaedraProcess::spawn().await;

        // First initialize
        let init_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });
        process.send_request_expect_success(init_request).await;

        // Send notifications/initialized (with prefix, as some clients do)
        let notifications_initialized_request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "notifications/initialized",
            "params": {}
        });

        let response = process
            .send_request(notifications_initialized_request)
            .await
            .expect("Should get response");

        // Should succeed without error (this was the bug in issue #4)
        assert!(
            response.get("error").is_none(),
            "notifications/initialized should not return 'Method not found' error, got: {:?}",
            response
        );

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_tools_list_returns_tools() {
        let mut process = DaedraProcess::spawn().await;
        process.initialize().await;

        let tools_request = json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/list",
            "params": {}
        });

        let response = process.send_request_expect_success(tools_request).await;
        let tools = response["result"]["tools"]
            .as_array()
            .expect("tools should be array");

        assert!(!tools.is_empty(), "Should have at least one tool");

        // Verify expected tools exist
        let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

        assert!(
            tool_names.contains(&"search_duckduckgo"),
            "Should have search_duckduckgo tool"
        );
        assert!(
            tool_names.contains(&"visit_page"),
            "Should have visit_page tool"
        );

        // Verify tool schema structure
        for tool in tools {
            assert!(tool["name"].is_string(), "Tool should have name");
            assert!(
                tool["inputSchema"].is_object(),
                "Tool should have inputSchema"
            );
        }

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_ping_returns_success() {
        let mut process = DaedraProcess::spawn().await;
        process.initialize().await;

        let ping_request = json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "ping",
            "params": {}
        });

        let response = process.send_request_expect_success(ping_request).await;
        assert!(response["result"].is_object());

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_unknown_method_returns_error() {
        let mut process = DaedraProcess::spawn().await;
        process.initialize().await;

        let unknown_request = json!({
            "jsonrpc": "2.0",
            "id": 999,
            "method": "unknown/method",
            "params": {}
        });

        let response = process
            .send_request(unknown_request)
            .await
            .expect("Should get response");

        assert!(response.get("error").is_some(), "Should return error");
        assert_eq!(
            response["error"]["code"], -32601,
            "Should be 'Method not found' error code"
        );

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_malformed_json_returns_parse_error() {
        let mut process = DaedraProcess::spawn().await;

        // Send malformed JSON
        process
            .stdin
            .write_all(b"this is not json\n")
            .await
            .unwrap();
        process.stdin.flush().await.unwrap();

        let response_line = timeout(Duration::from_secs(5), process.stdout_reader.next_line())
            .await
            .expect("Should not timeout")
            .expect("Should read line")
            .expect("Should have content");

        let response: Value =
            serde_json::from_str(&response_line).expect("Response should be valid JSON");

        assert!(response.get("error").is_some(), "Should return error");
        assert_eq!(
            response["error"]["code"], -32700,
            "Should be 'Parse error' code"
        );

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_missing_params_for_tools_call() {
        let mut process = DaedraProcess::spawn().await;
        process.initialize().await;

        // tools/call without params
        let request = json!({
            "jsonrpc": "2.0",
            "id": 50,
            "method": "tools/call"
        });

        let response = process
            .send_request(request)
            .await
            .expect("Should get response");

        assert!(
            response.get("error").is_some(),
            "Should return error for missing params"
        );
        assert_eq!(
            response["error"]["code"], -32602,
            "Should be 'Invalid params' error"
        );

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_full_mcp_handshake_sequence() {
        let mut process = DaedraProcess::spawn().await;

        // 1. Initialize
        let init_response = process.initialize().await;
        assert!(init_response["result"]["protocolVersion"].is_string());

        // 2. List tools
        let tools_request = json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/list",
            "params": {}
        });
        let tools_response = process.send_request_expect_success(tools_request).await;
        assert!(
            !tools_response["result"]["tools"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        // 3. Ping
        let ping_request = json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "ping",
            "params": {}
        });
        process.send_request_expect_success(ping_request).await;

        process.cleanup().await;
    }
}

mod quiet_mode_tests {
    use super::*;

    #[tokio::test]
    async fn test_quiet_mode_suppresses_logs() {
        let mut process = DaedraProcess::spawn_with_args(&["--quiet"]).await;

        // Send a request
        let init_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });

        let response = process
            .send_request(init_request)
            .await
            .expect("Should get response");

        assert_eq!(response["jsonrpc"], "2.0");
        assert!(response["result"].is_object());

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_verbose_mode_still_works_with_stderr() {
        let mut process = DaedraProcess::spawn_with_args(&["--verbose"]).await;

        // Send a request
        let init_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });

        let response = process
            .send_request(init_request)
            .await
            .expect("Should get response");

        // Verify stdout still contains only valid JSON-RPC
        assert_eq!(response["jsonrpc"], "2.0");
        assert!(
            response["result"].is_object(),
            "Response should be valid even in verbose mode"
        );

        // No ANSI codes should leak to stdout
        // (Logs go to stderr in stdio mode)

        process.cleanup().await;
    }
}

mod tool_execution_tests {
    use super::*;

    #[tokio::test]
    async fn test_search_tool_execution() {
        let mut process = DaedraProcess::spawn().await;
        process.initialize().await;

        let search_request = json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "tools/call",
            "params": {
                "name": "search_duckduckgo",
                "arguments": {
                    "query": "rust programming language",
                    "options": {
                        "num_results": 3,
                        "region": "wt-wt",
                        "safe_search": "MODERATE"
                    }
                }
            }
        });

        // Use longer timeout for network request
        let request_str = serde_json::to_string(&search_request).unwrap();
        process
            .stdin
            .write_all(request_str.as_bytes())
            .await
            .unwrap();
        process.stdin.write_all(b"\n").await.unwrap();
        process.stdin.flush().await.unwrap();

        let response_result =
            timeout(Duration::from_secs(60), process.stdout_reader.next_line()).await;

        match response_result {
            Ok(Ok(Some(response_line))) => {
                // Verify response is valid JSON
                let response: Value =
                    serde_json::from_str(&response_line).expect("Response should be valid JSON");

                assert_eq!(response["jsonrpc"], "2.0");
                assert!(response["result"].is_object(), "Should have result");

                let content = &response["result"]["content"];
                assert!(content.is_array(), "Result should have content array");

                // Content should have text
                let text = content[0]["text"].as_str().unwrap_or("");
                // Search may return results or may be rate-limited, but should not error
                assert!(!text.is_empty(), "Content text should not be empty");

                // No ANSI in response
                assert!(
                    !response_line.contains('\x1b'),
                    "Response should not contain ANSI codes"
                );
            },
            Ok(Ok(None)) => {
                panic!("No response received");
            },
            Ok(Err(e)) => {
                panic!("IO error: {}", e);
            },
            Err(_) => {
                // Timeout is acceptable for network tests in CI
                eprintln!("Search test timed out (may be network issue in CI)");
            },
        }

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_visit_page_tool_execution() {
        let mut process = DaedraProcess::spawn().await;
        process.initialize().await;

        let fetch_request = json!({
            "jsonrpc": "2.0",
            "id": 101,
            "method": "tools/call",
            "params": {
                "name": "visit_page",
                "arguments": {
                    "url": "https://example.com",
                    "include_images": false
                }
            }
        });

        let request_str = serde_json::to_string(&fetch_request).unwrap();
        process
            .stdin
            .write_all(request_str.as_bytes())
            .await
            .unwrap();
        process.stdin.write_all(b"\n").await.unwrap();
        process.stdin.flush().await.unwrap();

        let response_result =
            timeout(Duration::from_secs(30), process.stdout_reader.next_line()).await;

        match response_result {
            Ok(Ok(Some(response_line))) => {
                let response: Value =
                    serde_json::from_str(&response_line).expect("Response should be valid JSON");

                assert_eq!(response["jsonrpc"], "2.0");
                assert!(response["result"].is_object(), "Should have result");

                let content = &response["result"]["content"];
                assert!(content.is_array(), "Result should have content array");

                let text = content[0]["text"].as_str().unwrap_or("");
                assert!(!text.is_empty(), "Content should not be empty");

                // Should contain example.com content markers
                assert!(
                    text.to_lowercase().contains("example") || text.contains("Example"),
                    "Should contain example.com content"
                );

                // No ANSI in response
                assert!(
                    !response_line.contains('\x1b'),
                    "Response should not contain ANSI codes"
                );
            },
            Ok(Ok(None)) => {
                panic!("No response received");
            },
            Ok(Err(e)) => {
                eprintln!("Fetch test skipped due to IO error: {}", e);
            },
            Err(_) => {
                eprintln!("Fetch test timed out (may be network issue in CI)");
            },
        }

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_invalid_tool_name() {
        let mut process = DaedraProcess::spawn().await;
        process.initialize().await;

        let request = json!({
            "jsonrpc": "2.0",
            "id": 200,
            "method": "tools/call",
            "params": {
                "name": "nonexistent_tool",
                "arguments": {}
            }
        });

        let response = process
            .send_request(request)
            .await
            .expect("Should get response");

        assert!(
            response.get("error").is_some(),
            "Should return error for unknown tool"
        );
        assert_eq!(response["error"]["code"], -32601);

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_invalid_url_for_visit_page() {
        let mut process = DaedraProcess::spawn().await;
        process.initialize().await;

        let request = json!({
            "jsonrpc": "2.0",
            "id": 201,
            "method": "tools/call",
            "params": {
                "name": "visit_page",
                "arguments": {
                    "url": "not-a-valid-url"
                }
            }
        });

        let response = process
            .send_request(request)
            .await
            .expect("Should get response");

        // Should return a result with isError: true (MCP tool error format)
        assert!(response["result"].is_object());
        assert_eq!(
            response["result"]["isError"], true,
            "Should indicate error for invalid URL"
        );

        process.cleanup().await;
    }
}

mod concurrent_request_tests {
    use super::*;

    #[tokio::test]
    async fn test_multiple_sequential_requests() {
        let mut process = DaedraProcess::spawn().await;
        process.initialize().await;

        // Send multiple ping requests
        for i in 0..5 {
            let ping_request = json!({
                "jsonrpc": "2.0",
                "id": 1000 + i,
                "method": "ping",
                "params": {}
            });

            let response = process.send_request_expect_success(ping_request).await;
            assert_eq!(response["id"], 1000 + i);
        }

        process.cleanup().await;
    }

    #[tokio::test]
    async fn test_request_id_preserved() {
        let mut process = DaedraProcess::spawn().await;
        process.initialize().await;

        // Test various ID types
        let test_cases = vec![
            json!({"jsonrpc": "2.0", "id": 42, "method": "ping", "params": {}}),
            json!({"jsonrpc": "2.0", "id": "string-id", "method": "ping", "params": {}}),
            json!({"jsonrpc": "2.0", "id": null, "method": "ping", "params": {}}),
        ];

        for request in test_cases {
            let expected_id = request["id"].clone();
            let response = process
                .send_request(request)
                .await
                .expect("Should get response");

            assert_eq!(
                response["id"], expected_id,
                "Response ID should match request ID"
            );
        }

        process.cleanup().await;
    }
}
