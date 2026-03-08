//! Telegram channel — uses teloxide for the Telegram Bot API.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use skyclaw_core::types::config::ChannelConfig;
use skyclaw_core::types::error::SkyclawError;
use skyclaw_core::types::file::{FileData, FileMetadata, OutboundFile, ReceivedFile};
use skyclaw_core::types::message::{AttachmentRef, InboundMessage, OutboundMessage, ParseMode};
use skyclaw_core::{Channel, FileTransfer};

use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{InputFile, MediaKind, MessageKind};

/// Maximum file size the Telegram Bot API supports for uploads (50 MB).
const TELEGRAM_UPLOAD_LIMIT: usize = 50 * 1024 * 1024;

// ── Persistent allowlist ──────────────────────────────────────────

/// On-disk representation of the allowlist stored at `~/.skyclaw/allowlist.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AllowlistFile {
    /// The admin user ID (the first user to ever message the bot).
    admin: String,
    /// All allowed user IDs (admin is always included).
    users: Vec<String>,
}

/// Return the path to `~/.skyclaw/allowlist.toml`.
fn allowlist_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".skyclaw").join("allowlist.toml"))
}

/// Load the persisted allowlist from disk.
/// Returns `None` if the file does not exist or cannot be parsed.
fn load_allowlist_file() -> Option<AllowlistFile> {
    let path = allowlist_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&content).ok()
}

/// Save the allowlist to disk. Creates `~/.skyclaw/` if needed.
fn save_allowlist_file(data: &AllowlistFile) -> Result<(), SkyclawError> {
    let path = allowlist_path().ok_or_else(|| {
        SkyclawError::Channel("Cannot determine home directory for allowlist".into())
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            SkyclawError::Channel(format!("Failed to create ~/.skyclaw directory: {e}"))
        })?;
    }
    let content = toml::to_string_pretty(data)
        .map_err(|e| SkyclawError::Channel(format!("Failed to serialize allowlist: {e}")))?;
    std::fs::write(&path, content)
        .map_err(|e| SkyclawError::Channel(format!("Failed to write allowlist file: {e}")))?;
    tracing::info!(path = %path.display(), "Allowlist saved");
    Ok(())
}

/// Telegram messaging channel.
pub struct TelegramChannel {
    /// The teloxide Bot handle.
    bot: Option<Bot>,
    /// Bot token.
    token: String,
    /// Allowlist of user IDs. Empty at startup = auto-whitelist first user.
    allowlist: Arc<RwLock<Vec<String>>>,
    /// Admin user ID (first user to message the bot). `None` until the first
    /// user is auto-whitelisted or loaded from the persisted allowlist file.
    admin: Arc<RwLock<Option<String>>>,
    /// Sender used to forward inbound messages to the gateway.
    tx: mpsc::Sender<InboundMessage>,
    /// Receiver the gateway drains. Taken once via `take_receiver()`.
    rx: Option<mpsc::Receiver<InboundMessage>>,
    /// Handle to the polling dispatcher task.
    dispatcher_handle: Option<tokio::task::JoinHandle<()>>,
    /// Shutdown signal for the reconnection loop.
    shutdown: Arc<AtomicBool>,
}

