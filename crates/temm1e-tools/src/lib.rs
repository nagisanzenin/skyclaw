//! TEMM1E Tools — agent capabilities (shell, file, web, browser, etc.)

#[cfg(feature = "browser")]
mod browser;
#[cfg(feature = "browser")]
pub mod browser_observation;
#[cfg(feature = "browser")]
pub mod browser_pool;
#[cfg(feature = "browser")]
pub mod browser_session;
mod check_messages;
pub mod credential_scrub;
pub mod custom_tools;
#[cfg(feature = "desktop-control")]
pub mod desktop_tool;
mod file;
mod git;
pub mod grounding;
mod key_manage;
mod lambda_recall;
mod memory_manage;
mod mode_switch;
pub mod prowl_blueprints;
mod send_file;
mod send_message;
mod shell;
mod usage_audit;
mod web_fetch;

#[cfg(feature = "browser")]
pub use browser::BrowserTool;
#[cfg(feature = "browser")]
pub use browser_pool::BrowserPool;
pub use check_messages::{CheckMessagesTool, PendingMessages};
pub use custom_tools::{CustomToolRegistry, SelfCreateTool};
pub use file::{FileListTool, FileReadTool, FileWriteTool};
pub use git::GitTool;
pub use key_manage::KeyManageTool;
pub use lambda_recall::LambdaRecallTool;
pub use memory_manage::MemoryManageTool;
pub use mode_switch::{ModeSwitchTool, SharedMode};
pub use send_file::SendFileTool;
pub use send_message::SendMessageTool;
pub use shell::ShellTool;
pub use usage_audit::UsageAuditTool;
pub use web_fetch::WebFetchTool;

use std::sync::Arc;
use temm1e_core::types::config::ToolsConfig;
use temm1e_core::{Channel, Memory, SetupLinkGenerator, Tool, UsageStore, Vault};

/// Create tools based on the configuration flags.
/// Pass an optional channel for file transfer tools, an optional
/// pending-message queue for the check_messages tool, an optional
/// memory backend for the memory_manage tool, an optional
/// setup link generator for the key_manage tool, and an optional
/// shared mode handle for the mode_switch tool.
#[allow(clippy::too_many_arguments)]
pub fn create_tools(
    config: &ToolsConfig,
    channel: Option<Arc<dyn Channel>>,
    pending_messages: Option<PendingMessages>,
    memory: Option<Arc<dyn Memory>>,
    setup_link_gen: Option<Arc<dyn SetupLinkGenerator>>,
    usage_store: Option<Arc<dyn UsageStore>>,
    shared_mode: Option<SharedMode>,
    vault: Option<Arc<dyn Vault>>,
) -> Vec<Arc<dyn Tool>> {
    // vault is only used when browser feature is enabled
    let _ = &vault;

    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();

    if config.shell {
        tools.push(Arc::new(ShellTool::new()));
    }

    if config.file {
        tools.push(Arc::new(FileReadTool::new()));
        tools.push(Arc::new(FileWriteTool::new()));
        tools.push(Arc::new(FileListTool::new()));
    }

    if config.git {
        tools.push(Arc::new(GitTool::new()));
    }

    if config.http {
        tools.push(Arc::new(WebFetchTool::new()));
    }

    // Add channel-dependent tools
    if let Some(ch) = channel {
        // send_message: send intermediate text messages during tool execution
        tools.push(Arc::new(SendMessageTool::new(ch.clone())));

        // send_file: send files if channel supports file transfer
        if ch.file_transfer().is_some() {
            tools.push(Arc::new(SendFileTool::new(ch)));
        }
    }

    // check_messages: lets agent peek at pending user messages during tasks
    if let Some(pending) = pending_messages {
        tools.push(Arc::new(CheckMessagesTool::new(pending)));
    }

    // memory_manage: persistent knowledge store for the agent
    // lambda_recall: recall faded λ-memories by hash prefix
    if let Some(mem) = memory {
        tools.push(Arc::new(MemoryManageTool::new(Arc::clone(&mem))));
        tools.push(Arc::new(LambdaRecallTool::new(mem)));
    }

    // key_manage: generates setup links and guides users through key operations
    tools.push(Arc::new(KeyManageTool::new(setup_link_gen)));

    // usage_audit: query usage stats and toggle usage display
    if let Some(store) = usage_store {
        tools.push(Arc::new(UsageAuditTool::new(store)));
    }

    // mode_switch: toggle personality mode between PLAY, WORK, and PRO
    if let Some(mode) = shared_mode {
        tools.push(Arc::new(ModeSwitchTool::new(mode)));
    }

    // browser: headless Chrome automation (stealth mode)
    #[cfg(feature = "browser")]
    if config.browser {
        let browser_tool = BrowserTool::with_timeout(config.browser_timeout_secs);
        let browser_tool = if let Some(ref v) = vault {
            browser_tool.with_vault(Arc::clone(v))
        } else {
            browser_tool
        };
        tools.push(Arc::new(browser_tool));
    }

    // desktop: OS-level screen capture + input simulation (Tem Gaze)
    #[cfg(feature = "desktop-control")]
    {
        match desktop_tool::DesktopTool::new(0) {
            Ok(dt) => tools.push(Arc::new(dt)),
            Err(e) => tracing::warn!("Desktop tool unavailable: {}", e),
        }
    }

    tracing::info!(count = tools.len(), "Tools registered");
    tools
}

