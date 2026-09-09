//! HTTP transport — connects to a remote MCP server via Streamable HTTP.
//!
//! Uses HTTP POST for JSON-RPC requests and receives responses directly.
//! Session management via `Mcp-Session-Id` header.

use crate::jsonrpc::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::transport::Transport;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use temm1e_core::types::error::Temm1eError;
use tokio::sync::RwLock;
use tracing::debug;

pub struct HttpTransport {
    url: String,
    client: reqwest::Client,
    next_id: AtomicU64,
    session_id: Arc<RwLock<Option<String>>>,
    alive: AtomicBool,
    protocol_version: RwLock<Option<String>>,
    extra_headers: HashMap<String, String>,
    server_name: String,
}

impl HttpTransport {
    pub fn new(
        server_name: &str,
        url: &str,
        timeout: Duration,
        extra_headers: HashMap<String, String>,
    ) -> Result<Self, Temm1eError> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| Temm1eError::Tool(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            url: url.to_string(),
            client,
            next_id: AtomicU64::new(1),
            session_id: Arc::new(RwLock::new(None)),
            alive: AtomicBool::new(true),
            protocol_version: RwLock::new(None),
            extra_headers,
            server_name: server_name.to_string(),
        })
    }

    /// Build the HTTP request with common headers.
    fn build_request(&self, body: &str) -> Result<reqwest::RequestBuilder, Temm1eError> {
        let mut builder = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");

        for (key, value) in &self.extra_headers {
            builder = builder.header(key.as_str(), value.as_str());
        }

        Ok(builder.body(body.to_string()))
    }
}

