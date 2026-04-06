//! Role-Based Access Control — roles, role files, and shared I/O.
//!
//! Two roles:
//! - **Admin**: full access to all tools and slash commands
//! - **User**: full agent chat, most tools, blocked from dangerous tools and system commands
//!
//! `RoleFile` is the on-disk format, backward-compatible with the legacy `AllowlistFile`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::types::error::Temm1eError;

// ── Role enum ────────────────────────────────────────────────────────

/// User role within a channel.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    #[default]
    User,
}

impl Role {
    /// Tools blocked for this role (blacklist).
    /// Admin: nothing blocked. User: dangerous tools blocked.
    pub fn blocked_tools(&self) -> &'static [&'static str] {
        match self {
            Role::Admin => &[],
            Role::User => &[
                "shell",       // arbitrary command execution
                "key_manage",  // API credential management
                "invoke_core", // TemDOS cores (system access)
                "desktop",     // OS-level screen + input simulation
            ],
        }
    }

    /// Whether this role has unrestricted tool access.
    pub fn has_all_tools(&self) -> bool {
        matches!(self, Role::Admin)
    }

    /// Whether a specific tool is allowed for this role.
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        !self.blocked_tools().contains(&tool_name)
    }

    /// Slash commands this role can use (whitelist).
    /// Admin: empty = all commands allowed.
    pub fn allowed_commands(&self) -> &'static [&'static str] {
        match self {
            Role::Admin => &[],
            Role::User => &["/help", "/status", "/usage", "/mode", "/memory", "/quit"],
        }
    }

    /// Whether this role has unrestricted command access.
    pub fn has_all_commands(&self) -> bool {
        matches!(self, Role::Admin)
    }

    /// Whether a specific slash command is allowed for this role.
    pub fn is_command_allowed(&self, command: &str) -> bool {
        if self.has_all_commands() {
            return true;
        }
        // Extract the base command (e.g. "/mode play" -> "/mode")
        let base = command.split_whitespace().next().unwrap_or(command);
        self.allowed_commands().contains(&base)
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::Admin => write!(f, "admin"),
            Role::User => write!(f, "user"),
        }
    }
}

// ── RoleFile ─────────────────────────────────────────────────────────

/// On-disk access control list with role support.
/// Backward-compatible with legacy `AllowlistFile { admin, users }`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RoleFile {
    /// The original admin (first user). Kept for backward compatibility.
    #[serde(default)]
    pub admin: String,
    /// All admin user IDs.
    #[serde(default)]
    pub admins: Vec<String>,
    /// All allowed user IDs (admins + regular users).
    #[serde(default)]
    pub users: Vec<String>,
}

impl RoleFile {
    /// Create a new RoleFile with a single admin (first-user setup).
    pub fn new_with_admin(admin_id: &str) -> Self {
        Self {
            admin: admin_id.to_string(),
            admins: vec![admin_id.to_string()],
            users: vec![admin_id.to_string()],
        }
    }

    /// Migrate from legacy format: if `admins` is empty, seed from `admin`.
    pub fn migrate(&mut self) {
        if self.admins.is_empty() && !self.admin.is_empty() {
            self.admins.push(self.admin.clone());
        }
        // Ensure admin is always in the users list.
        if !self.admin.is_empty() && !self.users.contains(&self.admin) {
            self.users.insert(0, self.admin.clone());
        }
        // Ensure all admins are in the users list.
        for admin_id in self.admins.clone() {
            if !self.users.contains(&admin_id) {
                self.users.push(admin_id);
            }
        }
    }

    /// Get the role for a user ID. Returns None if user is not allowed.
    pub fn role_of(&self, user_id: &str) -> Option<Role> {
        if self.admins.iter().any(|a| a == user_id) || self.admin == user_id {
            Some(Role::Admin)
        } else if self.users.iter().any(|u| u == user_id) {
            Some(Role::User)
        } else {
            None
        }
    }

    /// Check if a user is allowed at all (any role).
    pub fn is_allowed(&self, user_id: &str) -> bool {
        self.role_of(user_id).is_some()
    }

    /// Add a user to the users list (if not already present).
    pub fn add_user(&mut self, user_id: &str) {
        if !self.users.iter().any(|u| u == user_id) {
            self.users.push(user_id.to_string());
        }
    }

