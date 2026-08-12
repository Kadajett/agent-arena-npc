use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::{Client, header};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{
    mcp::protocol::{
        JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, PROTOCOL_VERSION, ToolCallEnvelope,
    },
    observability::{AnalyticsEvent, AnalyticsSink, EventLevel, redact},
};

const SESSION_HEADER: &str = "mcp-session-id";
const PROTOCOL_HEADER: &str = "mcp-protocol-version";

#[derive(Debug, Error, Clone)]
pub enum McpError {
    #[error("MCP request timed out after {timeout_ms} ms")]
    Timeout { timeout_ms: u64 },
    #[error("MCP transport failed: {message}")]
    Transport { message: String },
    #[error("MCP endpoint returned HTTP {status}: {message}")]
    HttpStatus { status: u16, message: String },
    #[error("MCP protocol error: {message}")]
    Protocol { message: String },
    #[error("MCP JSON-RPC error {code}: {message}")]
    JsonRpc { code: i64, message: String },
    #[error("MCP tool {tool} failed: {message}")]
    Tool { tool: String, message: String },
}

impl McpError {
    pub fn class(&self) -> &'static str {
        match self {
            Self::Timeout { .. } => "timeout",
            Self::Transport { .. } => "transport",
            Self::HttpStatus { .. } => "http_status",
            Self::Protocol { .. } => "protocol",
            Self::JsonRpc { .. } => "json_rpc",
            Self::Tool { .. } => "tool",
        }
    }

    pub fn is_session_loss(&self) -> bool {
        matches!(self, Self::HttpStatus { status: 404, .. })
            || matches!(
                self,
                Self::HttpStatus { status: 400, message }
                    if message.contains("Invalid or missing MCP session ID")
            )
            || self.to_string().contains("AGENT_NOT_CONNECTED")
    }
}

#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn request(
        &self,
        method: &str,
        params: Value,
        correlation_id: Uuid,
    ) -> Result<Value, McpError>;

    async fn notify(
        &self,
        method: &str,
        params: Value,
        correlation_id: Uuid,
    ) -> Result<(), McpError>;

    async fn reset_session(&self);

    async fn session_id(&self) -> Option<String>;

    async fn set_protocol_version(&self, _version: &str) {}

    async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        correlation_id: Uuid,
    ) -> Result<Value, McpError> {
        let result = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
                correlation_id,
            )
            .await?;
        decode_tool_result(name, result)
    }
}

pub struct HttpMcpTransport {
    client: Client,
    endpoint: String,
    api_key: String,
    timeout: Duration,
    next_id: AtomicU64,
    session_id: RwLock<Option<String>>,
    protocol_version: RwLock<String>,
    analytics: Arc<dyn AnalyticsSink>,
}

