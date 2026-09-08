//! MCP client — handles the MCP protocol lifecycle:
//! initialize handshake, tool discovery, tool invocation, ping.

use crate::transport::Transport;
use std::sync::Arc;
use temm1e_core::types::error::Temm1eError;
use tracing::{debug, info};

/// Information about a single tool exposed by an MCP server.
#[derive(Debug, Clone)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Information about the connected MCP server.
#[derive(Debug, Clone)]
pub struct McpServerInfo {
    pub name: String,
    pub version: String,
    pub protocol_version: String,
}

/// MCP client — wraps a transport and speaks the MCP protocol.
pub struct McpClient {
    transport: Arc<dyn Transport>,
    server_info: tokio::sync::RwLock<Option<McpServerInfo>>,
    server_name: String,
}

impl McpClient {
    pub fn new(server_name: &str, transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            server_info: tokio::sync::RwLock::new(None),
            server_name: server_name.to_string(),
        }
    }

    /// Perform the MCP initialize handshake.
    /// Must be called before any other operations.
    pub async fn initialize(&self) -> Result<McpServerInfo, Temm1eError> {
        let params = serde_json::json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {
                "name": "temm1e",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        debug!(server = %self.server_name, "MCP initialize handshake");

        let response = self.transport.send("initialize", Some(params)).await?;

        let result = response.into_result()?;

        // Explicit legacy compatibility set. New lifecycle revisions require
        // their own negotiation path; accepting an unknown version is unsafe.
        let version = result["protocolVersion"].as_str().unwrap_or("");
        if !["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"].contains(&version) {
            let _ = self.transport.close().await;
            return Err(Temm1eError::Tool(
                "MCP server returned an unsupported protocol version".into(),
            ));
        }
        let required = |field: &str| {
            result["serverInfo"][field]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| Temm1eError::Tool(format!("MCP serverInfo is missing {field}")))
        };
        let server_info = McpServerInfo {
            name: required("name")?,
            version: required("version")?,
            protocol_version: version.to_owned(),
        };
        self.transport.set_protocol_version(version).await?;
        self.transport
            .notify("notifications/initialized", None)
            .await?;
        *self.server_info.write().await = Some(server_info.clone());

        info!(
            server = %self.server_name,
            mcp_server_name = %server_info.name,
            mcp_server_version = %server_info.version,
            protocol = %server_info.protocol_version,
            "MCP server initialized"
        );

        Ok(server_info)
    }

    /// List tools exposed by the MCP server.
    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>, Temm1eError> {
        // Cursors are opaque and scoped to this discovery operation. Bound both
        // requests and registrations; never return a silently partial catalog.
        const MAX_PAGES: usize = 256;
        const MAX_TOOLS: usize = 10_000;
        let mut tools = Vec::new();
        let mut names = std::collections::HashSet::new();
        let mut cursors = std::collections::HashSet::new();
        let mut params = None;
        for page in 0..MAX_PAGES {
            let result = self
                .transport
                .send("tools/list", params)
                .await?
                .into_result()?;
            let entries = result["tools"]
                .as_array()
                .ok_or_else(|| Temm1eError::Tool("MCP tools/list: missing 'tools' array".into()))?;
            if entries.len() > MAX_TOOLS.saturating_sub(tools.len()) {
                return Err(Temm1eError::Tool(
                    "MCP tool catalog exceeds 10,000 tools".into(),
                ));
            }
            for entry in entries {
                let name = entry["name"]
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .ok_or_else(|| Temm1eError::Tool("MCP tool has no nonempty name".into()))?;
                if !names.insert(name.to_owned()) {
                    return Err(Temm1eError::Tool(
                        "MCP tool catalog contains duplicate names".into(),
                    ));
                }
                let input_schema = entry
                    .get("inputSchema")
                    .filter(|s| s.is_object())
                    .ok_or_else(|| {
                        Temm1eError::Tool("MCP tool has no object inputSchema".into())
                    })?;
                tools.push(McpToolInfo {
                    name: name.to_owned(),
                    description: entry["description"].as_str().unwrap_or("").to_owned(),
                    input_schema: input_schema.clone(),
                });
            }
            match result.get("nextCursor") {
                None => break,
                Some(serde_json::Value::String(cursor)) => {
                    if !cursors.insert(cursor.clone()) {
                        return Err(Temm1eError::Tool(
                            "MCP tool pagination repeated a cursor".into(),
                        ));
                    }
                    if page + 1 == MAX_PAGES {
                        return Err(Temm1eError::Tool(
                            "MCP tool discovery exceeds 256 pages".into(),
                        ));
                    }
                    params = Some(serde_json::json!({"cursor": cursor}));
                }
                Some(_) => return Err(Temm1eError::Tool("MCP nextCursor must be a string".into())),
            }
        }

        debug!(
            server = %self.server_name,
            tool_count = tools.len(),
            "Discovered MCP tools"
        );

        Ok(tools)
    }

    /// Call a tool on the MCP server.
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult, Temm1eError> {
        let params = serde_json::json!({
            "name": tool_name,
            "arguments": arguments
        });

        debug!(
            server = %self.server_name,
            tool = %tool_name,
            "Calling MCP tool"
        );

        let response = self.transport.send("tools/call", Some(params)).await?;
        let result = response.into_result()?;

        // Extract text content from the MCP tool result
        let content_array = result["content"].as_array();
        let mut text_parts: Vec<String> = Vec::new();

        if let Some(contents) = content_array {
            for part in contents {
                if let Some(text) = part["text"].as_str() {
                    text_parts.push(text.to_string());
                } else if let Some(data) = part.get("data") {
                    // Binary/image content — include as JSON
                    text_parts.push(serde_json::to_string(data).unwrap_or_default());
                }
            }
        } else if let Some(text) = result.as_str() {
            // Some servers return plain text
            text_parts.push(text.to_string());
        }

        let is_error = result["isError"].as_bool().unwrap_or(false);

        Ok(McpToolResult {
            content: text_parts.join("\n"),
            is_error,
        })
    }

    /// Ping the MCP server to check health.
    pub async fn ping(&self) -> Result<(), Temm1eError> {
        let response = self.transport.send("ping", None).await?;
        let _ = response.into_result()?;
        Ok(())
    }

    /// Check if the underlying transport is alive.
    pub fn is_alive(&self) -> bool {
        self.transport.is_alive()
    }

    /// Close the client and transport.
    pub async fn close(&self) -> Result<(), Temm1eError> {
        self.transport.close().await
    }

    /// Get server info (available after initialize).
    pub async fn server_info(&self) -> Option<McpServerInfo> {
        self.server_info.read().await.clone()
    }
}

