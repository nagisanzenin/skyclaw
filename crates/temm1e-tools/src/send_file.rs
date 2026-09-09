//! Send file tool — sends a file from the workspace back to the user through
//! the messaging channel.

use crate::channel_target::ChannelTarget;
use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use temm1e_core::types::error::Temm1eError;
use temm1e_core::types::file::{FileData, OutboundFile};
use temm1e_core::{
    Channel, PathAccess, Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput,
};
use tokio::io::AsyncReadExt;

/// Conservative local upload ceiling (50 MiB), independent of platform quotas.
const MAX_SEND_SIZE: usize = 50 * 1024 * 1024;

pub struct SendFileTool {
    channel: ChannelTarget,
}

impl SendFileTool {
    pub fn new(channel: Arc<dyn Channel>) -> Self {
        Self {
            channel: ChannelTarget::Single(channel),
        }
    }

    /// Server tools resolve from the active session's transport, preserving
    /// the configured heartbeat destination as the only explicit fallback.
    pub fn routed(
        channels: HashMap<String, Arc<dyn Channel>>,
        heartbeat: Option<Arc<dyn Channel>>,
    ) -> Self {
        Self {
            channel: ChannelTarget::Routed {
                channels,
                heartbeat,
            },
        }
    }
}

#[async_trait]
impl Tool for SendFileTool {
    fn name(&self) -> &str {
        "send_file"
    }

    fn description(&self) -> &str {
        "Send a file from the workspace to the user through the chat. \
         Use this to deliver generated files, reports, images, or any file \
         the user requested. Paths are relative to the workspace directory."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to send (relative to workspace or absolute)"
                },
                "chat_id": {
                    "type": "string",
                    "description": "The chat ID to send the file to. Omit to send to the current conversation."
                },
                "caption": {
                    "type": "string",
                    "description": "Optional caption/message to include with the file"
                }
            },
            "required": ["path"]
        })
    }

    fn declarations(&self) -> ToolDeclarations {
        ToolDeclarations {
            file_access: vec![PathAccess::Read(".".into())],
            network_access: Vec::new(),
            shell_access: false,
        }
    }

    async fn execute(
        &self,
        input: ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let ft = self
            .channel
            .resolve(&ctx.channel)?
            .file_transfer()
            .ok_or_else(|| Temm1eError::Tool("Channel does not support file transfer".into()))?;
        let max_size = MAX_SEND_SIZE.min(ft.max_file_size());
        let path_str = input
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: path".into()))?;

        let chat_id = input
            .arguments
            .get("chat_id")
            .and_then(|v| v.as_str())
            .unwrap_or(&ctx.chat_id);

        let caption = input
            .arguments
            .get("caption")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let path = if std::path::Path::new(path_str).is_absolute() {
            std::path::PathBuf::from(path_str)
        } else {
            ctx.workspace_path.join(path_str)
        };

        // Bound bytes while reading, including a file that grows after open.
        let read_result = async {
            let file = tokio::fs::File::open(&path).await?;
            let mut data = Vec::new();
            file.take(max_size as u64 + 1)
                .read_to_end(&mut data)
                .await?;
            Ok::<_, std::io::Error>(data)
        }
        .await;
        let data = match read_result {
            Ok(data) => data,
            Err(error) => {
                return Ok(ToolOutput {
                    content: format!("Failed to read file '{}': {}", path_str, error),
                    is_error: true,
                })
            }
        };

        if data.len() > max_size {
            return Ok(ToolOutput {
                content: format!(
                    "File is too large ({} bytes, max {} bytes)",
                    data.len(),
                    max_size
                ),
                is_error: true,
            });
        }

        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());

        // Guess mime type from extension
        let mime_type = match path.extension().and_then(|e| e.to_str()) {
            Some("pdf") => "application/pdf",
            Some("txt") | Some("md") | Some("log") => "text/plain",
            Some("json") => "application/json",
            Some("csv") => "text/csv",
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("zip") => "application/zip",
            Some("tar") | Some("gz") => "application/gzip",
            Some("html") | Some("htm") => "text/html",
            Some("xml") => "application/xml",
            Some("py") => "text/x-python",
            Some("rs") => "text/x-rust",
            Some("js") | Some("ts") => "text/javascript",
            _ => "application/octet-stream",
        }
        .to_string();

        let outbound = OutboundFile {
            name: file_name.clone(),
            mime_type,
            data: FileData::Bytes(bytes::Bytes::from(data)),
            caption,
        };

        match ft.send_file(chat_id, outbound).await {
            Ok(()) => Ok(ToolOutput {
                content: format!("Sent file '{}' to chat", file_name),
                is_error: false,
            }),
            Err(e) => Ok(ToolOutput {
                content: format!("Failed to send file: {}", e),
                is_error: true,
            }),
        }
    }
}