impl HttpMcpTransport {
    /// Construct a stateful MCP Streamable HTTP transport.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be constructed.
    pub fn new(
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        timeout: Duration,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> Result<Self, McpError> {
        let client = Client::builder()
            .timeout(timeout)
            .user_agent("AgentArena-RustHarness/0.1")
            .build()
            .map_err(|error| McpError::Transport {
                message: error.to_string(),
            })?;
        Ok(Self {
            client,
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            timeout,
            next_id: AtomicU64::new(1),
            session_id: RwLock::new(None),
            protocol_version: RwLock::new(PROTOCOL_VERSION.to_owned()),
            analytics,
        })
    }

    pub async fn update_protocol_version(&self, version: impl Into<String>) {
        *self.protocol_version.write().await = version.into();
    }

    async fn post_builder(&self) -> reqwest::RequestBuilder {
        let protocol_version = self.protocol_version.read().await.clone();
        let mut request = self
            .client
            .post(&self.endpoint)
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header(PROTOCOL_HEADER, protocol_version);
        if let Some(session_id) = self.session_id.read().await.as_deref() {
            request = request.header(SESSION_HEADER, session_id);
        }
        request
    }

    async fn capture_session(&self, response: &reqwest::Response) -> bool {
        let Some(value) = response.headers().get(SESSION_HEADER) else {
            return false;
        };
        let Ok(value) = value.to_str() else {
            return false;
        };
        let mut session = self.session_id.write().await;
        let changed = session.as_deref() != Some(value);
        *session = Some(value.to_owned());
        changed
    }

    fn safe_error(&self, error: &impl ToString) -> String {
        redact(&error.to_string(), &[&self.api_key])
    }

    fn reqwest_error(&self, error: &reqwest::Error) -> McpError {
        if error.is_timeout() {
            McpError::Timeout {
                timeout_ms: duration_millis(self.timeout),
            }
        } else {
            McpError::Transport {
                message: self.safe_error(error),
            }
        }
    }

    fn record_failure(
        &self,
        event_name: &'static str,
        method: &str,
        request_id: Option<u64>,
        correlation_id: Uuid,
        started: Instant,
        error: &McpError,
    ) {
        let mut event = AnalyticsEvent::new(event_name, EventLevel::Warn)
            .correlation(correlation_id)
            .attribute("method", method)
            .attribute("duration_ms", millis(started.elapsed()))
            .attribute("error_class", error.class())
            .attribute("error", self.safe_error(error));
        if let Some(request_id) = request_id {
            event = event.attribute("request_id", request_id);
        }
        self.analytics.record(event);
    }

    async fn decode_response(
        &self,
        response: reqwest::Response,
        method: &str,
        request_id: u64,
        correlation_id: Uuid,
        started: Instant,
    ) -> Result<Value, McpError> {
        let status = response.status();
        let session_changed = self.capture_session(&response).await;
        if !status.is_success() {
            let message = response.text().await.map_or_else(
                |error| self.safe_error(&error),
                |body| self.safe_error(&body),
            );
            let error = McpError::HttpStatus {
                status: status.as_u16(),
                message,
            };
            self.record_failure(
                "mcp.request_failed",
                method,
                Some(request_id),
                correlation_id,
                started,
                &error,
            );
            return Err(error);
        }
        let is_sse = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("text/event-stream"));
        let parsed = if is_sse {
            let stream = response.bytes_stream();
            match tokio::time::timeout(
                self.timeout,
                read_matching_sse(Box::pin(stream), request_id, duration_millis(self.timeout)),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(McpError::Timeout {
                    timeout_ms: duration_millis(self.timeout),
                }),
            }
        } else {
            match response.json().await {
                Ok(value) => parse_matching_response(value, request_id),
                Err(error) if error.is_timeout() => Err(McpError::Timeout {
                    timeout_ms: duration_millis(self.timeout),
                }),
                Err(error) => Err(McpError::Protocol {
                    message: self.safe_error(&error),
                }),
            }
        };
        let parsed = parsed.inspect_err(|error| {
            self.record_failure(
                "mcp.request_failed",
                method,
                Some(request_id),
                correlation_id,
                started,
                error,
            );
        })?;
        self.analytics.record(
            AnalyticsEvent::new("mcp.request_completed", EventLevel::Debug)
                .correlation(correlation_id)
                .attribute("method", method)
                .attribute("request_id", request_id)
                .attribute("duration_ms", millis(started.elapsed()))
                .attribute("response_mode", if is_sse { "sse" } else { "json" })
                .attribute("session_changed", session_changed),
        );
        Ok(parsed)
    }
}