/// Result from calling an MCP tool.
#[derive(Debug, Clone)]
pub struct McpToolResult {
    pub content: String,
    pub is_error: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixtureTransport {
        replies: tokio::sync::Mutex<std::collections::VecDeque<serde_json::Value>>,
        requests: tokio::sync::Mutex<Vec<(String, Option<serde_json::Value>)>>,
        closed: std::sync::atomic::AtomicBool,
        fail_notify: bool,
    }

    impl FixtureTransport {
        fn new(replies: Vec<serde_json::Value>, fail_notify: bool) -> Arc<Self> {
            Arc::new(Self {
                replies: tokio::sync::Mutex::new(replies.into()),
                requests: Default::default(),
                closed: false.into(),
                fail_notify,
            })
        }
    }

    #[async_trait::async_trait]
    impl Transport for FixtureTransport {
        async fn send(
            &self,
            method: &str,
            params: Option<serde_json::Value>,
        ) -> Result<crate::jsonrpc::JsonRpcResponse, Temm1eError> {
            self.requests.lock().await.push((method.into(), params));
            Ok(crate::jsonrpc::JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: Some(1),
                error: None,
                result: Some(
                    self.replies
                        .lock()
                        .await
                        .pop_front()
                        .expect("unexpected request"),
                ),
            })
        }
        async fn notify(&self, _: &str, _: Option<serde_json::Value>) -> Result<(), Temm1eError> {
            if self.fail_notify {
                Err(Temm1eError::Tool("fixture notification failed".into()))
            } else {
                Ok(())
            }
        }
        fn is_alive(&self) -> bool {
            !self.closed.load(std::sync::atomic::Ordering::Relaxed)
        }
        async fn close(&self) -> Result<(), Temm1eError> {
            self.closed
                .store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
    }

    fn page(name: &str, cursor: Option<&str>) -> serde_json::Value {
        let mut result =
            serde_json::json!({"tools": [{"name": name, "inputSchema": {"type": "object"}}]});
        if let Some(cursor) = cursor {
            result["nextCursor"] = cursor.into();
        }
        result
    }

    #[tokio::test]
    async fn discovery_follows_opaque_cursor_and_preserves_all_tools() {
        let transport =
            FixtureTransport::new(vec![page("a", Some("雪/+==")), page("b", None)], false);
        let tools = McpClient::new("fixture", transport.clone())
            .list_tools()
            .await
            .unwrap();
        assert_eq!(
            tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        let calls = transport.requests.lock().await;
        assert_eq!(calls[0].1, None);
        assert_eq!(calls[1].1, Some(serde_json::json!({"cursor": "雪/+=="})));
    }

    #[tokio::test]
    async fn discovery_rejects_cycles_duplicates_and_malformed_pages() {
        for replies in [
            vec![page("a", Some("same")), page("b", Some("same"))],
            vec![page("a", Some("next")), page("a", None)],
            vec![serde_json::json!({"tools": [], "nextCursor": 1})],
            vec![serde_json::json!({"tools": [{"inputSchema": {}}]})],
            vec![serde_json::json!({"tools": [{"name": "bad", "inputSchema": []}]})],
        ] {
            let client = McpClient::new("fixture", FixtureTransport::new(replies, false));
            assert!(client.list_tools().await.is_err());
        }
    }

    #[tokio::test]
    async fn handshake_rejects_unknown_version_and_never_publishes_failed_initialization() {
        let transport = FixtureTransport::new(
            vec![serde_json::json!({"protocolVersion": "unknown"})],
            false,
        );
        let client = McpClient::new("fixture", transport.clone());
        assert!(client.initialize().await.is_err());
        assert!(!transport.is_alive());
        assert!(client.server_info().await.is_none());
        let transport = FixtureTransport::new(
            vec![serde_json::json!({
                "protocolVersion": "2025-11-25", "serverInfo": {"name": "fixture", "version": "1"}
            })],
            true,
        );
        let client = McpClient::new("fixture", transport);
        assert!(client.initialize().await.is_err());
        assert!(client.server_info().await.is_none());
    }

    #[test]
    fn mcp_tool_info_debug() {
        let info = McpToolInfo {
            name: "test".to_string(),
            description: "A test tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        };
        assert_eq!(info.name, "test");
        assert_eq!(info.description, "A test tool");
    }

    #[test]
    fn mcp_server_info_debug() {
        let info = McpServerInfo {
            name: "test-server".to_string(),
            version: "1.0.0".to_string(),
            protocol_version: "2025-11-25".to_string(),
        };
        assert_eq!(info.name, "test-server");
    }

    #[test]
    fn mcp_tool_result_error() {
        let result = McpToolResult {
            content: "Something went wrong".to_string(),
            is_error: true,
        };
        assert!(result.is_error);
    }
}