/// Create tools and return a separate reference to the BrowserTool (if enabled).
///
/// This is used by the gateway to access browser-specific methods for the
/// `/browser` command without needing to downcast from `Arc<dyn Tool>`.
#[cfg(feature = "browser")]
#[allow(clippy::too_many_arguments)]
pub fn create_tools_with_browser(
    config: &ToolsConfig,
    channel: Option<Arc<dyn Channel>>,
    pending_messages: Option<PendingMessages>,
    memory: Option<Arc<dyn Memory>>,
    setup_link_gen: Option<Arc<dyn SetupLinkGenerator>>,
    usage_store: Option<Arc<dyn UsageStore>>,
    shared_mode: Option<SharedMode>,
    vault: Option<Arc<dyn Vault>>,
) -> (Vec<Arc<dyn Tool>>, Option<Arc<BrowserTool>>) {
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();

    if config.shell {
        tools.push(Arc::new(ShellTool::new()));
    }

    if config.file {
        tools.push(Arc::new(FileReadTool::new()));
        tools.push(Arc::new(FileWriteTool::new()));
        tools.push(Arc::new(FileListTool::new()));
    }

    if config.git {
        tools.push(Arc::new(GitTool::new()));
    }

    if config.http {
        tools.push(Arc::new(WebFetchTool::new()));
    }

    if let Some(ch) = channel {
        tools.push(Arc::new(SendMessageTool::new(ch.clone())));
        if ch.file_transfer().is_some() {
            tools.push(Arc::new(SendFileTool::new(ch)));
        }
    }

    if let Some(pending) = pending_messages {
        tools.push(Arc::new(CheckMessagesTool::new(pending)));
    }

    if let Some(mem) = memory {
        tools.push(Arc::new(MemoryManageTool::new(Arc::clone(&mem))));
        tools.push(Arc::new(LambdaRecallTool::new(mem)));
    }

    tools.push(Arc::new(KeyManageTool::new(setup_link_gen)));

    if let Some(store) = usage_store {
        tools.push(Arc::new(UsageAuditTool::new(store)));
    }

    if let Some(mode) = shared_mode {
        tools.push(Arc::new(ModeSwitchTool::new(mode)));
    }

    // browser: create as Arc<BrowserTool> and keep a reference
    let browser_ref = if config.browser {
        let browser_tool = BrowserTool::with_timeout(config.browser_timeout_secs);
        let browser_tool = if let Some(ref v) = vault {
            browser_tool.with_vault(Arc::clone(v))
        } else {
            browser_tool
        };
        let arc = Arc::new(browser_tool);
        tools.push(arc.clone() as Arc<dyn Tool>);
        Some(arc)
    } else {
        None
    };

    // desktop: OS-level screen capture + input simulation (Tem Gaze)
    #[cfg(feature = "desktop-control")]
    {
        match desktop_tool::DesktopTool::new(0) {
            Ok(dt) => tools.push(Arc::new(dt)),
            Err(e) => tracing::warn!("Desktop tool unavailable: {}", e),
        }
    }

    tracing::info!(count = tools.len(), "Tools registered (with browser ref)");
    (tools, browser_ref)
}

