//! Token storage and auto-refresh for OpenAI Codex OAuth tokens.
//!
//! Tokens are stored in `~/.temm1e/oauth.json`. Access tokens expire in ~1 hour
//! and are auto-refreshed using the refresh token. A stable OS file lock
//! serializes refresh/login/logout across stores and processes. A rotation
//! marker prevents blind reuse after an interrupted or ambiguous refresh.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use temm1e_core::private_file::PrivateFileLock;
use temm1e_core::types::error::Temm1eError;
use tokio::sync::Mutex;

/// OAuth token set — stored in ~/.temm1e/oauth.json
#[derive(Clone, Serialize, Deserialize)]
pub struct CodexOAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64, // Unix timestamp
    pub email: String,
    pub account_id: String,
}

impl std::fmt::Debug for CodexOAuthTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexOAuthTokens")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Thread-safe token store with auto-refresh.
pub struct TokenStore {
    tokens: Mutex<CodexOAuthTokens>,
    path: PathBuf,
    client: reqwest::Client,
    token_endpoint: String,
}

/// The OpenAI auth token endpoint.
const TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
/// The public Codex CLI client ID (used by OpenClaw, Roo Code, OpenCode, etc.)
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// Refresh buffer — refresh if within this many seconds of expiry.
const REFRESH_BUFFER_SECS: u64 = 300; // 5 minutes

impl TokenStore {
    /// Create a new token store from saved tokens.
    pub fn new(tokens: CodexOAuthTokens) -> Self {
        Self {
            path: Self::default_path(),
            tokens: Mutex::new(tokens),
            client: reqwest::Client::new(),
            token_endpoint: TOKEN_ENDPOINT.into(),
        }
    }

    /// Load tokens from ~/.temm1e/oauth.json
    pub fn load() -> Result<Self, Temm1eError> {
        let path = Self::default_path();
        let content = std::fs::read_to_string(&path).map_err(|e| {
            Temm1eError::Auth(format!(
                "No OAuth tokens found at {}. Run `temm1e auth login` first. ({})",
                path.display(),
                e
            ))
        })?;
        let tokens: CodexOAuthTokens = serde_json::from_str(&content)
            .map_err(|e| Temm1eError::Auth(format!("Failed to parse OAuth tokens: {}", e)))?;
        Ok(Self {
            path,
            tokens: Mutex::new(tokens),
            client: reqwest::Client::new(),
            token_endpoint: TOKEN_ENDPOINT.into(),
        })
    }