#[async_trait]
impl McpTransport for HttpMcpTransport {
    async fn request(
        &self,
        method: &str,
        params: Value,
        correlation_id: Uuid,
    ) -> Result<Value, McpError> {
        let request_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let started = Instant::now();
        self.analytics.record(
            AnalyticsEvent::new("mcp.request_started", EventLevel::Debug)
                .correlation(correlation_id)
                .attribute("method", method)
                .attribute("request_id", request_id),
        );
        let envelope = JsonRpcRequest {
            jsonrpc: "2.0",
            id: request_id,
            method,
            params,
        };
        let response = match tokio::time::timeout(
            self.timeout,
            self.post_builder().await.json(&envelope).send(),
        )
        .await
        {
            Err(_) => {
                let error = McpError::Timeout {
                    timeout_ms: duration_millis(self.timeout),
                };
                self.record_failure(
                    "mcp.request_failed",
                    method,
                    Some(request_id),
                    correlation_id,
                    started,
                    &error,
                );
                return Err(error);
            }
            Ok(Err(error)) => {
                let error = self.reqwest_error(&error);
                self.record_failure(
                    "mcp.request_failed",
                    method,
                    Some(request_id),
                    correlation_id,
                    started,
                    &error,
                );
                return Err(error);
            }
            Ok(Ok(response)) => response,
        };
        self.decode_response(response, method, request_id, correlation_id, started)
            .await
    }

    async fn notify(
        &self,
        method: &str,
        params: Value,
        correlation_id: Uuid,
    ) -> Result<(), McpError> {
        let started = Instant::now();
        self.analytics.record(
            AnalyticsEvent::new("mcp.notification_started", EventLevel::Debug)
                .correlation(correlation_id)
                .attribute("method", method),
        );
        let envelope = JsonRpcNotification {
            jsonrpc: "2.0",
            method,
            params,
        };
        let response = match tokio::time::timeout(
            self.timeout,
            self.post_builder().await.json(&envelope).send(),
        )
        .await
        {
            Err(_) => {
                let error = McpError::Timeout {
                    timeout_ms: duration_millis(self.timeout),
                };
                self.record_failure(
                    "mcp.notification_failed",
                    method,
                    None,
                    correlation_id,
                    started,
                    &error,
                );
                return Err(error);
            }
            Ok(Err(error)) => {
                let error = self.reqwest_error(&error);
                self.record_failure(
                    "mcp.notification_failed",
                    method,
                    None,
                    correlation_id,
                    started,
                    &error,
                );
                return Err(error);
            }
            Ok(Ok(response)) => response,
        };
        let status = response.status();
        self.capture_session(&response).await;
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let error = McpError::HttpStatus {
                status: status.as_u16(),
                message: self.safe_error(&body),
            };
            self.record_failure(
                "mcp.notification_failed",
                method,
                None,
                correlation_id,
                started,
                &error,
            );
            return Err(error);
        }
        self.analytics.record(
            AnalyticsEvent::new("mcp.notification_completed", EventLevel::Debug)
                .correlation(correlation_id)
                .attribute("method", method)
                .attribute("duration_ms", millis(started.elapsed())),
        );
        Ok(())
    }

    async fn reset_session(&self) {
        *self.session_id.write().await = None;
        let mut protocol_version = self.protocol_version.write().await;
        PROTOCOL_VERSION.clone_into(&mut *protocol_version);
    }

    async fn session_id(&self) -> Option<String> {
        self.session_id.read().await.clone()
    }

    async fn set_protocol_version(&self, version: &str) {
        self.update_protocol_version(version).await;
    }
}

fn decode_tool_result(tool: &str, result: Value) -> Result<Value, McpError> {
    let envelope: ToolCallEnvelope =
        serde_json::from_value(result).map_err(|error| McpError::Protocol {
            message: format!("tools/call returned an invalid envelope: {error}"),
        })?;
    let body = if let Some(value) = envelope.structured_content {
        value
    } else {
        let text = envelope
            .content
            .iter()
            .find(|content| content.kind == "text")
            .and_then(|content| content.text.as_deref())
            .ok_or_else(|| McpError::Protocol {
                message: format!("tool {tool} returned no text or structured content"),
            })?;
        match serde_json::from_str(text) {
            Ok(value) => value,
            Err(error) if text.trim_start().starts_with(['{', '[']) => {
                return Err(McpError::Protocol {
                    message: format!("tool {tool} returned invalid JSON text: {error}"),
                });
            }
            Err(_) => json!({ "message": text }),
        }
    };
    if envelope.is_error {
        let code = body.get("error").and_then(Value::as_str);
        let message = body
            .get("message")
            .and_then(Value::as_str)
            .or(code)
            .unwrap_or("tool returned an error")
            .to_owned();
        let message = code.map_or(message.clone(), |code| {
            if code == message {
                message.clone()
            } else {
                format!("{code}: {message}")
            }
        });
        return Err(McpError::Tool {
            tool: tool.to_owned(),
            message,
        });
    }
    Ok(body)
}

