//! CLI channel — interactive REPL over stdin/stdout.
//!
//! ## Multi-line input
//!
//! Each newline on stdin is normally treated as one message boundary —
//! suitable for interactive typing and single-line piped automation. To
//! send a multi-line message (pasting code, structured prompts), wrap the
//! content with `/paste` and `/send`:
//!
//! ```text
//! /paste
//! line 1
//! line 2
//!     indented line 3
//! /send
//! ```
//!
//! Indentation and blank lines inside the paste buffer are preserved.
//! If stdin reaches EOF while a paste buffer is open, it is flushed as
//! one message automatically (useful for piped scripts that omit /send).

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use temm1e_core::types::error::Temm1eError;
use temm1e_core::types::file::{FileData, FileMetadata, OutboundFile, ReceivedFile};
use temm1e_core::types::message::{AttachmentRef, InboundMessage, OutboundMessage};
use temm1e_core::{Channel, FileTransfer};

/// A channel that reads from stdin and writes to stdout, for local CLI usage.
pub struct CliChannel {
    /// Sender used by the stdin reader task to forward messages to the gateway.
    tx: Option<mpsc::Sender<InboundMessage>>,
    /// Receiver the gateway can drain to get inbound messages.
    rx: Option<mpsc::Receiver<InboundMessage>>,
    /// Handle to the background stdin reader task.
    reader_handle: Option<tokio::task::JoinHandle<()>>,
    /// Workspace directory for file operations.
    workspace: PathBuf,
}

impl CliChannel {
    /// Create a new CLI channel.
    ///
    /// `workspace` is the directory where received files are saved and from
    /// which files are read for sending.
    pub fn new(workspace: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel(64);
        Self {
            tx: Some(tx),
            rx: Some(rx),
            reader_handle: None,
            workspace,
        }
    }

    fn start_reader<R>(&mut self, reader: R) -> Result<(), Temm1eError>
    where
        R: tokio::io::AsyncBufRead + Unpin + Send + 'static,
    {
        let tx = self.tx.take().ok_or_else(|| {
            Temm1eError::Channel(
                "CLI reader already started; create a new channel to restart".into(),
            )
        })?;

        let handle = tokio::spawn(async move {
            let mut lines = reader.lines();
            let mut state = CliInputState::default();

            // Print a prompt before reading.
            eprint!("temm1e> ");

            loop {
                match lines.next_line().await {
                    Ok(Some(raw_line)) => {
                        let cmd = state.handle_line(raw_line);
                        match cmd {
                            CliCommand::Continue => {
                                // In paste mode, silently accumulate; do NOT
                                // reprint the prompt (would be ugly mid-paste).
                                if !state.paste_mode {
                                    eprint!("temm1e> ");
                                }
                            }
                            CliCommand::Info(msg) => {
                                eprintln!("  [{msg}]");
                                if !state.paste_mode {
                                    eprint!("temm1e> ");
                                }
                            }
                            CliCommand::FileNotFound(path) => {
                                eprintln!("  [file not found: {}]", path.display());
                                eprint!("temm1e> ");
                            }
                            CliCommand::SubmitText(text) => {
                                if !send_to_gateway(&tx, text, vec![]).await {
                                    break;
                                }
                                // Prompt redrawn when the agent's reply arrives
                                // (see `Channel::send_message`).
                            }
                            CliCommand::SubmitFile(path) => {
                                let att = AttachmentRef {
                                    file_id: path.to_string_lossy().to_string(),
                                    file_name: path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string()),
                                    mime_type: None,
                                    size: tokio::fs::metadata(&path)
                                        .await
                                        .ok()
                                        .map(|m| m.len() as usize),
                                };
                                let text = format!("[file: {}]", path.display());
                                if !send_to_gateway(&tx, text, vec![att]).await {
                                    break;
                                }
                            }
                            CliCommand::Exit => {
                                tracing::info!("CLI session ended by user");
                                break;
                            }
                        }
                    }
                    Ok(None) => {
                        // EOF — if a paste buffer is open, flush it as a
                        // final message before exiting.
                        if let Some(text) = state.flush_on_eof() {
                            let _ = send_to_gateway(&tx, text, vec![]).await;
                        }
                        tracing::info!("stdin closed");
                        break;
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Error reading stdin");
                        break;
                    }
                }
            }
        });