    /// Get a fresh access token, auto-refreshing if near expiry.
    ///
    /// The Mutex ensures only one refresh happens at a time — concurrent callers
    /// will wait for the refresh to complete and then get the fresh token.
    pub async fn get_access_token(&self) -> Result<String, Temm1eError> {
        let mut tokens = self.tokens.lock().await;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(35);
        let _file_lock = loop {
            if let Some(lock) = PrivateFileLock::try_exclusive(&self.path.with_extension("lock"))
                .map_err(|e| Temm1eError::Auth(format!("OAuth lock failed: {e}")))?
            {
                break lock;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Temm1eError::Auth(
                    "OAuth credentials are busy in another process; retry shortly".into(),
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        let marker = self.path.with_extension("refresh-pending");
        if marker.exists() {
            return Err(Temm1eError::Auth("A prior OAuth refresh has an uncertain outcome. Run `temm1e auth login` to reconnect safely.".into()));
        }
        // Reload under the cross-process lock: another store may have rotated
        // tokens, switched accounts, or logged out since this object was made.
        let content = std::fs::read_to_string(&self.path).map_err(|_| {
            Temm1eError::Auth("OAuth credentials are unavailable. Run `temm1e auth login`.".into())
        })?;
        *tokens = serde_json::from_str(&content).map_err(|_| {
            Temm1eError::Auth(
                "OAuth credentials are malformed; reconnect with `temm1e auth login`.".into(),
            )
        })?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if tokens.expires_at > now + REFRESH_BUFFER_SECS {
            return Ok(tokens.access_token.clone());
        }

        tracing::info!("Refreshing Codex OAuth token");
        temm1e_core::private_file::write_private_atomic(
            &marker,
            b"OAuth rotation pending; reconnect if interrupted\n",
        )
        .map_err(|e| Temm1eError::Auth(format!("Could not record OAuth refresh intent: {e}")))?;
        let new_tokens =
            match Self::refresh_token(&self.client, &tokens.refresh_token, &self.token_endpoint)
                .await
            {
                Ok(tokens) => tokens,
                Err(RefreshFailure::Rejected(error)) => {
                    Self::remove_if_exists(&marker)?;
                    return Err(error);
                }
                Err(RefreshFailure::Uncertain(error)) => return Err(error),
            };

        // Preserve email and account_id from the original tokens
        let updated = CodexOAuthTokens {
            access_token: new_tokens.access_token,
            refresh_token: new_tokens.refresh_token,
            expires_at: new_tokens.expires_at,
            email: tokens.email.clone(),
            account_id: tokens.account_id.clone(),
        };

        // Rotation has already happened remotely; do not retry an old refresh
        // token in this process if durable storage fails.
        *tokens = updated.clone();
        self.save_unlocked(&updated)?;
        std::fs::remove_file(&marker)
            .map_err(|e| Temm1eError::Auth(format!("Failed to finalize OAuth refresh: {e}")))?;
        tracing::info!("Codex OAuth token refreshed successfully");

        Ok(updated.access_token)
    }

    /// Get a clone of the current tokens (for export).
    pub async fn get_tokens(&self) -> CodexOAuthTokens {
        self.tokens.lock().await.clone()
    }

    /// Get the current email (for display purposes).
    pub async fn email(&self) -> String {
        self.tokens.lock().await.email.clone()
    }

    /// Get the current account ID.
    pub async fn account_id(&self) -> String {
        self.tokens.lock().await.account_id.clone()
    }

    /// Check if the token is expired or will expire within the buffer period.
    pub async fn is_expired(&self) -> bool {
        let tokens = self.tokens.lock().await;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        tokens.expires_at <= now + REFRESH_BUFFER_SECS
    }

    /// Get the expiry time as a human-readable string.
    pub async fn expires_in(&self) -> String {
        let tokens = self.tokens.lock().await;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if tokens.expires_at <= now {
            "expired".to_string()
        } else {
            let remaining = tokens.expires_at - now;
            if remaining > 3600 {
                format!("{}h {}m", remaining / 3600, (remaining % 3600) / 60)
            } else {
                format!("{}m", remaining / 60)
            }
        }
    }

    /// Save tokens to disk.
    pub fn save_to_disk(&self, tokens: &CodexOAuthTokens) -> Result<(), Temm1eError> {
        let _lock = Self::lock_now(&self.path)?;
        self.save_unlocked(tokens)?;
        Self::remove_if_exists(&self.path.with_extension("refresh-pending"))
    }

    fn lock_now(path: &std::path::Path) -> Result<PrivateFileLock, Temm1eError> {
        PrivateFileLock::try_exclusive(&path.with_extension("lock"))
            .map_err(|e| Temm1eError::Auth(format!("OAuth lock failed: {e}")))?
            .ok_or_else(|| {
                Temm1eError::Auth(
                    "OAuth credentials are busy in another process; retry shortly".into(),
                )
            })
    }

    fn remove_if_exists(path: &std::path::Path) -> Result<(), Temm1eError> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Temm1eError::Auth(format!(
                "Failed to remove OAuth state: {e}"
            ))),
        }
    }

    fn save_unlocked(&self, tokens: &CodexOAuthTokens) -> Result<(), Temm1eError> {
        let content = serde_json::to_string_pretty(tokens)
            .map_err(|e| Temm1eError::Auth(format!("Failed to serialize tokens: {}", e)))?;
        temm1e_core::private_file::write_private_atomic(&self.path, content.as_bytes())
            .map_err(|e| Temm1eError::Auth(format!("Failed to persist OAuth tokens: {e}")))?;
        tracing::debug!(path = %self.path.display(), "OAuth tokens saved");
        Ok(())
    }

    /// Refresh the access token using the refresh token.
    async fn refresh_token(
        client: &reqwest::Client,
        refresh_token: &str,
        endpoint: &str,
    ) -> Result<RefreshResponse, RefreshFailure> {
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ];

        let resp = client
            .post(endpoint)
            .timeout(std::time::Duration::from_secs(30))
            .form(&params)
            .send()
            .await
            .map_err(|e| {
                RefreshFailure::Uncertain(Temm1eError::Auth(format!(
                    "Token refresh request failed: {}",
                    e.without_url()
                )))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let error = Temm1eError::Auth(format!(
                "Token refresh failed ({status}); reconnect if authentication was rejected"
            ));
            return Err(if status.is_client_error() {
                RefreshFailure::Rejected(error)
            } else {
                RefreshFailure::Uncertain(error)
            });
        }

        let token_resp: TokenResponse = resp.json().await.map_err(|_| {
            RefreshFailure::Uncertain(Temm1eError::Auth(
                "Failed to parse refresh response; reconnect before retrying".into(),
            ))
        })?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Ok(RefreshResponse {
            access_token: token_resp.access_token,
            refresh_token: token_resp
                .refresh_token
                .unwrap_or_else(|| refresh_token.to_string()),
            expires_at: now + token_resp.expires_in.unwrap_or(3600),
        })
    }

    /// Default path: ~/.temm1e/oauth.json
    fn default_path() -> PathBuf {
        temm1e_core::config::data_dir().join("oauth.json")
    }