/// Run an interactive browser login session from the CLI.
///
/// Launches a headless Chrome browser, navigates to the given URL, and enters
/// an interactive loop where the user sees numbered elements and types numbers
/// to click, text to fill fields, or "done" to finish. The session cookies and
/// storage are captured and encrypted in the vault.
///
/// Returns a human-readable summary string on success.
#[cfg(feature = "browser")]
pub async fn browser_session_login(
    service: &str,
    url: &str,
    vault: &dyn temm1e_core::Vault,
) -> Result<String, temm1e_core::types::error::Temm1eError> {
    use chromiumoxide::browser::{Browser, BrowserConfig};
    use futures::StreamExt;
    use std::io::{self, BufRead, Write};

    // Launch browser with stealth flags (matching BrowserTool's config)
    let config = BrowserConfig::builder()
        .no_sandbox()
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--disable-dev-shm-usage")
        .arg("--disable-blink-features=AutomationControlled")
        .arg("--disable-infobars")
        .arg("--disable-background-timer-throttling")
        .arg("--disable-backgrounding-occluded-windows")
        .arg("--disable-renderer-backgrounding")
        .arg("--disable-ipc-flooding-protection")
        .arg("--window-size=1280,900")
        .build()
        .map_err(|e| {
            temm1e_core::types::error::Temm1eError::Tool(format!("Browser config failed: {}", e))
        })?;

    let (browser, mut handler) = Browser::launch(config).await.map_err(|e| {
        temm1e_core::types::error::Temm1eError::Tool(format!("Browser launch failed: {}", e))
    })?;

    // Spawn handler in background — continue on WS errors (chromiumoxide 0.7
    // can't deserialize some CDP messages from newer Chrome, but the connection
    // still works for our purposes)
    let handle = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let mut session =
        browser_session::InteractiveBrowseSession::new(&browser, service, url).await?;

    let mut turn_count: usize = 0;
    loop {
        // Capture annotated screenshot and print element list
        let (_png_data, description) = session.capture_annotated().await?;

        println!("\n--- Page Elements ---");
        println!("{}", description);
        println!("---");
        println!("Type a number to click, text to type into focused field, or 'done' to finish.");

        // Read user input from stdin (blocking via spawn_blocking)
        eprint!("login> ");
        io::stderr().flush().ok();

        let input = tokio::task::spawn_blocking(|| {
            let stdin = io::stdin();
            let mut line = String::new();
            stdin.lock().read_line(&mut line).ok();
            line
        })
        .await
        .map_err(|e| {
            temm1e_core::types::error::Temm1eError::Tool(format!("Failed to read input: {}", e))
        })?;

        let input = input.trim().to_string();
        if input.is_empty() {
            continue;
        }

        match session.handle_input(&input).await {
            Ok(browser_session::SessionAction::Continue) => {
                turn_count += 1;
                println!("  Action completed.");
            }
            Ok(browser_session::SessionAction::Done) => {
                // Capture session to vault
                session.capture_session(vault).await?;
                break;
            }
            Err(e) => {
                println!("  Error: {}", e);
            }
        }
    }

    // Cleanup
    handle.abort();

    Ok(format!(
        "Login session for '{}' completed ({} interactions). Session saved to vault.",
        service, turn_count
    ))
}