        self.reader_handle = Some(handle);
        tracing::info!("CLI channel started");
        Ok(())
    }

    /// Take the inbound message receiver. The gateway should call this once
    /// before calling `start()` to wire up the message pipeline.
    pub fn take_receiver(&mut self) -> Option<mpsc::Receiver<InboundMessage>> {
        self.rx.take()
    }
}

/// Outcome of processing one line of CLI input through [`CliInputState`].
///
/// Pure — no side effects. The caller decides what to print and whether
/// to dispatch the message. This separation makes the input parser
/// fully unit-testable without touching stdin/stdout.
#[derive(Debug, PartialEq)]
enum CliCommand {
    /// Line consumed; nothing to display or dispatch.
    Continue,
    /// Submit this text as a plain user message.
    SubmitText(String),
    /// Submit a message attaching the file at this path.
    SubmitFile(PathBuf),
    /// `/file <path>` was used but the path doesn't exist.
    FileNotFound(PathBuf),
    /// Informational note to display (e.g. paste-mode confirmations).
    Info(String),
    /// User asked to leave (`/quit` or `/exit`).
    Exit,
}

/// State machine for CLI input parsing.
///
/// Default state: line-by-line — every non-empty line becomes one message,
/// matching the historical CLI behavior.
///
/// `/paste` enters paste mode, where subsequent lines are accumulated
/// verbatim (indentation preserved, blank lines preserved) until `/send`
/// or `/quit` or EOF. `/send` submits the buffer as a single multi-line
/// message; `/quit` discards the buffer and exits; EOF flushes the buffer
/// as a final message and exits.
#[derive(Default, Debug)]
struct CliInputState {
    paste_mode: bool,
    paste_buf: Vec<String>,
}

impl CliInputState {
    /// Process one line of input. Mutates state, returns the action the
    /// caller should take.
    fn handle_line(&mut self, raw_line: String) -> CliCommand {
        let trimmed = raw_line.trim();

        // `/paste` — enter multi-line buffering mode.
        if trimmed == "/paste" {
            if self.paste_mode {
                return CliCommand::Info("already in paste mode; type /send to submit".to_string());
            }
            self.paste_mode = true;
            self.paste_buf.clear();
            return CliCommand::Info(
                "paste mode — type /send on its own line to submit".to_string(),
            );
        }

        // `/send` — flush the paste buffer as one message.
        if trimmed == "/send" {
            if !self.paste_mode {
                return CliCommand::Info(
                    "not in paste mode; type /paste first to begin".to_string(),
                );
            }
            self.paste_mode = false;
            if self.paste_buf.is_empty() {
                return CliCommand::Info("empty paste buffer; nothing to send".to_string());
            }
            let text = std::mem::take(&mut self.paste_buf).join("\n");
            return CliCommand::SubmitText(text);
        }

        // While in paste mode: accumulate the raw line verbatim
        // (indentation preserved). `/quit` and `/exit` still break out.
        if self.paste_mode {
            if trimmed == "/quit" || trimmed == "/exit" {
                self.paste_mode = false;
                self.paste_buf.clear();
                return CliCommand::Exit;
            }
            self.paste_buf.push(raw_line);
            return CliCommand::Continue;
        }

        // Normal (line-by-line) mode below.
        if trimmed.is_empty() {
            return CliCommand::Continue;
        }
        if trimmed == "/quit" || trimmed == "/exit" {
            return CliCommand::Exit;
        }
        if let Some(path_str) = trimmed.strip_prefix("/file ") {
            let path = PathBuf::from(path_str.trim());
            if path.exists() {
                return CliCommand::SubmitFile(path);
            }
            return CliCommand::FileNotFound(path);
        }
        CliCommand::SubmitText(trimmed.to_string())
    }

    /// Called when stdin closes. If a paste buffer is pending, returns
    /// the accumulated text as the final message to send before exit.
    /// Always resets paste state — at EOF we're tearing down regardless.
    fn flush_on_eof(&mut self) -> Option<String> {
        let was_paste = self.paste_mode;
        self.paste_mode = false;
        if was_paste && !self.paste_buf.is_empty() {
            Some(std::mem::take(&mut self.paste_buf).join("\n"))
        } else {
            self.paste_buf.clear();
            None
        }
    }
}