    /// Remove a user from the users list (and admins if applicable).
    /// Returns false if user_id is the original admin (cannot be removed).
    pub fn remove_user(&mut self, user_id: &str) -> bool {
        if user_id == self.admin {
            return false; // cannot remove original owner
        }
        self.admins.retain(|a| a != user_id);
        self.users.retain(|u| u != user_id);
        true
    }

    /// Promote a user to admin. The user must already be in the users list.
    pub fn promote_to_admin(&mut self, user_id: &str) -> bool {
        if !self.users.iter().any(|u| u == user_id) {
            return false; // not on allowlist
        }
        if !self.admins.iter().any(|a| a == user_id) {
            self.admins.push(user_id.to_string());
        }
        true
    }

    /// Demote an admin to regular user. Cannot demote the original owner.
    pub fn demote_from_admin(&mut self, user_id: &str) -> bool {
        if user_id == self.admin {
            return false; // cannot demote original owner
        }
        self.admins.retain(|a| a != user_id);
        true
    }
}

// ── Shared I/O ───────────────────────────────────────────────────────

/// Get the on-disk path for a channel's role file.
pub fn role_file_path(channel_name: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let temm1e_dir = home.join(".temm1e");
    let filename = match channel_name {
        "telegram" => "allowlist.toml",
        "discord" => "discord_allowlist.toml",
        "slack" => "slack_allowlist.toml",
        "whatsapp" | "whatsapp_web" | "whatsapp_cloud" => "whatsapp_allowlist.toml",
        _ => return None,
    };
    Some(temm1e_dir.join(filename))
}

/// Load a channel's role file from disk.
pub fn load_role_file(channel_name: &str) -> Option<RoleFile> {
    let path = role_file_path(channel_name)?;
    load_role_file_from_path(&path)
}

/// Load a role file from a specific path.
pub fn load_role_file_from_path(path: &Path) -> Option<RoleFile> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut file: RoleFile = toml::from_str(&content).ok()?;
    file.migrate();
    Some(file)
}

/// Save a channel's role file to disk.
pub fn save_role_file(channel_name: &str, data: &RoleFile) -> Result<(), Temm1eError> {
    let path = role_file_path(channel_name).ok_or_else(|| {
        Temm1eError::Config(format!("No role file path for channel '{}'", channel_name))
    })?;
    save_role_file_to_path(&path, data)
}