fn parse_matching_response(value: Value, request_id: u64) -> Result<Value, McpError> {
    let response: JsonRpcResponse =
        serde_json::from_value(value).map_err(|error| McpError::Protocol {
            message: format!("invalid JSON-RPC response: {error}"),
        })?;
    if !response.has_id(request_id) {
        return Err(McpError::Protocol {
            message: format!("response did not match request id {request_id}"),
        });
    }
    response_value(response)
}

fn response_value(response: JsonRpcResponse) -> Result<Value, McpError> {
    if let Some(error) = response.error {
        return Err(McpError::JsonRpc {
            code: error.code,
            message: error.message,
        });
    }
    response.result.ok_or_else(|| McpError::Protocol {
        message: "JSON-RPC response contained neither result nor error".to_owned(),
    })
}

type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

async fn read_matching_sse(
    mut stream: ByteStream,
    request_id: u64,
    timeout_ms: u64,
) -> Result<Value, McpError> {
    let mut buffered = String::new();
    let mut data_lines = Vec::new();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|error| {
            if error.is_timeout() {
                McpError::Timeout { timeout_ms }
            } else {
                McpError::Transport {
                    message: error.to_string(),
                }
            }
        })?;
        buffered.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(newline) = buffered.find('\n') {
            let line = buffered[..newline].trim_end_matches('\r').to_owned();
            buffered.drain(..=newline);
            if let Some(data) = line.strip_prefix("data:") {
                data_lines.push(data.trim_start().to_owned());
                if let Some(result) = try_sse_value(&data_lines, request_id)? {
                    return Ok(result);
                }
            } else if line.is_empty() {
                data_lines.clear();
            }
        }
    }
    Err(McpError::Protocol {
        message: "MCP event stream closed without the matching response".to_owned(),
    })
}