/// Construct and dispatch one `InboundMessage` to the gateway.
///
/// Returns `true` on success, `false` if the receiver is gone (caller
/// should stop the stdin reader).
async fn send_to_gateway(
    tx: &mpsc::Sender<InboundMessage>,
    text: String,
    attachments: Vec<AttachmentRef>,
) -> bool {
    let msg = InboundMessage {
        id: uuid::Uuid::new_v4().to_string(),
        channel: "cli".to_string(),
        chat_id: "cli".to_string(),
        user_id: "local".to_string(),
        username: Some(whoami()),
        text: Some(text),
        attachments,
        reply_to: None,
        timestamp: chrono::Utc::now(),
    };
    if tx.send(msg).await.is_err() {
        tracing::warn!("CLI channel receiver dropped, stopping stdin reader");
        return false;
    }
    true
}

#[async_trait]
impl Channel for CliChannel {
    fn name(&self) -> &str {
        "cli"
    }

    async fn start(&mut self) -> Result<(), Temm1eError> {
        self.start_reader(BufReader::new(tokio::io::stdin()))
    }

    async fn stop(&mut self) -> Result<(), Temm1eError> {
        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
        }
        tracing::info!("CLI channel stopped");
        Ok(())
    }

    async fn send_message(&self, msg: OutboundMessage) -> Result<(), Temm1eError> {
        // Print the response to stdout with a visual separator
        println!();
        println!("{}", msg.text);
        println!();
        eprint!("temm1e> ");
        Ok(())
    }

    fn file_transfer(&self) -> Option<&dyn FileTransfer> {
        Some(self)
    }

    fn is_allowed(&self, _user_id: &str) -> bool {
        // CLI is always local; no access control needed.
        true
    }
}

#[async_trait]
impl FileTransfer for CliChannel {
    async fn receive_file(&self, msg: &InboundMessage) -> Result<Vec<ReceivedFile>, Temm1eError> {
        let mut files = Vec::new();
        for att in &msg.attachments {
            // The file_id for CLI is the local file path
            let path = std::path::Path::new(&att.file_id);
            let data = tokio::fs::read(path).await.map_err(|e| {
                Temm1eError::FileTransfer(format!("Failed to read {}: {e}", path.display()))
            })?;
            let size = data.len();
            files.push(ReceivedFile {
                name: att.file_name.clone().unwrap_or_else(|| "file".to_string()),
                mime_type: att
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "application/octet-stream".to_string()),
                size,
                data: Bytes::from(data),
            });
        }
        Ok(files)
    }

    async fn send_file(&self, _chat_id: &str, file: OutboundFile) -> Result<(), Temm1eError> {
        let dest = self.workspace.join(&file.name);
        let data = match &file.data {
            FileData::Bytes(b) => b.clone(),
            FileData::Url(url) => {
                return Err(Temm1eError::FileTransfer(format!(
                    "CLI channel does not support URL file sending: {url}"
                )));
            }
        };
        tokio::fs::create_dir_all(&self.workspace)
            .await
            .map_err(|e| Temm1eError::FileTransfer(format!("Failed to create workspace: {e}")))?;
        tokio::fs::write(&dest, &data)
            .await
            .map_err(|e| Temm1eError::FileTransfer(format!("Failed to write file: {e}")))?;

        if let Some(caption) = &file.caption {
            println!("  [file saved: {} — {}]", dest.display(), caption);
        } else {
            println!("  [file saved: {}]", dest.display());
        }
        Ok(())
    }

    async fn send_file_stream(
        &self,
        _chat_id: &str,
        _stream: BoxStream<'_, Bytes>,
        _metadata: FileMetadata,
    ) -> Result<(), Temm1eError> {
        Err(Temm1eError::FileTransfer(
            "CLI channel does not support streaming file transfers".to_string(),
        ))
    }

    fn max_file_size(&self) -> usize {
        // 100 MB — local files, practically unlimited
        100 * 1024 * 1024
    }
}