/// Save a role file to a specific path.
pub fn save_role_file_to_path(path: &Path, data: &RoleFile) -> Result<(), Temm1eError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Temm1eError::Config(format!("Failed to create directory: {}", e)))?;
    }
    let content = toml::to_string_pretty(data)
        .map_err(|e| Temm1eError::Config(format!("Failed to serialize role file: {}", e)))?;
    std::fs::write(path, content)
        .map_err(|e| Temm1eError::Config(format!("Failed to write role file: {}", e)))?;
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_display() {
        assert_eq!(format!("{}", Role::Admin), "admin");
        assert_eq!(format!("{}", Role::User), "user");
    }

    #[test]
    fn admin_has_all_tools() {
        assert!(Role::Admin.has_all_tools());
        assert!(Role::Admin.is_tool_allowed("shell"));
        assert!(Role::Admin.is_tool_allowed("key_manage"));
    }

    #[test]
    fn user_blocked_from_dangerous_tools() {
        assert!(!Role::User.has_all_tools());
        assert!(!Role::User.is_tool_allowed("shell"));
        assert!(!Role::User.is_tool_allowed("key_manage"));
        assert!(!Role::User.is_tool_allowed("invoke_core"));
        assert!(!Role::User.is_tool_allowed("desktop"));
    }

    #[test]
    fn user_can_use_safe_tools() {
        assert!(Role::User.is_tool_allowed("file_read"));
        assert!(Role::User.is_tool_allowed("file_write"));
        assert!(Role::User.is_tool_allowed("browser"));
        assert!(Role::User.is_tool_allowed("web_fetch"));
        assert!(Role::User.is_tool_allowed("git"));
        assert!(Role::User.is_tool_allowed("send_file"));
        assert!(Role::User.is_tool_allowed("memory_manage"));
        assert!(Role::User.is_tool_allowed("use_skill"));
    }

    #[test]
    fn admin_has_all_commands() {
        assert!(Role::Admin.has_all_commands());
        assert!(Role::Admin.is_command_allowed("/addkey"));
        assert!(Role::Admin.is_command_allowed("/model"));
    }

    #[test]
    fn user_command_whitelist() {
        assert!(!Role::User.has_all_commands());
        assert!(Role::User.is_command_allowed("/help"));
        assert!(Role::User.is_command_allowed("/status"));
        assert!(Role::User.is_command_allowed("/usage"));
        assert!(Role::User.is_command_allowed("/mode"));
        assert!(Role::User.is_command_allowed("/mode play"));
        assert!(Role::User.is_command_allowed("/quit"));
        // Blocked
        assert!(!Role::User.is_command_allowed("/addkey"));
        assert!(!Role::User.is_command_allowed("/model"));
        assert!(!Role::User.is_command_allowed("/keys"));
    }

    #[test]
    fn role_file_new_with_admin() {
        let rf = RoleFile::new_with_admin("123");
        assert_eq!(rf.admin, "123");
        assert_eq!(rf.admins, vec!["123"]);
        assert_eq!(rf.users, vec!["123"]);
        assert_eq!(rf.role_of("123"), Some(Role::Admin));
        assert_eq!(rf.role_of("456"), None);
    }

    #[test]
    fn role_file_legacy_migration() {
        // Simulate old format: has admin + users but no admins field
        let mut rf = RoleFile {
            admin: "100".to_string(),
            admins: vec![], // empty = old format
            users: vec!["100".to_string(), "200".to_string()],
        };
        rf.migrate();
        assert_eq!(rf.admins, vec!["100"]);
        assert_eq!(rf.role_of("100"), Some(Role::Admin));
        assert_eq!(rf.role_of("200"), Some(Role::User));
    }

    #[test]
    fn role_file_deserialize_legacy() {
        let toml = r#"
admin = "12345"
users = ["12345", "67890"]
"#;
        let mut rf: RoleFile = toml::from_str(toml).unwrap();
        rf.migrate();
        assert_eq!(rf.role_of("12345"), Some(Role::Admin));
        assert_eq!(rf.role_of("67890"), Some(Role::User));
        assert_eq!(rf.role_of("99999"), None);
    }

    #[test]
    fn role_file_deserialize_new_format() {
        let toml = r#"
admin = "100"
admins = ["100", "200"]
users = ["100", "200", "300"]
"#;
        let rf: RoleFile = toml::from_str(toml).unwrap();
        assert_eq!(rf.role_of("100"), Some(Role::Admin));
        assert_eq!(rf.role_of("200"), Some(Role::Admin));
        assert_eq!(rf.role_of("300"), Some(Role::User));
    }

    #[test]
    fn add_and_remove_user() {
        let mut rf = RoleFile::new_with_admin("1");
        rf.add_user("2");
        assert_eq!(rf.role_of("2"), Some(Role::User));
        rf.remove_user("2");
        assert_eq!(rf.role_of("2"), None);
    }

    #[test]
    fn cannot_remove_original_admin() {
        let mut rf = RoleFile::new_with_admin("1");
        assert!(!rf.remove_user("1"));
        assert_eq!(rf.role_of("1"), Some(Role::Admin));
    }

    #[test]
    fn promote_and_demote() {
        let mut rf = RoleFile::new_with_admin("1");
        rf.add_user("2");

        assert!(rf.promote_to_admin("2"));
        assert_eq!(rf.role_of("2"), Some(Role::Admin));

        assert!(rf.demote_from_admin("2"));
        assert_eq!(rf.role_of("2"), Some(Role::User));
    }

    #[test]
    fn cannot_demote_original_owner() {
        let mut rf = RoleFile::new_with_admin("1");
        assert!(!rf.demote_from_admin("1"));
        assert_eq!(rf.role_of("1"), Some(Role::Admin));
    }

    #[test]
    fn cannot_promote_non_user() {
        let mut rf = RoleFile::new_with_admin("1");
        assert!(!rf.promote_to_admin("999")); // not on allowlist
    }

    #[test]
    fn role_file_path_mapping() {
        assert!(role_file_path("telegram")
            .unwrap()
            .ends_with("allowlist.toml"));
        assert!(role_file_path("discord")
            .unwrap()
            .ends_with("discord_allowlist.toml"));
        assert!(role_file_path("slack")
            .unwrap()
            .ends_with("slack_allowlist.toml"));
        assert!(role_file_path("whatsapp")
            .unwrap()
            .ends_with("whatsapp_allowlist.toml"));
        assert!(role_file_path("unknown").is_none());
    }
}
