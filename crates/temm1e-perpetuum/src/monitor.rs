use sha2::{Digest, Sha256};
use temm1e_core::types::error::Temm1eError;

use crate::types::{CheckResult, ExtractMode, MonitorCheck};

/// Execute a monitor check and return the raw content + hash.
pub async fn execute_check(check: &MonitorCheck) -> Result<CheckResult, Temm1eError> {
    match check {
        MonitorCheck::Web {
            url,
            selector,
            extract,
        } => execute_web_check(url, selector.as_deref(), extract).await,
        MonitorCheck::Command {
            command,
            working_dir,
        } => execute_command_check(command, working_dir.as_deref()).await,
        MonitorCheck::File { path } => execute_file_check(path).await,
    }
}

async fn execute_web_check(
    url: &str,
    selector: Option<&str>,
    extract: &ExtractMode,
) -> Result<CheckResult, Temm1eError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| Temm1eError::Tool(format!("HTTP client build: {e}")))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| Temm1eError::Tool(format!("HTTP GET {url}: {e}")))?;

    if !resp.status().is_success() {
        return Err(Temm1eError::Tool(format!(
            "HTTP {} from {url}",
            resp.status()
        )));
    }

    // Cap response body at 10MB to prevent OOM on large pages
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Temm1eError::Tool(format!("Read body from {url}: {e}")))?;
    if bytes.len() > 10 * 1024 * 1024 {
        return Err(Temm1eError::Tool(format!(
            "Response too large ({} bytes) from {url}",
            bytes.len()
        )));
    }
    let body = String::from_utf8_lossy(&bytes).to_string();

    let content = extract_content(&body, selector, extract)?;
    let hash = sha256_hex(&content);
    Ok(CheckResult {
        content,
        content_hash: hash,
    })
}

fn extract_content(
    body: &str,
    selector: Option<&str>,
    extract: &ExtractMode,
) -> Result<String, Temm1eError> {
    match (selector, extract) {
        (Some(sel), _) => {
            let document = scraper::Html::parse_document(body);
            let css_selector = scraper::Selector::parse(sel)
                .map_err(|e| Temm1eError::Tool(format!("Invalid CSS selector: {e:?}")))?;
            let text: Vec<String> = document
                .select(&css_selector)
                .map(|el| el.text().collect::<Vec<_>>().join(""))
                .collect();
            Ok(text.join("\n"))
        }
        (None, ExtractMode::JsonPath(path)) => {
            let json: serde_json::Value = serde_json::from_str(body)
                .map_err(|e| Temm1eError::Tool(format!("JSON parse: {e}")))?;
            Ok(json
                .pointer(path)
                .map(|v| v.to_string())
                .unwrap_or_default())
        }
        _ => {
            // Full text, truncated to 2000 chars for LLM context
            Ok(body.chars().take(2000).collect())
        }
    }
}

async fn execute_command_check(
    command: &str,
    working_dir: Option<&str>,
) -> Result<CheckResult, Temm1eError> {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(command);
    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    let output = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output())
        .await
        .map_err(|_| Temm1eError::Tool(format!("Command timed out: {command}")))?
        .map_err(|e| Temm1eError::Tool(format!("Command failed: {e}")))?;

    let content = String::from_utf8_lossy(&output.stdout).to_string();
    let hash = sha256_hex(&content);
    Ok(CheckResult {
        content,
        content_hash: hash,
    })
}

async fn execute_file_check(path: &str) -> Result<CheckResult, Temm1eError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "[file not found]".to_string(),
        Err(e) => {
            return Err(Temm1eError::Tool(format!("Read file {path}: {e}")));
        }
    };
    let hash = sha256_hex(&content);
    Ok(CheckResult {
        content,
        content_hash: hash,
    })
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_deterministic() {
        let h1 = sha256_hex("hello world");
        let h2 = sha256_hex("hello world");
        assert_eq!(h1, h2);
        assert_ne!(sha256_hex("hello"), sha256_hex("world"));
    }

    #[test]
    fn extract_full_text_truncates() {
        let body = "a".repeat(5000);
        let result = extract_content(&body, None, &ExtractMode::FullText).unwrap();
        assert_eq!(result.len(), 2000);
    }

    #[test]
    fn extract_json_path() {
        let body = r#"{"data":{"count":42}}"#;
        let result = extract_content(body, None, &ExtractMode::JsonPath("/data/count".into()));
        assert_eq!(result.unwrap(), "42");
    }

    #[test]
    fn extract_css_selector() {
        let body = r#"<html><body><div class="title">Hello</div><div class="title">World</div></body></html>"#;
        let result = extract_content(body, Some(".title"), &ExtractMode::Selector).unwrap();
        assert_eq!(result, "Hello\nWorld");
    }

    #[tokio::test]
    async fn file_check_missing() {
        let result = execute_file_check("/tmp/nonexistent_perpetuum_test_file").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().content, "[file not found]");
    }

    #[tokio::test]
    async fn command_check_echo() {
        let result = execute_command_check("echo hello", None).await;
        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(r.content.contains("hello"));
        assert!(!r.content_hash.is_empty());
    }
}
