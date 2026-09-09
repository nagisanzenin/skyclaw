//! Shared utilities for WhatsApp channels (Cloud API + Web).

use serde::{Deserialize, Serialize};

/// On-disk representation of a WhatsApp allowlist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppAllowlistFile {
    /// The admin phone number (first user to message the bot).
    pub admin: String,
    /// All allowed phone numbers (admin is always included).
    pub users: Vec<String>,
}

/// Return the path to `~/.temm1e/{filename}`.
pub fn whatsapp_allowlist_path(filename: &str) -> Option<std::path::PathBuf> {
    Some(temm1e_core::config::data_dir().join(filename))
}

/// Load a persisted WhatsApp allowlist from disk.
pub fn load_whatsapp_allowlist(filename: &str) -> Option<WhatsAppAllowlistFile> {
    let path = whatsapp_allowlist_path(filename)?;
    let content = std::fs::read_to_string(&path).ok()?;
    match toml::from_str(&content) {
        Ok(parsed) => Some(parsed),
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "Failed to parse WhatsApp allowlist file, ignoring"
            );
            None
        }
    }
}

/// Save a WhatsApp allowlist to disk.
pub fn save_whatsapp_allowlist(
    filename: &str,
    data: &WhatsAppAllowlistFile,
) -> Result<(), temm1e_core::types::error::Temm1eError> {
    use temm1e_core::types::error::Temm1eError;

    let path = whatsapp_allowlist_path(filename).ok_or_else(|| {
        Temm1eError::Channel("Cannot determine home directory for WhatsApp allowlist".into())
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            Temm1eError::Channel(format!("Failed to create ~/.temm1e directory: {e}"))
        })?;
    }
    let content = toml::to_string_pretty(data)
        .map_err(|e| Temm1eError::Channel(format!("Failed to serialize allowlist: {e}")))?;
    std::fs::write(&path, content)
        .map_err(|e| Temm1eError::Channel(format!("Failed to write allowlist: {e}")))?;
    tracing::info!(path = %path.display(), "WhatsApp allowlist saved");
    Ok(())
}

/// Normalize a phone number to digits-only (strip everything except digits).
/// The `+` prefix is stripped too so `+15551234567` and `15551234567` match.
pub fn normalize_phone(phone: &str) -> String {
    phone.chars().filter(|c| c.is_ascii_digit()).collect()
}

/// UTF-8 safe string truncation. Never panics on multi-byte characters.
pub fn safe_truncate(text: &str, max_chars: usize) -> &str {
    if text.chars().count() <= max_chars {
        return text;
    }
    let boundary = text
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    &text[..boundary]
}

/// Sanitize a filename to prevent path traversal attacks.
pub fn sanitize_filename(name: &str) -> String {
    std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unnamed")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_non_digits() {
        assert_eq!(normalize_phone("+1 (555) 123-4567"), "15551234567");
        assert_eq!(normalize_phone("555.123.4567"), "5551234567");
        assert_eq!(normalize_phone("+84 987 654 321"), "84987654321");
    }

    #[test]
    fn safe_truncate_ascii() {
        assert_eq!(safe_truncate("hello world", 5), "hello");
        assert_eq!(safe_truncate("hi", 10), "hi");
    }

    #[test]
    fn safe_truncate_multibyte() {
        // Vietnamese text with multi-byte chars
        let text = "Xin chào thế giới";
        let truncated = safe_truncate(text, 8);
        assert_eq!(truncated, "Xin chào");
        // Emoji
        let emoji = "😀😁😂🤣😃";
        assert_eq!(safe_truncate(emoji, 3), "😀😁😂");
    }

    #[test]
    fn safe_truncate_empty() {
        assert_eq!(safe_truncate("", 10), "");
        assert_eq!(safe_truncate("", 0), "");
    }

    #[test]
    fn sanitize_filename_strips_path() {
        assert_eq!(sanitize_filename("../../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("/tmp/secret.txt"), "secret.txt");
        assert_eq!(sanitize_filename("normal.pdf"), "normal.pdf");
        assert_eq!(sanitize_filename(""), "unnamed");
    }
}
