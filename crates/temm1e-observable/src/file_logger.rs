//! Centralized file logger — daily rotating log at ~/.temm1e/logs/.
//!
//! Always on, local only, zero privacy risk. Provides a persistent log
//! file that users can attach to bug reports.

use std::path::PathBuf;
use tracing_appender::rolling::RollingFileAppender;

/// Maximum total log directory size in bytes (100 MB).
const MAX_LOG_DIR_BYTES: u64 = 100 * 1024 * 1024;

/// Log directory: ~/.temm1e/logs/ (Unix) or %LOCALAPPDATA%\temm1e\logs\ (Windows).
pub fn log_dir() -> PathBuf {
    #[cfg(windows)]
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("temm1e")
        .join("logs");

    #[cfg(not(windows))]
    let base = temm1e_core::config::data_dir().join("logs");

    std::fs::create_dir_all(&base).ok();
    base
}

/// Create a daily-rolling file appender.
pub fn create_file_appender() -> RollingFileAppender {
    tracing_appender::rolling::daily(log_dir(), "temm1e.log")
}

/// Log file path for the current day (for user-facing messages).
pub fn current_log_path() -> PathBuf {
    log_dir().join("temm1e.log")
}

/// Delete log files older than `max_days` and enforce the size budget.
pub fn cleanup_logs(max_days: u32) {
    let dir = log_dir();
    let cutoff = chrono::Utc::now() - chrono::Duration::days(max_days as i64);

    // Pass 1: delete files older than max_days
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    let modified: chrono::DateTime<chrono::Utc> = modified.into();
                    if modified < cutoff {
                        std::fs::remove_file(entry.path()).ok();
                    }
                }
            }
        }
    }

    // Pass 2: enforce hard size budget regardless of age
    enforce_log_budget(&dir);
}

/// Delete oldest files until directory is under MAX_LOG_DIR_BYTES.
fn enforce_log_budget(dir: &std::path::Path) {
    let mut files: Vec<(PathBuf, std::time::SystemTime, u64)> = Vec::new();
    let mut total: u64 = 0;

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                let size = meta.len();
                total += size;
                if let Ok(modified) = meta.modified() {
                    files.push((entry.path(), modified, size));
                }
            }
        }
    }

    if total <= MAX_LOG_DIR_BYTES {
        return;
    }

    // Sort oldest first
    files.sort_by_key(|(_, modified, _)| *modified);

    let mut freed: u64 = 0;
    let excess = total - MAX_LOG_DIR_BYTES;
    for (path, _, size) in &files {
        if freed >= excess {
            break;
        }
        std::fs::remove_file(path).ok();
        freed += size;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_dir_is_under_temm1e() {
        let dir = log_dir();
        let dir_str = dir.to_string_lossy();
        assert!(dir.starts_with(temm1e_core::config::data_dir()));
        assert!(dir_str.contains("logs"));
    }

    #[test]
    fn current_log_path_ends_with_temm1e_log() {
        let path = current_log_path();
        assert!(path.to_string_lossy().ends_with("temm1e.log"));
    }

    #[test]
    fn create_file_appender_succeeds() {
        let _appender = create_file_appender();
    }

    #[test]
    fn cleanup_logs_does_not_panic() {
        cleanup_logs(7);
    }

    #[test]
    fn enforce_budget_on_empty_dir() {
        let dir = std::env::temp_dir().join("temm1e_test_empty_budget");
        std::fs::create_dir_all(&dir).ok();
        enforce_log_budget(&dir);
        std::fs::remove_dir_all(&dir).ok();
    }
}