fn try_sse_value(data_lines: &[String], request_id: u64) -> Result<Option<Value>, McpError> {
    let Ok(value) = serde_json::from_str::<Value>(&data_lines.join("\n")) else {
        return Ok(None);
    };
    let Ok(response) = serde_json::from_value::<JsonRpcResponse>(value) else {
        return Ok(None);
    };
    if !response.has_id(request_id) {
        return Ok(None);
    }
    response_value(response).map(Some)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures_util::stream;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    use crate::observability::RecordingAnalyticsSink;

    use super::*;

    #[test]
    fn decodes_text_and_structured_tool_results() {
        let text = json!({
            "content": [{"type": "text", "text": "{\"moved\":true}"}]
        });
        assert_eq!(
            decode_tool_result("move", text).expect("text")["moved"],
            true
        );

        let structured = json!({
            "structuredContent": {"moved": true},
            "content": []
        });
        assert_eq!(
            decode_tool_result("move", structured).expect("structured")["moved"],
            true
        );

        let plain_text = json!({
            "content": [{"type": "text", "text": "Movement started."}]
        });
        assert_eq!(
            decode_tool_result("move", plain_text).expect("plain text")["message"],
            "Movement started."
        );
    }

    #[test]
    fn preserves_backend_error_code_for_session_loss_classification() {
        let envelope = json!({
            "isError": true,
            "content": [{
                "type": "text",
                "text": "{\"error\":\"AGENT_NOT_CONNECTED\",\"message\":\"gone\"}"
            }]
        });
        let error = decode_tool_result("arena_observe", envelope).expect_err("tool error");
        assert!(error.is_session_loss());
        assert!(error.to_string().contains("AGENT_NOT_CONNECTED"));
    }

    #[test]
    fn classifies_the_production_invalid_http_session_response_as_session_loss() {
        let error = McpError::HttpStatus {
            status: 400,
            message: "Invalid or missing MCP session ID.".to_owned(),
        };

        assert!(error.is_session_loss());
        assert!(
            !McpError::HttpStatus {
                status: 400,
                message: "ordinary invalid tool arguments".to_owned(),
            }
            .is_session_loss()
        );
    }

    #[tokio::test]
    async fn sse_ignores_notifications_and_joins_split_data_lines() {
        let chunks = vec![
            Ok(Bytes::from_static(
                b"data: {\"jsonrpc\":\"2.0\",\"method\":\"note\"}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"jsonrpc\":\"2.0\",\"id\":7,\n",
            )),
            Ok(Bytes::from_static(b"data: \"result\":{\"ok\":true}}\n\n")),
        ];
        let stream: ByteStream = Box::pin(stream::iter(chunks));
        let value = read_matching_sse(stream, 7, 1_000)
            .await
            .expect("matching event");
        assert_eq!(value["ok"], true);
    }

    #[tokio::test]
    async fn sse_returns_without_waiting_for_stream_closure() {
        let first = stream::once(async {
            Ok(Bytes::from_static(
                b"data: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"ok\":true}}\n\n",
            ))
        });
        let never = stream::pending::<Result<Bytes, reqwest::Error>>();
        let stream: ByteStream = Box::pin(first.chain(never));
        let result = tokio::time::timeout(
            Duration::from_millis(50),
            read_matching_sse(stream, 3, 1_000),
        )
        .await
        .expect("parser must not wait for close")
        .expect("matching result");
        assert_eq!(result["ok"], true);
    }

    #[tokio::test]
    async fn http_transport_captures_and_reuses_session_header() {
        let first = json_response(
            1,
            &json!({"first": true}),
            &[("mcp-session-id", "session-abc")],
        );
        let second = json_response(2, &json!({"second": true}), &[]);
        let (endpoint, requests, server) = spawn_server(vec![first, second]).await;
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let transport = HttpMcpTransport::new(
            endpoint,
            "secret-token",
            Duration::from_secs(1),
            analytics.clone(),
        )
        .expect("transport");

        transport
            .request("first", json!({}), Uuid::new_v4())
            .await
            .expect("first request");
        transport
            .request("second", json!({}), Uuid::new_v4())
            .await
            .expect("second request");
        server.await.expect("server");

        assert_eq!(transport.session_id().await.as_deref(), Some("session-abc"));
        let requests = requests.lock().expect("requests");
        assert!(!requests[0].to_ascii_lowercase().contains("mcp-session-id:"));
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("mcp-session-id: session-abc")
        );
        assert!(
            requests[0]
                .to_ascii_lowercase()
                .contains("mcp-protocol-version: 2025-06-18")
        );
        assert!(
            analytics
                .events()
                .iter()
                .any(|event| event.name == "mcp.request_completed")
        );
    }

    #[tokio::test]
    async fn http_sse_returns_while_server_keeps_connection_open() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let endpoint = format!("http://{}/mcp", listener.local_addr().expect("address"));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let _ = read_request(&mut socket).await;
            let event = b"data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n";
            let headers = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nconnection: keep-alive\r\n\r\n";
            socket.write_all(headers.as_bytes()).await.expect("headers");
            socket
                .write_all(format!("{:x}\r\n", event.len()).as_bytes())
                .await
                .expect("chunk size");
            socket.write_all(event).await.expect("event");
            socket.write_all(b"\r\n").await.expect("chunk end");
            socket.flush().await.expect("flush");
            tokio::time::sleep(Duration::from_secs(2)).await;
        });
        let transport = HttpMcpTransport::new(
            endpoint,
            "token",
            Duration::from_secs(1),
            Arc::new(RecordingAnalyticsSink::default()),
        )
        .expect("transport");

        let result = tokio::time::timeout(
            Duration::from_millis(200),
            transport.request("streamed", json!({}), Uuid::new_v4()),
        )
        .await
        .expect("must return before server closes")
        .expect("request");
        assert_eq!(result["ok"], true);
        server.abort();
    }

    #[tokio::test]
    async fn timeout_and_http_errors_are_classified_observable_and_redacted() {
        let secret = "never-log-this-token";
        let response = format!(
            "HTTP/1.1 401 Unauthorized\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{secret}",
            secret.len()
        );
        let (endpoint, _, server) = spawn_server(vec![response]).await;
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let transport =
            HttpMcpTransport::new(endpoint, secret, Duration::from_secs(1), analytics.clone())
                .expect("transport");
        let error = transport
            .request("rejected", json!({}), Uuid::new_v4())
            .await
            .expect_err("HTTP error");
        server.await.expect("server");

        assert_eq!(error.class(), "http_status");
        assert!(!error.to_string().contains(secret));
        let failure = analytics
            .events()
            .into_iter()
            .find(|event| event.name == "mcp.request_failed")
            .expect("failure event");
        assert_eq!(failure.attributes["error_class"], "http_status");
        assert!(
            !serde_json::to_string(&failure.attributes)
                .expect("attributes")
                .contains(secret)
        );
    }

    #[tokio::test]
    async fn request_timeout_is_classified_and_emits_terminal_event() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let endpoint = format!("http://{}/mcp", listener.local_addr().expect("address"));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let _ = read_request(&mut socket).await;
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let transport = HttpMcpTransport::new(
            endpoint,
            "token",
            Duration::from_millis(30),
            analytics.clone(),
        )
        .expect("transport");

        let error = transport
            .request("slow", json!({}), Uuid::new_v4())
            .await
            .expect_err("timeout");
        assert_eq!(error.class(), "timeout");
        let events = analytics.events();
        let failures = events
            .iter()
            .filter(|event| event.name == "mcp.request_failed")
            .collect::<Vec<_>>();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].attributes["error_class"], "timeout");
        server.abort();
    }

    #[tokio::test]
    async fn failed_notification_has_its_own_terminal_event() {
        let response =
            "HTTP/1.1 500 Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                .to_owned();
        let (endpoint, _, server) = spawn_server(vec![response]).await;
        let analytics = Arc::new(RecordingAnalyticsSink::default());
        let transport =
            HttpMcpTransport::new(endpoint, "token", Duration::from_secs(1), analytics.clone())
                .expect("transport");

        transport
            .notify("notifications/initialized", json!({}), Uuid::new_v4())
            .await
            .expect_err("notification failure");
        server.await.expect("server");
        let names = analytics
            .events()
            .into_iter()
            .map(|event| event.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names
                .iter()
                .filter(|name| name.as_str() == "mcp.notification_failed")
                .count(),
            1
        );
        assert!(!names.iter().any(|name| name == "mcp.request_failed"));
    }

    async fn spawn_server(
        responses: Vec<String>,
    ) -> (String, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let endpoint = format!("http://{}/mcp", listener.local_addr().expect("address"));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let server = tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.expect("accept");
                let request = read_request(&mut socket).await;
                recorded.lock().expect("requests").push(request);
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("response");
            }
        });
        (endpoint, requests, server)
    }

    async fn read_request(socket: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 2048];
        loop {
            let count = socket.read(&mut buffer).await.expect("request bytes");
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(header_end) = find_bytes(&bytes, b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if bytes.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    fn json_response(id: u64, result: &Value, headers: &[(&str, &str)]) -> String {
        use std::fmt::Write;

        let body = json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string();
        let extra = headers
            .iter()
            .fold(String::new(), |mut output, (name, value)| {
                write!(output, "{name}: {value}\r\n").expect("writing to a string cannot fail");
                output
            });
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n{extra}content-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    }
}