#[async_trait]
impl Transport for HttpTransport {
    async fn send(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<JsonRpcResponse, Temm1eError> {
        if !self.alive.load(Ordering::Relaxed) {
            return Err(Temm1eError::Tool(format!(
                "MCP HTTP transport for '{}' is closed",
                self.server_name
            )));
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = JsonRpcRequest::new(id, method, params);
        let json = serde_json::to_string(&request)
            .map_err(|e| Temm1eError::Tool(format!("Failed to serialize request: {}", e)))?;

        let mut http_req = self.build_request(&json)?;
        if let Some(version) = self.protocol_version.read().await.as_ref() {
            http_req = http_req.header("MCP-Protocol-Version", version);
        }

        // Add session ID if we have one
        if let Some(sid) = self.session_id.read().await.as_ref() {
            http_req = http_req.header("Mcp-Session-Id", sid.as_str());
        }

        debug!(
            server = %self.server_name,
            method = %method,
            "Sending MCP HTTP request"
        );

        let http_resp = http_req.send().await.map_err(|e| {
            Temm1eError::Tool(format!(
                "MCP HTTP request to '{}' failed: {}",
                self.server_name,
                e.without_url()
            ))
        })?;

        let status = http_resp.status();
        if !status.is_success() {
            return Err(Temm1eError::Tool(format!(
                "MCP HTTP request to '{}' returned {}",
                self.server_name, status
            )));
        }
        let session = http_resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let response = read_response(http_resp, id).await?;
        if method == "initialize" && response.error.is_none() {
            if let Some(session) = session {
                *self.session_id.write().await = Some(session);
            }
        }

        Ok(response)
    }

    async fn notify(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<(), Temm1eError> {
        if !self.alive.load(Ordering::Relaxed) {
            return Err(Temm1eError::Tool(format!(
                "MCP HTTP transport for '{}' is closed",
                self.server_name
            )));
        }

        let notification = JsonRpcNotification::new(method, params);
        let json = serde_json::to_string(&notification)
            .map_err(|e| Temm1eError::Tool(format!("Failed to serialize notification: {}", e)))?;

        let mut http_req = self.build_request(&json)?;
        if let Some(version) = self.protocol_version.read().await.as_ref() {
            http_req = http_req.header("MCP-Protocol-Version", version);
        }

        if let Some(sid) = self.session_id.read().await.as_ref() {
            http_req = http_req.header("Mcp-Session-Id", sid.as_str());
        }

        let resp = http_req.send().await.map_err(|e| {
            Temm1eError::Tool(format!(
                "MCP HTTP notification to '{}' failed: {}",
                self.server_name,
                e.without_url()
            ))
        })?;

        if !resp.status().is_success() {
            return Err(Temm1eError::Tool(format!(
                "MCP HTTP notification returned {}",
                resp.status()
            )));
        }

        Ok(())
    }

    async fn set_protocol_version(&self, version: &str) -> Result<(), Temm1eError> {
        *self.protocol_version.write().await = Some(version.to_owned());
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    async fn close(&self) -> Result<(), Temm1eError> {
        self.alive.store(false, Ordering::Relaxed);
        Ok(())
    }
}

// Bound bytes even when Content-Length is absent or a server keeps sending events.
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

fn decode_response(bytes: &[u8], expected_id: u64) -> Result<JsonRpcResponse, Temm1eError> {
    let response: JsonRpcResponse = serde_json::from_slice(bytes)
        .map_err(|_| Temm1eError::Tool("MCP returned invalid JSON-RPC".into()))?;
    if response.jsonrpc != "2.0"
        || response.id != Some(expected_id)
        || response.result.is_some() == response.error.is_some()
    {
        return Err(Temm1eError::Tool(
            "MCP returned an invalid response envelope or request ID".into(),
        ));
    }
    Ok(response)
}

#[derive(Default)]
struct SseDecoder {
    line: Vec<u8>,
    data: Vec<u8>,
    after_cr: bool,
}

impl SseDecoder {
    fn push(&mut self, byte: u8, id: u64) -> Result<Option<JsonRpcResponse>, Temm1eError> {
        if self.after_cr && byte == b'\n' {
            self.after_cr = false;
            return Ok(None);
        }
        self.after_cr = byte == b'\r';
        if byte != b'\n' && byte != b'\r' {
            self.line.push(byte);
            return Ok(None);
        }
        let line = std::mem::take(&mut self.line);
        if line.is_empty() {
            if self.data.is_empty() {
                return Ok(None);
            }
            let data = std::mem::take(&mut self.data);
            let value: serde_json::Value = serde_json::from_slice(&data)
                .map_err(|_| Temm1eError::Tool("MCP SSE event contains invalid JSON".into()))?;
            if value.get("id").is_some() {
                return decode_response(&data, id).map(Some);
            }
            // Notifications may precede the result. They cannot complete a call.
            if value["jsonrpc"] != "2.0" || !value["method"].is_string() {
                return Err(Temm1eError::Tool(
                    "MCP SSE event has an invalid notification".into(),
                ));
            }
        } else if let Some(data) = line.strip_prefix(b"data:") {
            self.data
                .extend_from_slice(data.strip_prefix(b" ").unwrap_or(data));
            self.data.push(b'\n');
        } else if line == b"data" {
            self.data.push(b'\n');
        }
        Ok(None)
    }
}

async fn read_response(
    mut response: reqwest::Response,
    id: u64,
) -> Result<JsonRpcResponse, Temm1eError> {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    let sse = content_type.eq_ignore_ascii_case("text/event-stream");
    if !sse && !content_type.eq_ignore_ascii_case("application/json") {
        return Err(Temm1eError::Tool(
            "MCP returned an unsupported response Content-Type".into(),
        ));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
    {
        return Err(Temm1eError::Tool(
            "MCP response exceeds 32 MiB limit".into(),
        ));
    }
    let mut total = 0usize;
    let mut bytes = Vec::new();
    let mut decoder = SseDecoder::default();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| Temm1eError::Tool(format!("MCP response read failed: {}", e.without_url())))?
    {
        total = total.saturating_add(chunk.len());
        if total > MAX_RESPONSE_BYTES {
            return Err(Temm1eError::Tool(
                "MCP response exceeds 32 MiB limit".into(),
            ));
        }
        if sse {
            for byte in chunk {
                if let Some(result) = decoder.push(byte, id)? {
                    return Ok(result);
                }
            }
        } else {
            bytes.extend_from_slice(&chunk);
        }
    }
    if sse {
        Err(Temm1eError::Tool(
            "MCP event stream ended without a matching response".into(),
        ))
    } else {
        decode_response(&bytes, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn http_negotiation_headers_and_streamed_result_work_together() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for step in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let (offset, length) = loop {
                    let mut chunk = [0; 1024];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .unwrap()
                            .trim()
                            .parse()
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                while bytes.len() < offset + length {
                    let mut chunk = [0; 1024];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let headers = String::from_utf8_lossy(&bytes[..offset]).to_lowercase();
                let request: serde_json::Value =
                    serde_json::from_slice(&bytes[offset..offset + length]).unwrap();
                let (content_type, body) = match step {
                    0 => {
                        assert_eq!(request["method"], "initialize");
                        ("application/json", serde_json::json!({"jsonrpc":"2.0", "id":request["id"], "result":{
                            "protocolVersion":"2025-11-25", "serverInfo":{"name":"fixture", "version":"1"}
                        }}).to_string())
                    }
                    1 => {
                        assert_eq!(request["method"], "notifications/initialized");
                        ("application/json", String::new())
                    }
                    _ => {
                        assert_eq!(request["method"], "tools/list");
                        (
                            "text/event-stream",
                            format!(
                                "data: {}\n\n",
                                serde_json::json!({
                                    "jsonrpc":"2.0", "id":request["id"], "result":{"tools":[]}
                                })
                            ),
                        )
                    }
                };
                if step > 0 {
                    assert!(headers.contains("mcp-protocol-version: 2025-11-25"));
                    assert!(headers.contains("mcp-session-id: fixture-session"));
                }
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nMcp-Session-Id: fixture-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let transport = Arc::new(
            HttpTransport::new("fixture", &url, Duration::from_secs(2), HashMap::new()).unwrap(),
        );
        let client = crate::client::McpClient::new("fixture", transport);
        tokio::time::timeout(Duration::from_secs(5), async {
            client.initialize().await.unwrap();
            assert!(client.list_tools().await.unwrap().is_empty());
            server.await.unwrap();
        })
        .await
        .unwrap();
    }

    #[test]
    fn sse_handles_notifications_multiline_data_and_all_line_endings() {
        for ending in ["\n", "\r\n", "\r"] {
            let lines = [
                ": heartbeat",
                "",
                "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}",
                "",
                "event: message",
                "data: {\"jsonrpc\":\"2.0\",",
                "data: \"id\":7,\"result\":{\"text\":\"雪\"}}",
                "",
                "",
            ];
            let stream = lines.join(ending);
            let mut decoder = SseDecoder::default();
            let mut responses = Vec::new();
            for byte in stream.bytes() {
                if let Some(result) = decoder.push(byte, 7).unwrap() {
                    responses.push(result);
                }
            }
            assert_eq!(responses.len(), 1);
            assert_eq!(responses[0].result.as_ref().unwrap()["text"], "雪");
        }
    }

    #[test]
    fn malformed_or_mismatched_response_cannot_complete_request() {
        for body in [
            r#"{"jsonrpc":"2.0","id":8,"result":{}}"#,
            r#"{"jsonrpc":"1.0","id":7,"result":{}}"#,
            r#"{"jsonrpc":"2.0","id":7}"#,
            r#"{"jsonrpc":"2.0","id":7,"result":{},"error":{"code":1,"message":"x"}}"#,
        ] {
            assert!(decode_response(body.as_bytes(), 7).is_err());
        }
    }
}