    /// Delete the token file (for logout).
    pub fn delete() -> Result<(), Temm1eError> {
        let path = Self::default_path();
        let _lock = Self::lock_now(&path)?;
        Self::remove_if_exists(&path)?;
        Self::remove_if_exists(&path.with_extension("refresh-pending"))
    }

    /// Check if tokens exist on disk.
    pub fn exists() -> bool {
        Self::default_path().exists()
    }
}

/// Raw token response from OpenAI auth endpoint.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[allow(dead_code)]
    id_token: Option<String>,
}

enum RefreshFailure {
    Rejected(Temm1eError),
    Uncertain(Temm1eError),
}

/// Internal struct for refresh results.
struct RefreshResponse {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    async fn separate_stores_share_one_refresh_and_observe_logout() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("oauth.json");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            let body =
                r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600}"#;
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let tokens = CodexOAuthTokens {
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            expires_at: 0,
            email: "fixture".into(),
            account_id: "fixture".into(),
        };
        let make_store = || TokenStore {
            path: path.clone(),
            tokens: Mutex::new(tokens.clone()),
            client: reqwest::Client::new(),
            token_endpoint: endpoint.clone(),
        };
        let first = make_store();
        let second = make_store();
        first.save_to_disk(&tokens).unwrap();
        let (a, b) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(first.get_access_token(), second.get_access_token())
        })
        .await
        .unwrap();
        assert_eq!(a.unwrap(), "new-access");
        assert_eq!(b.unwrap(), "new-access");
        server.await.unwrap();
        assert!(!path.with_extension("refresh-pending").exists());
        let saved: CodexOAuthTokens =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved.refresh_token, "new-refresh");
        {
            let _lock = TokenStore::lock_now(&path).unwrap();
            TokenStore::remove_if_exists(&path).unwrap();
        }
        assert!(
            first.get_access_token().await.is_err(),
            "logout invalidates a cached access token"
        );
    }

    #[tokio::test]
    async fn uncertain_rotation_requires_reconnection_before_any_network_call() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("oauth.json");
        let tokens = CodexOAuthTokens {
            access_token: "fixture".into(),
            refresh_token: "fixture".into(),
            expires_at: 0,
            email: "fixture".into(),
            account_id: "fixture".into(),
        };
        let store = TokenStore {
            path: path.clone(),
            tokens: Mutex::new(tokens.clone()),
            client: reqwest::Client::new(),
            token_endpoint: "http://127.0.0.1:1".into(),
        };
        store.save_to_disk(&tokens).unwrap();
        std::fs::write(path.with_extension("refresh-pending"), "pending").unwrap();
        assert!(store
            .get_access_token()
            .await
            .unwrap_err()
            .to_string()
            .contains("uncertain outcome"));
        store.save_to_disk(&tokens).unwrap();
        assert!(!path.with_extension("refresh-pending").exists());
    }

    #[test]
    fn oauth_debug_redacts_credentials() {
        let tokens = super::CodexOAuthTokens {
            access_token: "private-access".into(),
            refresh_token: "private-refresh".into(),
            expires_at: 10,
            email: "private-email".into(),
            account_id: "private-account".into(),
        };
        let rendered = format!("{tokens:?}");
        assert!(!rendered.contains("private-"));
        assert!(rendered.contains("expires_at"));
    }
    use super::*;

    #[test]
    fn default_path_ends_with_oauth_json() {
        let path = TokenStore::default_path();
        assert!(path.ends_with("oauth.json"));
        assert_eq!(
            path.parent(),
            Some(temm1e_core::config::data_dir().as_path())
        );
    }

    #[test]
    fn token_serialization_roundtrip() {
        let tokens = CodexOAuthTokens {
            access_token: "eyJhb-test".to_string(),
            refresh_token: "ort_test_refresh".to_string(),
            expires_at: 1710180000,
            email: "test@example.com".to_string(),
            account_id: "org-test123".to_string(),
        };
        let json = serde_json::to_string(&tokens).unwrap();
        let parsed: CodexOAuthTokens = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.email, "test@example.com");
        assert_eq!(parsed.expires_at, 1710180000);
    }

    #[tokio::test]
    async fn token_store_expiry_check() {
        let tokens = CodexOAuthTokens {
            access_token: "test".to_string(),
            refresh_token: "test".to_string(),
            expires_at: 0, // Already expired
            email: "test@example.com".to_string(),
            account_id: "org-test".to_string(),
        };
        let store = TokenStore::new(tokens);
        assert!(store.is_expired().await);
    }

    #[tokio::test]
    async fn token_store_not_expired() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let tokens = CodexOAuthTokens {
            access_token: "test".to_string(),
            refresh_token: "test".to_string(),
            expires_at: now + 7200, // 2 hours from now
            email: "test@example.com".to_string(),
            account_id: "org-test".to_string(),
        };
        let store = TokenStore::new(tokens);
        assert!(!store.is_expired().await);
    }
}
