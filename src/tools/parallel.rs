//! Opt-in, anonymous search through [Parallel Search MCP](https://docs.parallel.ai/integrations/mcp/search-mcp).
//!
//! Construct a `SearchProvider::new` with this backend to enable it. It is not
//! included in `SearchProvider::auto`. No account or API key is read or required.
//! Anonymous searches use Fast mode and are subject to the service's free-tier
//! limits. Region, recency and SafeSearch controls are not exposed by this tool;
//! callers must explicitly select `SafeSearchLevel::Off` and worldwide results.

use super::backend::SearchBackend;
use crate::types::{
    DaedraError, DaedraResult, ResultMetadata, SafeSearchLevel, SearchArgs, SearchResponse,
    SearchResult,
};
use async_trait::async_trait;
use reqwest::header::HeaderValue;
use reqwest::{Client, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

const ENDPOINT: &str = "https://search.parallel.ai/mcp";
const PROTOCOL: &str = "2025-03-26";
const USER_AGENT: &str = concat!(
    "daedra/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/dirmacs/daedra)"
);

/// Anonymous Parallel MCP search backend for explicitly constructed providers.
///
/// Reuse an instance for related searches: its session identifier stays stable.
/// Dropping a search future cancels the in-flight HTTP request. Each search has
/// a 30-second total timeout, including MCP initialization.
pub struct ParallelBackend {
    client: Client,
    endpoint: String,
    session_id: String,
    timeout: Duration,
}

#[derive(Deserialize)]
struct SearchData {
    results: Vec<Hit>,
}

#[derive(Deserialize)]
struct Hit {
    url: String,
    title: Option<String>,
    publish_date: Option<String>,
    excerpts: Vec<String>,
}

impl ParallelBackend {
    /// Create an anonymous backend. Environment API keys are never used.
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(30))
                .build()
                .expect("HTTP client"),
            endpoint: ENDPOINT.to_string(),
            session_id: uuid::Uuid::new_v4().to_string(),
            timeout: Duration::from_secs(30),
        }
    }

    async fn post(&self, body: Value, session: Option<&HeaderValue>) -> DaedraResult<Response> {
        let mut request = self
            .client
            .post(&self.endpoint)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTOCOL)
            .json(&body);
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        let response = request.send().await?;
        match response.status().as_u16() {
            429 => Err(DaedraError::RateLimitExceeded),
            401 | 403 => Err(DaedraError::SearchError(format!(
                "Parallel MCP HTTP {}",
                response.status()
            ))),
            _ => Ok(response.error_for_status()?),
        }
    }

    async fn search_data(&self, query: &str) -> DaedraResult<SearchData> {
        let response = self
            .post(
                json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
                    "protocolVersion":PROTOCOL, "capabilities":{},
                    "clientInfo":{"name":"daedra", "version":env!("CARGO_PKG_VERSION")}
                }}),
                None,
            )
            .await?;
        let session = response.headers().get("Mcp-Session-Id").cloned();
        let initialized = read_result(response, 1).await?;
        if initialized["protocolVersion"] != PROTOCOL {
            return Err(DaedraError::SearchError(
                "Parallel MCP negotiated an unsupported protocol version".to_string(),
            ));
        }
        self.post(
            json!({"jsonrpc":"2.0", "method":"notifications/initialized"}),
            session.as_ref(),
        )
        .await?;
        let response = self
            .post(
                json!({"jsonrpc":"2.0", "id":2, "method":"tools/call", "params":{
                    "name":"web_search", "arguments":{
                        "objective":query, "search_queries":[query], "session_id":self.session_id
                    }
                }}),
                session.as_ref(),
            )
            .await?;
        let result = read_result(response, 2).await?;
        if result["isError"] == true {
            return Err(tool_error(&result["content"].to_string()));
        }
        let data = if let Some(structured) = result.get("structuredContent") {
            structured.clone()
        } else {
            let text = result["content"]
                .as_array()
                .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text"))
                .and_then(|block| block["text"].as_str())
                .ok_or_else(|| {
                    DaedraError::SearchError("Parallel MCP returned no search data".to_string())
                })?;
            serde_json::from_str(text)?
        };
        Ok(serde_json::from_value(data)?)
    }
}

impl Default for ParallelBackend {
    fn default() -> Self {
        Self::new()
    }
}