impl TelegramChannel {
    /// Create a new Telegram channel from a `ChannelConfig`.
    ///
    /// If a persisted allowlist exists at `~/.skyclaw/allowlist.toml`, it is
    /// loaded and merged with any entries from the config file. The admin is
    /// always the user recorded in the persisted file (or the first entry in
    /// the config allowlist, if no file exists yet).
    pub fn new(config: &ChannelConfig) -> Result<Self, SkyclawError> {
        let token = config
            .token
            .clone()
            .ok_or_else(|| SkyclawError::Config("Telegram channel requires a bot token".into()))?;

        let (tx, rx) = mpsc::channel(256);

        // Try to load persisted allowlist; fall back to config.
        let (allowlist, admin) = if let Some(file) = load_allowlist_file() {
            tracing::info!(
                admin = %file.admin,
                users = ?file.users,
                "Loaded persisted allowlist"
            );
            (file.users.clone(), Some(file.admin.clone()))
        } else if !config.allowlist.is_empty() {
            // Legacy: first entry in the config allowlist becomes admin.
            let admin = config.allowlist[0].clone();
            (config.allowlist.clone(), Some(admin))
        } else {
            (Vec::new(), None)
        };

        Ok(Self {
            bot: None,
            token,
            allowlist: Arc::new(RwLock::new(allowlist)),
            admin: Arc::new(RwLock::new(admin)),
            tx,
            rx: Some(rx),
            dispatcher_handle: None,
            shutdown: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Take the inbound message receiver. The gateway should call this once.
    pub fn take_receiver(&mut self) -> Option<mpsc::Receiver<InboundMessage>> {
        self.rx.take()
    }

    /// Check if a user (by numeric ID) is on the allowlist.
    ///
    /// Only numeric user IDs are matched. Usernames are ignored because
    /// they can be changed, enabling allowlist bypass (CA-04).
    /// An empty allowlist means no one is whitelisted yet (auto-whitelist
    /// happens in `handle_telegram_message` when the first user writes).
    fn check_allowed(&self, user_id: &str, _username: Option<&str>) -> bool {
        let list = self.allowlist.read().unwrap();
        if list.is_empty() {
            return false; // No one whitelisted yet
        }
        list.iter().any(|a| a == user_id)
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    async fn start(&mut self) -> Result<(), SkyclawError> {
        let bot = Bot::new(&self.token);

        // Validate token by calling getMe before starting the dispatcher.
        // teloxide panics on invalid tokens during dispatch — catch it here.
        bot.get_me().await.map_err(|e| {
            SkyclawError::Channel(format!("Invalid Telegram bot token — getMe failed: {e}"))
        })?;

        self.bot = Some(bot.clone());

        let tx = self.tx.clone();
        let allowlist = self.allowlist.clone();
        let admin = self.admin.clone();
        let shutdown = self.shutdown.clone();

        let handle = tokio::spawn(async move {
            let mut backoff = std::time::Duration::from_secs(1);

            loop {
                if shutdown.load(Ordering::Relaxed) {
                    tracing::info!("Telegram dispatcher shutdown requested");
                    break;
                }

                // Rebuild handler each iteration (dispatcher takes ownership)
                let tx = tx.clone();
                let allowlist = allowlist.clone();
                let admin = admin.clone();
                let handler = Update::filter_message().endpoint(
                    move |bot: Bot, msg: teloxide::types::Message| {
                        let tx = tx.clone();
                        let allowlist = allowlist.clone();
                        let admin = admin.clone();
                        async move {
                            if let Err(e) =
                                handle_telegram_message(&bot, msg, &tx, allowlist, admin).await
                            {
                                tracing::error!(error = %e, "Failed to handle Telegram message");
                            }
                            respond(())
                        }
                    },
                );

                let mut dispatcher = Dispatcher::builder(bot.clone(), handler).build();

                dispatcher.dispatch().await;

                // Dispatcher exited — network error, API throttle, etc.
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                tracing::warn!(
                    backoff_secs = backoff.as_secs(),
                    "Telegram dispatcher exited unexpectedly, reconnecting"
                );
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(std::time::Duration::from_secs(60));
            }
        });

        self.dispatcher_handle = Some(handle);
        tracing::info!("Telegram channel started");
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), SkyclawError> {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.dispatcher_handle.take() {
            // Give the dispatcher a moment to notice the shutdown
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        }
        tracing::info!("Telegram channel stopped");
        Ok(())
    }

    async fn send_message(&self, msg: OutboundMessage) -> Result<(), SkyclawError> {
        let bot = self
            .bot
            .as_ref()
            .ok_or_else(|| SkyclawError::Channel("Telegram bot not started".into()))?;

        let chat_id: ChatId = msg
            .chat_id
            .parse::<i64>()
            .map(ChatId)
            .map_err(|_| SkyclawError::Channel(format!("Invalid chat_id: {}", msg.chat_id)))?;

        let mut request = bot.send_message(chat_id, &msg.text);

        if let Some(ref mode) = msg.parse_mode {
            request = match mode {
                ParseMode::Markdown => request.parse_mode(teloxide::types::ParseMode::MarkdownV2),
                ParseMode::Html => request.parse_mode(teloxide::types::ParseMode::Html),
                ParseMode::Plain => request,
            };
        }

        request
            .await
            .map_err(|e| SkyclawError::Channel(format!("Failed to send Telegram message: {e}")))?;

        Ok(())
    }

    fn file_transfer(&self) -> Option<&dyn FileTransfer> {
        Some(self)
    }

    fn is_allowed(&self, user_id: &str) -> bool {
        self.check_allowed(user_id, None)
    }
}

#[async_trait]
impl FileTransfer for TelegramChannel {
    async fn receive_file(&self, msg: &InboundMessage) -> Result<Vec<ReceivedFile>, SkyclawError> {
        let bot = self
            .bot
            .as_ref()
            .ok_or_else(|| SkyclawError::Channel("Telegram bot not started".into()))?;

        let mut files = Vec::new();

        for att in &msg.attachments {
            let file_id = teloxide::types::FileId(att.file_id.clone());
            let tg_file = bot
                .get_file(file_id)
                .await
                .map_err(|e| SkyclawError::FileTransfer(format!("Failed to get file info: {e}")))?;

            // Use teloxide's built-in download to avoid exposing the bot
            // token in a manually-constructed URL (CA-03).
            let mut buf = Vec::new();
            bot.download_file(&tg_file.path, &mut buf)
                .await
                .map_err(|e| SkyclawError::FileTransfer(format!("Failed to download file: {e}")))?;

            let data = bytes::Bytes::from(buf);

            let name = att
                .file_name
                .clone()
                .unwrap_or_else(|| format!("file_{}", att.file_id));

            files.push(ReceivedFile {
                name,
                mime_type: att
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "application/octet-stream".to_string()),
                size: data.len(),
                data,
            });
        }

        Ok(files)
    }

    async fn send_file(&self, chat_id: &str, file: OutboundFile) -> Result<(), SkyclawError> {
        let bot = self
            .bot
            .as_ref()
            .ok_or_else(|| SkyclawError::Channel("Telegram bot not started".into()))?;

        let tg_chat_id: ChatId = chat_id
            .parse::<i64>()
            .map(ChatId)
            .map_err(|_| SkyclawError::Channel(format!("Invalid chat_id: {chat_id}")))?;

        let input_file = match &file.data {
            FileData::Bytes(b) => InputFile::memory(b.to_vec()).file_name(file.name.clone()),
            FileData::Url(url) => {
                let parsed = url
                    .parse::<url::Url>()
                    .map_err(|e| SkyclawError::FileTransfer(format!("Invalid file URL: {e}")))?;
                InputFile::url(parsed).file_name(file.name.clone())
            }
        };

        let mut request = bot.send_document(tg_chat_id, input_file);
        if let Some(ref caption) = file.caption {
            request = request.caption(caption);
        }

        request
            .await
            .map_err(|e| SkyclawError::FileTransfer(format!("Failed to send document: {e}")))?;

        Ok(())
    }

    async fn send_file_stream(
        &self,
        _chat_id: &str,
        _stream: BoxStream<'_, Bytes>,
        metadata: FileMetadata,
    ) -> Result<(), SkyclawError> {
        // Telegram does not support streaming uploads. We could buffer the
        // entire stream, but for now we return an error — callers should use
        // `send_file` with the fully-buffered data instead.
        Err(SkyclawError::FileTransfer(format!(
            "Telegram does not support streaming file uploads. \
                 Buffer the file ({}) and use send_file() instead.",
            metadata.name
        )))
    }

    fn max_file_size(&self) -> usize {
        TELEGRAM_UPLOAD_LIMIT
    }
}

/// Send a plain-text reply to the originating chat via the bot.
async fn bot_reply(bot: &Bot, chat_id: ChatId, text: &str) -> Result<(), SkyclawError> {
    bot.send_message(chat_id, text)
        .await
        .map_err(|e| SkyclawError::Channel(format!("Failed to send admin-command reply: {e}")))?;
    Ok(())
}

/// Persist the current in-memory allowlist + admin to disk.
fn persist_allowlist(
    allowlist: &Arc<RwLock<Vec<String>>>,
    admin: &Arc<RwLock<Option<String>>>,
) -> Result<(), SkyclawError> {
    let list = allowlist.read().unwrap().clone();
    let admin_id = admin.read().unwrap().clone().unwrap_or_default();
    save_allowlist_file(&AllowlistFile {
        admin: admin_id,
        users: list,
    })
}

/// Convert and forward a teloxide Message to the gateway via the mpsc sender.
///
/// Admin commands (`/allow`, `/revoke`, `/users`) are intercepted here and
/// handled directly — they never reach the agent.
async fn handle_telegram_message(
    bot: &Bot,
    msg: teloxide::types::Message,
    tx: &mpsc::Sender<InboundMessage>,
    allowlist: Arc<RwLock<Vec<String>>>,
    admin: Arc<RwLock<Option<String>>>,
) -> Result<(), SkyclawError> {
    let user = msg.from.as_ref();

    let user_id = user
        .map(|u| u.id.0.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let username = user.and_then(|u| u.username.clone());

    let chat_id = msg.chat.id;

    // ── Auto-whitelist first user & set as admin ──────────────────
    {
        let mut list = allowlist.write().unwrap();
        if list.is_empty() {
            list.push(user_id.clone());
            let mut adm = admin.write().unwrap();
            *adm = Some(user_id.clone());
            tracing::info!(user_id = %user_id, username = ?username, "Auto-whitelisted first user as admin");
            // Persist immediately so the admin survives a restart.
            drop(list);
            drop(adm);
            if let Err(e) = persist_allowlist(&allowlist, &admin) {
                tracing::error!(error = %e, "Failed to persist allowlist after auto-whitelist");
            }
        }
    }

    // ── Reject non-allowlisted users ─────────────────────────────
    {
        let list = allowlist.read().unwrap();
        if !list.iter().any(|a| a == &user_id) {
            drop(list);
            tracing::warn!(user_id = %user_id, username = ?username, "Rejected message from non-allowlisted user");
            return Ok(());
        }
    }

    // ── Intercept admin commands ─────────────────────────────────
    if let Some(text) = msg.text() {
        let trimmed = text.trim();

        if trimmed.starts_with("/allow ") || trimmed.starts_with("/revoke ") || trimmed == "/users"
        {
            let is_admin = {
                let adm = admin.read().unwrap();
                adm.as_deref() == Some(&user_id)
            };

            if !is_admin {
                bot_reply(bot, chat_id, "Only the admin can use this command.").await?;
                return Ok(());
            }

            // /users — list all allowed user IDs
            if trimmed == "/users" {
                let reply_text = {
                    let list = allowlist.read().unwrap();
                    let admin_id = admin.read().unwrap().clone().unwrap_or_default();
                    if list.is_empty() {
                        "Allowlist is empty.".to_string()
                    } else {
                        let mut lines = Vec::with_capacity(list.len());
                        for uid in list.iter() {
                            if uid == &admin_id {
                                lines.push(format!("{} (admin)", uid));
                            } else {
                                lines.push(uid.clone());
                            }
                        }
                        format!("Allowed users:\n{}", lines.join("\n"))
                    }
                };
                bot_reply(bot, chat_id, &reply_text).await?;
                return Ok(());
            }

            // /allow <user_id>
            if let Some(target) = trimmed.strip_prefix("/allow ") {
                let target = target.trim().to_string();
                if target.is_empty() {
                    bot_reply(bot, chat_id, "Usage: /allow <user_id>").await?;
                    return Ok(());
                }
                let already_exists = {
                    let mut list = allowlist.write().unwrap();
                    if list.iter().any(|a| a == &target) {
                        true
                    } else {
                        list.push(target.clone());
                        false
                    }
                };
                if already_exists {
                    bot_reply(
                        bot,
                        chat_id,
                        &format!("User {} is already allowed.", target),
                    )
                    .await?;
                    return Ok(());
                }
                if let Err(e) = persist_allowlist(&allowlist, &admin) {
                    tracing::error!(error = %e, "Failed to persist allowlist after /allow");
                    bot_reply(
                        bot,
                        chat_id,
                        &format!("User {} added (but failed to save to disk: {}).", target, e),
                    )
                    .await?;
                } else {
                    bot_reply(
                        bot,
                        chat_id,
                        &format!("User {} added to the allowlist.", target),
                    )
                    .await?;
                }
                tracing::info!(target = %target, "Admin added user to allowlist");
                return Ok(());
            }

            // /revoke <user_id>
            if let Some(target) = trimmed.strip_prefix("/revoke ") {
                let target = target.trim().to_string();
                if target.is_empty() {
                    bot_reply(bot, chat_id, "Usage: /revoke <user_id>").await?;
                    return Ok(());
                }
                // Cannot revoke self (the admin).
                if target == user_id {
                    bot_reply(bot, chat_id, "You cannot revoke yourself.").await?;
                    return Ok(());
                }
                let was_present = {
                    let mut list = allowlist.write().unwrap();
                    let before = list.len();
                    list.retain(|a| a != &target);
                    list.len() < before
                };
                if !was_present {
                    bot_reply(
                        bot,
                        chat_id,
                        &format!("User {} is not on the allowlist.", target),
                    )
                    .await?;
                    return Ok(());
                }
                if let Err(e) = persist_allowlist(&allowlist, &admin) {
                    tracing::error!(error = %e, "Failed to persist allowlist after /revoke");
                    bot_reply(
                        bot,
                        chat_id,
                        &format!(
                            "User {} revoked (but failed to save to disk: {}).",
                            target, e
                        ),
                    )
                    .await?;
                } else {
                    bot_reply(
                        bot,
                        chat_id,
                        &format!("User {} removed from the allowlist.", target),
                    )
                    .await?;
                }
                tracing::info!(target = %target, "Admin revoked user from allowlist");
                return Ok(());
            }
        }
    }

    // ── Forward to gateway as usual ──────────────────────────────
    let chat_id_str = chat_id.0.to_string();

    let text = msg.text().map(|t| t.to_string());
    let attachments = extract_attachments(&msg);
    let reply_to = msg.reply_to_message().map(|r| r.id.0.to_string());

    let inbound = InboundMessage {
        id: msg.id.0.to_string(),
        channel: "telegram".to_string(),
        chat_id: chat_id_str,
        user_id,
        username,
        text,
        attachments,
        reply_to,
        timestamp: msg.date,
    };

    tx.send(inbound)
        .await
        .map_err(|_| SkyclawError::Channel("Inbound message receiver dropped".into()))?;

    Ok(())
}

/// Extract attachment references from a Telegram message (documents, photos, etc.).
fn extract_attachments(msg: &teloxide::types::Message) -> Vec<AttachmentRef> {
    let mut attachments = Vec::new();

    if let MessageKind::Common(common) = &msg.kind {
        match &common.media_kind {
            MediaKind::Document(doc) => {
                let d = &doc.document;
                attachments.push(AttachmentRef {
                    file_id: d.file.id.0.clone(),
                    file_name: d.file_name.clone(),
                    mime_type: d.mime_type.as_ref().map(|m| m.to_string()),
                    size: Some(d.file.size as usize),
                });
            }
            MediaKind::Photo(photo) => {
                // Use the largest photo size
                if let Some(largest) = photo.photo.iter().max_by_key(|p| p.width * p.height) {
                    attachments.push(AttachmentRef {
                        file_id: largest.file.id.0.clone(),
                        file_name: Some("photo.jpg".to_string()),
                        mime_type: Some("image/jpeg".to_string()),
                        size: Some(largest.file.size as usize),
                    });
                }
            }
            MediaKind::Audio(audio) => {
                let a = &audio.audio;
                attachments.push(AttachmentRef {
                    file_id: a.file.id.0.clone(),
                    file_name: a.file_name.clone(),
                    mime_type: a.mime_type.as_ref().map(|m| m.to_string()),
                    size: Some(a.file.size as usize),
                });
            }
            MediaKind::Voice(voice) => {
                let v = &voice.voice;
                attachments.push(AttachmentRef {
                    file_id: v.file.id.0.clone(),
                    file_name: Some("voice.ogg".to_string()),
                    mime_type: v.mime_type.as_ref().map(|m| m.to_string()),
                    size: Some(v.file.size as usize),
                });
            }
            MediaKind::Video(video) => {
                let v = &video.video;
                attachments.push(AttachmentRef {
                    file_id: v.file.id.0.clone(),
                    file_name: v.file_name.clone(),
                    mime_type: v.mime_type.as_ref().map(|m| m.to_string()),
                    size: Some(v.file.size as usize),
                });
            }
            MediaKind::VideoNote(vn) => {
                let v = &vn.video_note;
                attachments.push(AttachmentRef {
                    file_id: v.file.id.0.clone(),
                    file_name: Some("video_note.mp4".to_string()),
                    mime_type: Some("video/mp4".to_string()),
                    size: Some(v.file.size as usize),
                });
            }
            _ => {}
        }
    }

    attachments
}