/// Get the current OS username, best-effort.
fn whoami() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reader_exit_closes_gateway_queue_even_while_channel_is_retained() {
        for input in ["hello\n/quit\n", "hello\n/exit\n", "hello\n"] {
            let mut channel = CliChannel::new(PathBuf::from("."));
            let mut rx = channel.take_receiver().unwrap();
            channel
                .start_reader(BufReader::new(std::io::Cursor::new(
                    input.as_bytes().to_vec(),
                )))
                .unwrap();
            let message = rx.recv().await.unwrap();
            assert_eq!(message.text.as_deref(), Some("hello"));
            assert!(
                tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                channel.reader_handle.is_some(),
                "channel still retained by the gateway"
            );
            channel.stop().await.unwrap();
        }
    }

    #[test]
    fn single_line_unchanged() {
        let mut state = CliInputState::default();
        let cmd = state.handle_line("hello world".to_string());
        assert_eq!(cmd, CliCommand::SubmitText("hello world".to_string()));
        assert!(!state.paste_mode);
        assert!(state.paste_buf.is_empty());
    }

    #[test]
    fn empty_line_normal_mode_is_continue() {
        let mut state = CliInputState::default();
        let cmd = state.handle_line(String::new());
        assert_eq!(cmd, CliCommand::Continue);
        assert!(!state.paste_mode);
    }

    #[test]
    fn quit_normal_mode_returns_exit() {
        let mut state = CliInputState::default();
        let cmd = state.handle_line("/quit".to_string());
        assert_eq!(cmd, CliCommand::Exit);
        let cmd = state.handle_line("/exit".to_string());
        assert_eq!(cmd, CliCommand::Exit);
    }

    #[test]
    fn paste_then_send_submits_joined_text() {
        let mut state = CliInputState::default();
        let cmd = state.handle_line("/paste".to_string());
        assert!(matches!(cmd, CliCommand::Info(_)));
        assert!(state.paste_mode);

        assert_eq!(state.handle_line("line1".to_string()), CliCommand::Continue);
        assert_eq!(state.handle_line("line2".to_string()), CliCommand::Continue);

        let cmd = state.handle_line("/send".to_string());
        assert_eq!(cmd, CliCommand::SubmitText("line1\nline2".to_string()));
        assert!(!state.paste_mode);
        assert!(state.paste_buf.is_empty());
    }

    #[test]
    fn paste_preserves_indentation_and_blank_lines() {
        let mut state = CliInputState::default();
        state.handle_line("/paste".to_string());
        state.handle_line("    indented".to_string());
        state.handle_line(String::new()); // blank line inside paste — preserved
        state.handle_line("  also indented".to_string());
        let cmd = state.handle_line("/send".to_string());
        assert_eq!(
            cmd,
            CliCommand::SubmitText("    indented\n\n  also indented".to_string())
        );
    }

    #[test]
    fn paste_eof_flushes_pending_buffer() {
        let mut state = CliInputState::default();
        state.handle_line("/paste".to_string());
        state.handle_line("line1".to_string());
        state.handle_line("line2".to_string());
        let flushed = state.flush_on_eof();
        assert_eq!(flushed, Some("line1\nline2".to_string()));
        assert!(!state.paste_mode);
        assert!(state.paste_buf.is_empty());
    }

    #[test]
    fn flush_on_eof_no_paste_returns_none() {
        let mut state = CliInputState::default();
        state.handle_line("normal line".to_string());
        assert_eq!(state.flush_on_eof(), None);
    }

    #[test]
    fn flush_on_eof_empty_paste_returns_none() {
        let mut state = CliInputState::default();
        state.handle_line("/paste".to_string());
        // No content added.
        assert_eq!(state.flush_on_eof(), None);
        // Paste mode is also reset.
        assert!(!state.paste_mode);
    }

    #[test]
    fn send_without_paste_returns_info() {
        let mut state = CliInputState::default();
        let cmd = state.handle_line("/send".to_string());
        assert!(matches!(cmd, CliCommand::Info(_)));
        assert!(!state.paste_mode);
    }

    #[test]
    fn quit_in_paste_discards_buffer_and_exits() {
        let mut state = CliInputState::default();
        state.handle_line("/paste".to_string());
        state.handle_line("would be discarded".to_string());
        let cmd = state.handle_line("/quit".to_string());
        assert_eq!(cmd, CliCommand::Exit);
        assert!(!state.paste_mode);
        assert!(state.paste_buf.is_empty());
    }

    #[test]
    fn paste_inside_paste_is_warning_buffer_preserved() {
        let mut state = CliInputState::default();
        state.handle_line("/paste".to_string());
        state.handle_line("first".to_string());
        let cmd = state.handle_line("/paste".to_string());
        assert!(matches!(cmd, CliCommand::Info(_)));
        assert!(state.paste_mode);
        assert_eq!(state.paste_buf, vec!["first".to_string()]);
    }

    #[test]
    fn file_command_missing_path_returns_file_not_found() {
        let mut state = CliInputState::default();
        let cmd = state
            .handle_line("/file /tmp/__definitely_nonexistent_temm1e_test_path_zzz".to_string());
        match cmd {
            CliCommand::FileNotFound(p) => {
                assert!(p.to_string_lossy().contains("nonexistent"));
            }
            other => panic!("expected FileNotFound, got {other:?}"),
        }
    }
}