fn tool_error(message: &str) -> DaedraError {
    let lower = message.to_lowercase();
    if ["rate limit", "quota", "too many requests"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        DaedraError::RateLimitExceeded
    } else {
        DaedraError::SearchError(format!("Parallel MCP: {message}"))
    }
}

async fn read_result(response: Response, id: u64) -> DaedraResult<Value> {
    let is_sse = response
        .headers()
        .get("Content-Type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    let body = response.text().await?;
    let value: Value = if is_sse {
        // POST response streams terminate after their JSON-RPC response. Ignore
        // keepalives and notifications; match the response to this request id.
        body.replace("\r\n", "\n")
            .split("\n\n")
            .filter_map(|event| {
                let data = event
                    .lines()
                    .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
                    .collect::<Vec<_>>()
                    .join("\n");
                serde_json::from_str::<Value>(&data).ok()
            })
            .find(|value| value["id"] == id)
            .ok_or_else(|| DaedraError::SearchError("Parallel MCP response missing".to_string()))?
    } else {
        serde_json::from_str(&body)?
    };
    if value["jsonrpc"] != "2.0" || value["id"] != id {
        return Err(DaedraError::SearchError(
            "Parallel MCP returned an invalid response envelope".to_string(),
        ));
    }
    if let Some(error) = value.get("error") {
        return Err(tool_error(&error.to_string()));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| DaedraError::SearchError("Parallel MCP response has no result".to_string()))
}

#[async_trait]
impl SearchBackend for ParallelBackend {
    fn name(&self) -> &str {
        "parallel"
    }

    async fn search(&self, args: &SearchArgs) -> DaedraResult<SearchResponse> {
        let opts = args.options.clone().unwrap_or_default();
        if args.query.trim().is_empty() || opts.num_results == 0 {
            return Err(DaedraError::InvalidArguments(
                "Parallel requires a nonempty query and at least one result".to_string(),
            ));
        }
        if opts.safe_search != SafeSearchLevel::Off
            || opts.region != "wt-wt"
            || opts.time_range.is_some()
        {
            return Err(DaedraError::InvalidArguments(
                "Parallel MCP does not expose SafeSearch, region or time-range filters; select \
                 safe_search OFF, region wt-wt and no time_range explicitly"
                    .to_string(),
            ));
        }
        let data = tokio::time::timeout(self.timeout, self.search_data(&args.query))
            .await
            .map_err(|_| DaedraError::Timeout)??;
        let results = data
            .results
            .into_iter()
            .filter(|hit| super::fetch::is_valid_url(&hit.url))
            .take(opts.num_results)
            .map(|hit| SearchResult {
                title: hit
                    .title
                    .unwrap_or_else(|| hit.url.clone())
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                metadata: ResultMetadata {
                    content_type: crate::url_classification::classify_search_url(&hit.url),
                    source: "parallel".to_string(),
                    favicon: None,
                    published_date: hit.publish_date,
                },
                url: hit.url,
                description: hit.excerpts.join("\n\n"),
            })
            .collect();
        Ok(SearchResponse::new(args.query.clone(), results, &opts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::SearchProvider;
    use crate::types::SearchOptions;
    use wiremock::matchers::{body_partial_json, header, headers, method, path};
    use wiremock::{Mock, MockBuilder, MockServer, ResponseTemplate};

    fn args() -> SearchArgs {
        SearchArgs {
            query: "Rust async runtime".to_string(),
            options: Some(SearchOptions {
                num_results: 1,
                safe_search: SafeSearchLevel::Off,
                backends: Some(vec!["parallel".to_string()]),
                ..Default::default()
            }),
        }
    }

    async fn backend() -> (MockServer, ParallelBackend) {
        let server = MockServer::start().await;
        let backend = ParallelBackend {
            endpoint: format!("{}/mcp", server.uri()),
            ..ParallelBackend::new()
        };
        assert_eq!(ENDPOINT, "https://search.parallel.ai/mcp");
        (server, backend)
    }

    fn rpc(method_name: &str) -> MockBuilder {
        Mock::given(method("POST"))
            .and(path("/mcp"))
            .and(header("User-Agent", USER_AGENT))
            .and(header("Content-Type", "application/json"))
            .and(headers(
                "Accept",
                vec!["application/json", "text/event-stream"],
            ))
            .and(header("MCP-Protocol-Version", PROTOCOL))
            .and(body_partial_json(json!({"method":method_name})))
    }

    async fn handshake(server: &MockServer) {
        rpc("initialize")
            .and(body_partial_json(json!({"id":1,"params":{
                "protocolVersion":PROTOCOL,"capabilities":{},
                "clientInfo":{"name":"daedra","version":env!("CARGO_PKG_VERSION")}
            }})))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Mcp-Session-Id", "transport-session")
                    .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":{
                        "protocolVersion":PROTOCOL,"capabilities":{"tools":{}},
                        "serverInfo":{"name":"fixture","version":"1"}
                    }})),
            )
            .expect(1)
            .mount(server)
            .await;
        rpc("notifications/initialized")
            .and(header("Mcp-Session-Id", "transport-session"))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(server)
            .await;
    }

    fn search_data() -> Value {
        json!({"results":[
            {"url":"javascript:alert(1)","excerpts":["invalid URL"]},
            {"url":"https://docs.rs/tokio","title":" Rust  async runtime ",
                "publish_date":"2026-10-01","excerpts":["Tokio Rust runtime","Async I/O"]},
            {"url":"https://example.com/extra","excerpts":["extra Rust result"]}
        ]})
    }

    async fn tool_response(server: &MockServer, response: ResponseTemplate) {
        rpc("tools/call")
            .and(header("Mcp-Session-Id", "transport-session"))
            .and(body_partial_json(json!({"jsonrpc":"2.0","id":2,"params":{
                "name":"web_search","arguments":{
                    "objective":"Rust async runtime","search_queries":["Rust async runtime"]
                }
            }})))
            .respond_with(response)
            .expect(1)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn provider_consumes_structured_results_and_sends_anonymous_requests() {
        let (server, backend) = backend().await;
        let session_id = backend.session_id.clone();
        handshake(&server).await;
        tool_response(
            &server,
            ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc":"2.0","id":2,"result":{"structuredContent":search_data(),"isError":false}
            })),
        )
        .await;
        let provider = SearchProvider::new(vec![Box::new(backend)]);
        let response = provider.search(&args()).await.unwrap();
        assert_eq!(response.data.len(), 1);
        let hit = &response.data[0];
        assert_eq!(hit.title, "Rust async runtime");
        assert_eq!(hit.url, "https://docs.rs/tokio");
        assert_eq!(hit.description, "Tokio Rust runtime\n\nAsync I/O");
        assert_eq!(hit.metadata.source, "parallel");
        assert_eq!(hit.metadata.published_date.as_deref(), Some("2026-10-01"));
        assert_eq!(response.metadata.search_context.safe_search, "OFF");
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3);
        for request in &requests {
            assert!(!request.headers.contains_key("Authorization"));
            assert_eq!(request.url.path(), "/mcp");
            assert_eq!(request.headers["User-Agent"], USER_AGENT);
        }
        let call: Value = serde_json::from_slice(&requests[2].body).unwrap();
        assert_eq!(call["params"]["arguments"]["session_id"], session_id);
        assert!(call["params"]["arguments"].get("model_name").is_none());
        server.verify().await;
    }

    #[tokio::test]
    async fn accepts_sse_with_json_text_tool_content() {
        let (server, backend) = backend().await;
        handshake(&server).await;
        let result = json!({"content":[{"type":"text","text":search_data().to_string()}]});
        let body = format!(
            ": keepalive\r\n\r\ndata: {{\"jsonrpc\":\"2.0\",\r\ndata: \"id\":2,\"result\":{result}}}\r\n\r\n"
        );
        tool_response(
            &server,
            ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"),
        )
        .await;
        let provider = SearchProvider::new(vec![Box::new(backend)]);
        assert_eq!(provider.search(&args()).await.unwrap().data.len(), 1);
        server.verify().await;
    }

    #[tokio::test]
    async fn related_searches_reuse_the_anonymous_session_identifier() {
        let (server, backend) = backend().await;
        let default = ParallelBackend::default();
        assert!(!default.requires_api_key());
        assert!(default.is_available());
        rpc("initialize")
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc":"2.0","id":1,"result":{"protocolVersion":PROTOCOL}
            })))
            .expect(2)
            .mount(&server)
            .await;
        rpc("notifications/initialized")
            .respond_with(ResponseTemplate::new(202))
            .expect(2)
            .mount(&server)
            .await;
        rpc("tools/call")
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc":"2.0","id":2,"result":{"structuredContent":search_data()}
            })))
            .expect(2)
            .mount(&server)
            .await;
        let provider = SearchProvider::new(vec![Box::new(backend)]);
        for _ in 0..2 {
            assert!(!provider.search(&args()).await.unwrap().data.is_empty());
        }
        let calls = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter_map(|request| serde_json::from_slice::<Value>(&request.body).ok())
            .filter(|body| body["method"] == "tools/call")
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0]["params"]["arguments"]["session_id"],
            calls[1]["params"]["arguments"]["session_id"]
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn provider_does_not_retry_http_or_tool_quotas() {
        for response in [
            ResponseTemplate::new(429),
            ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":2,
                "result":{"isError":true,"content":[{"type":"text","text":"Free tier quota exceeded"}]}
            })),
        ] {
            let (server, backend) = backend().await;
            handshake(&server).await;
            tool_response(&server, response).await;
            let provider = SearchProvider::new(vec![Box::new(backend)]);
            let error = provider.search(&args()).await.unwrap_err().to_string();
            assert!(error.contains("Rate limit exceeded"), "{error}");
            assert_eq!(server.received_requests().await.unwrap().len(), 3);
            server.verify().await;
        }
    }

    #[tokio::test]
    async fn malformed_and_error_responses_never_become_empty_successes() {
        for result in [
            json!({"jsonrpc":"2.0","id":99,"result":{"structuredContent":search_data()}}),
            json!({"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"search unavailable"}}),
            json!({"jsonrpc":"2.0","id":2,"result":{"isError":true,"content":[{"type":"text","text":"search unavailable"}]}}),
            json!({"jsonrpc":"2.0","id":2,"result":{"structuredContent":{}}}),
            json!({"jsonrpc":"2.0","id":2,"result":{"content":[]}}),
        ] {
            let (server, backend) = backend().await;
            handshake(&server).await;
            tool_response(&server, ResponseTemplate::new(200).set_body_json(result)).await;
            assert!(backend.search(&args()).await.is_err());
            server.verify().await;
        }
    }

    #[tokio::test]
    async fn unsupported_filters_and_disabled_selection_make_no_requests() {
        let (server, backend) = backend().await;
        for options in [
            SearchOptions::default(),
            SearchOptions {
                region: "us-en".to_string(),
                ..args().options.unwrap()
            },
            SearchOptions {
                time_range: Some("w".to_string()),
                ..args().options.unwrap()
            },
            SearchOptions {
                num_results: 0,
                ..args().options.unwrap()
            },
        ] {
            assert!(matches!(
                backend
                    .search(&SearchArgs {
                        options: Some(options),
                        ..args()
                    })
                    .await,
                Err(DaedraError::InvalidArguments(_))
            ));
        }
        assert!(
            backend
                .search(&SearchArgs {
                    query: " ".to_string(),
                    ..args()
                })
                .await
                .is_err()
        );
        let provider = SearchProvider::new(vec![Box::new(backend)]);
        let mut excluded = args();
        excluded.options.as_mut().unwrap().exclude_backends = Some(vec!["parallel".to_string()]);
        assert!(provider.search(&excluded).await.is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(
            !SearchProvider::auto()
                .available_backends()
                .contains(&"parallel")
        );
    }

    #[tokio::test]
    async fn total_timeout_and_cancellation_stop_the_calling_path() {
        let (server, mut backend) = backend().await;
        backend.timeout = Duration::from_millis(50);
        rpc("initialize")
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&server)
            .await;
        assert!(matches!(
            backend.search(&args()).await,
            Err(DaedraError::Timeout)
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);

        backend.timeout = Duration::from_secs(30);
        let provider = SearchProvider::new(vec![Box::new(backend)]);
        let task = tokio::spawn(async move { provider.search(&args()).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while server.received_requests().await.unwrap().len() < 2 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::sleep(Duration::from_millis(500)).await;
        // One timed-out initialization, one cancelled initialization, no
        // initialized notification, tools/call, or provider retry after abort.
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }
}
