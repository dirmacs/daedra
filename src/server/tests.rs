//! Server, transport, and handler tests.

use super::*;
use super::jsonrpc::{
    format_page_result, tool_error_response, tool_success_response,
};
use super::stdio::parse_tool_call_params;
use crate::types::{PageContent, SearchArgs, SearchResponse};
use crate::SERVER_NAME;
use serde_json::{Value, json};

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

        assert_eq!(tools.len(), 4);
        assert!(tools.iter().any(|t| t.name == "web_search"));
        assert!(tools.iter().any(|t| t.name == "search_duckduckgo"));
        assert!(tools.iter().any(|t| t.name == "visit_page"));
        assert!(tools.iter().any(|t| t.name == "crawl_site"));
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
        assert_eq!(tools.len(), 4);
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

    #[tokio::test]
    async fn test_handle_notifications_initialized() {
        let config = ServerConfig::default();
        let handler = DaedraHandler::new(config).unwrap();

        // Test the "notifications/initialized" variant (with prefix)
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "notifications/initialized".to_string(),
            params: None,
        };

        let response = handler.handle_request(request).await;
        // Should succeed, not return "Method not found"
        assert!(
            response.error.is_none(),
            "notifications/initialized should not return error"
        );
        assert!(response.result.is_some());
    }

    #[tokio::test]
    async fn test_handle_initialized_without_prefix() {
        let config = ServerConfig::default();
        let handler = DaedraHandler::new(config).unwrap();

        // Test the "initialized" variant (without prefix)
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "initialized".to_string(),
            params: None,
        };

        let response = handler.handle_request(request).await;
        assert!(
            response.error.is_none(),
            "initialized should not return error"
        );
        assert!(response.result.is_some());
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn test_handle_call_tool_web_search() {
        let config = ServerConfig::default();
        let handler = DaedraHandler::new(config).unwrap();

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "tools/call".to_string(),
            params: Some(json!({"name": "web_search", "arguments": {"query": "test"}})),
        };

        let response = handler.handle_request(request).await;
        assert!(response.error.is_none());
        let result = response.result.unwrap();
        assert_eq!(result["isError"], false);
        assert!(result["content"].as_array().is_some());
    }

    #[tokio::test]
    async fn test_handle_call_tool_unknown() {
        let config = ServerConfig::default();
        let handler = DaedraHandler::new(config).unwrap();

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "tools/call".to_string(),
            params: Some(json!({"name": "nonexistent", "arguments": {}})),
        };

        let response = handler.handle_request(request).await;
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn test_handle_call_tool_missing_params() {
        let config = ServerConfig::default();
        let handler = DaedraHandler::new(config).unwrap();

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "tools/call".to_string(),
            params: None,
        };

        let response = handler.handle_request(request).await;
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32602);
    }

    #[test]
    fn test_is_notification_no_id() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: "initialized".to_string(),
            params: None,
        };
        assert!(is_notification(&request));
    }

    #[test]
    fn test_is_notification_null_id() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::Null),
            method: "initialized".to_string(),
            params: None,
        };
        assert!(is_notification(&request));
    }

    #[test]
    fn test_is_notification_with_id() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "ping".to_string(),
            params: None,
        };
        assert!(!is_notification(&request));
    }

    #[tokio::test]
    async fn test_json_rpc_parse_error() {
        let config = ServerConfig::default();
        let handler = DaedraHandler::new(config).unwrap();

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "tools/call".to_string(),
            params: Some(json!({"name": "web_search", "arguments": {"not_query": true}})),
        };

        let response = handler.handle_request(request).await;
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32602);
    }

    #[tokio::test]
    async fn test_execute_search_caches_results() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let args = SearchArgs {
            query: "cache-test-unique-query-xyz".to_string(),
            options: None,
        };
        let options = args.options.clone().unwrap_or_default();
        let cached_response = SearchResponse::new(args.query.clone(), vec![], &options);
        handler
            .cache()
            .set_search(
                &args.query,
                &options.region,
                &options.safe_search.to_string(),
                cached_response.clone(),
            )
            .await;

        let result = handler.execute_search(args).await.unwrap();
        assert_eq!(result.data.len(), cached_response.data.len());
        assert_eq!(result.metadata.query, cached_response.metadata.query);
    }

    #[tokio::test]
    async fn test_handle_method_initialize() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler
            .handle_method("initialize", Some(json!(1)), None)
            .await;
        assert!(response.result.is_some());
        assert!(response.error.is_none());
        let result = response.result.unwrap();
        assert_eq!(result["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], SERVER_NAME);
    }

    #[tokio::test]
    async fn test_handle_method_ping() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler.handle_method("ping", Some(json!(1)), None).await;
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[tokio::test]
    async fn test_handle_method_tools_list() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler
            .handle_method("tools/list", Some(json!(1)), None)
            .await;
        assert!(response.result.is_some());
        let result = response.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 4);
    }

    #[tokio::test]
    async fn test_handle_method_unknown() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler.handle_method("foo", Some(json!(1)), None).await;
        let err = response.error.unwrap();
        assert_eq!(err.code, -32601);
        assert!(err.message.contains("foo"));
    }

    #[tokio::test]
    async fn test_handle_method_initialized() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler
            .handle_method("initialized", Some(json!(1)), None)
            .await;
        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap(), json!({}));
    }

    #[tokio::test]
    async fn test_handle_method_notifications_initialized() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler
            .handle_method("notifications/initialized", Some(json!(1)), None)
            .await;
        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap(), json!({}));
    }

    #[tokio::test]
    async fn test_handle_method_tools_call_missing_params() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler
            .handle_method("tools/call", Some(json!(1)), None)
            .await;
        assert!(response.result.is_none());
        let err = response.error.unwrap();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("Missing parameters"));
    }

    #[tokio::test]
    async fn test_handle_method_tools_call_unknown_tool() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler
            .handle_method(
                "tools/call",
                Some(json!(1)),
                Some(json!({"name": "unknown", "arguments": {}})),
            )
            .await;
        assert!(response.result.is_none());
        let err = response.error.unwrap();
        assert_eq!(err.code, -32601);
        assert!(err.message.contains("unknown"));
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn test_handle_method_tools_call_web_search_no_args() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler
            .handle_method(
                "tools/call",
                Some(json!(1)),
                Some(json!({"name": "web_search", "arguments": {}})),
            )
            .await;
        assert!(response.error.is_none());
        let result = response.result.unwrap();
        assert!(result.get("isError").is_some());
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn test_execute_search_returns_results() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let args = SearchArgs {
            query: "Rust programming language".to_string(),
            options: Some(crate::types::SearchOptions {
                num_results: 5,
                ..Default::default()
            }),
        };
        let response = handler.execute_search(args).await.unwrap();
        assert!(!response.data.is_empty(), "search should return results");
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn test_execute_search_caches_on_second_call() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let args = SearchArgs {
            query: "cache-second-call-unique-query-abc".to_string(),
            options: None,
        };
        let first = handler.execute_search(args.clone()).await;
        let second = handler.execute_search(args).await;
        assert!(
            first.is_ok(),
            "first search should succeed: {:?}",
            first.err()
        );
        assert!(
            second.is_ok(),
            "second search should succeed: {:?}",
            second.err()
        );
        assert!(!first.unwrap().data.is_empty());
        assert!(!second.unwrap().data.is_empty());
    }

    #[tokio::test]
    async fn test_handle_visit_page_malformed_args() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler
            .handle_visit_page(Some(json!(1)), json!({"url": 12345}))
            .await;
        assert!(response.result.is_none());
        let err = response.error.unwrap();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("Invalid fetch arguments"));
    }

    #[tokio::test]
    async fn test_handle_visit_page_invalid_url() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler
            .handle_method(
                "tools/call",
                Some(json!(1)),
                Some(json!({
                    "name": "visit_page",
                    "arguments": {"url": "ftp://example.com"}
                })),
            )
            .await;
        assert!(response.error.is_none());
        let result = response.result.unwrap();
        assert_eq!(result["isError"], true);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Invalid URL"));
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn test_handle_method_tools_call_web_search() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler
            .handle_method(
                "tools/call",
                Some(json!(1)),
                Some(json!({
                    "name": "web_search",
                    "arguments": {"query": "test"}
                })),
            )
            .await;
        assert!(response.error.is_none());
        let result = response.result.unwrap();
        assert_eq!(result["isError"], false);
        assert!(result["content"].as_array().is_some());
    }

    #[tokio::test]
    async fn test_handle_method_tools_call_visit_page_invalid() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler
            .handle_method(
                "tools/call",
                Some(json!(1)),
                Some(json!({
                    "name": "visit_page",
                    "arguments": {"url": "not-a-url"}
                })),
            )
            .await;
        assert!(response.error.is_none());
        let result = response.result.unwrap();
        assert_eq!(result["isError"], true);
    }

    #[test]
    fn test_parse_tool_call_params_valid() {
        let result = parse_tool_call_params(
            Some(json!({"name": "web_search", "arguments": {}})),
            Some(json!(1)),
        );
        assert!(result.is_ok());
        let (name, args) = result.unwrap();
        assert_eq!(name, "web_search");
        assert_eq!(args, json!({}));
    }

    #[test]
    fn test_parse_tool_call_params_missing() {
        let result = parse_tool_call_params(None, Some(json!(1)));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.error.unwrap().code, -32602);
    }

    #[test]
    fn test_parse_tool_call_params_no_name() {
        let result = parse_tool_call_params(Some(json!({})), Some(json!(1)));
        assert!(result.is_ok());
        let (name, args) = result.unwrap();
        assert_eq!(name, "");
        assert_eq!(args, json!({}));
    }

    #[test]
    fn test_parse_tool_call_params_with_args() {
        let result = parse_tool_call_params(
            Some(json!({
                "name": "visit_page",
                "arguments": {"url": "https://example.com"}
            })),
            Some(json!(1)),
        );
        assert!(result.is_ok());
        let (name, args) = result.unwrap();
        assert_eq!(name, "visit_page");
        assert_eq!(args["url"], "https://example.com");
    }

    #[test]
    fn test_tool_error_response_has_is_error() {
        let response = tool_error_response(Some(json!(1)), "something went wrong");
        let result = response.result.unwrap();
        assert_eq!(result["isError"], true);
        assert_eq!(
            result["content"][0]["text"].as_str().unwrap(),
            "something went wrong"
        );
    }

    #[test]
    fn test_tool_success_response_no_error() {
        let response = tool_success_response(Some(json!(1)), "ok".to_string());
        let result = response.result.unwrap();
        assert_eq!(result["isError"], false);
        assert_eq!(result["content"][0]["text"].as_str().unwrap(), "ok");
    }

    #[test]
    fn test_format_page_result() {
        let content = PageContent {
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            content: "Hello world".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            word_count: 2,
            content_type: None,
            links: None,
        };
        let formatted = format_page_result(&content);
        assert!(formatted.contains("Example"));
        assert!(formatted.contains("https://example.com"));
        assert!(formatted.contains("**Words:** 2"));
        assert!(formatted.contains("Hello world"));
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn test_handle_visit_page_valid_url() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler
            .handle_visit_page(Some(json!(1)), json!({"url": "https://example.com"}))
            .await;
        assert!(response.error.is_none());
        let result = response.result.unwrap();
        assert_eq!(result["isError"], false);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Example") || text.contains("example.com"));
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn test_handle_visit_page_valid_url_fetch_fails() {
        let handler = DaedraHandler::new(ServerConfig::default()).unwrap();
        let response = handler
            .handle_visit_page(Some(json!(1)), json!({"url": "https://127.0.0.1:1/"}))
            .await;
        assert!(response.error.is_none());
        let result = response.result.unwrap();
        assert_eq!(result["isError"], true);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Failed to fetch"));
    }
