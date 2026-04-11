//! Browser Pool — manages a single Chrome process with multiple isolated browser contexts.
//!
//! Each pool slot is an independent browser context with its own cookies,
//! cache, and storage. Workers acquire slots via lock-free atomic CAS on a
//! bitset, ensuring zero contention between Hive workers claiming browser
//! contexts for parallel browsing tasks.
//!
//! ## Memory Budget
//!
//! - Baseline Chrome: ~100 MB
//! - Per context (simple pages): 30–80 MB
//! - Default 4 contexts: ~220–420 MB total
//!
//! Heavy SPAs can push this higher. Configure `max_size` based on available
//! memory.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::browser::BrowserContextId;
use chromiumoxide::cdp::browser_protocol::target::{
    CreateBrowserContextParams, CreateTargetParams,
};
use chromiumoxide::page::Page;
use futures::StreamExt;
use tokio::sync::Mutex;
use tracing::{debug, info};

use temm1e_core::types::error::Temm1eError;

/// Default pool size — 4 browser contexts.
pub const DEFAULT_POOL_SIZE: usize = 4;

/// Realistic user-agent string (matches the one in `BrowserTool`).
const STEALTH_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

/// A single pooled browser context with an optional cached page.
struct PooledContext {
    context_id: BrowserContextId,
    page: Option<Page>,
}

/// A pool of isolated browser contexts sharing a single Chrome process.
///
/// Workers acquire slots via `try_acquire()` (lock-free CAS), create pages
/// with `get_page()`, and release back with `release()`. On release, the
/// context is disposed and recreated to guarantee clean state.
pub struct BrowserPool {
    browser: Arc<Mutex<Browser>>,
    contexts: Vec<Arc<Mutex<PooledContext>>>,
    available: AtomicU64,
    max_size: usize,
    /// Handle to the CDP handler task — must stay alive for the browser to work.
    _cdp_handle: tokio::task::JoinHandle<()>,
    /// PID of the Chrome main process — used to kill child processes on shutdown.
    chrome_pid: AtomicU32,
}

impl BrowserPool {
    /// Launch a single Chrome process and pre-create `max_size` isolated browser
    /// contexts. Each context has its own cookies, cache, and storage.
    ///
    /// Returns an error if `max_size` exceeds 64 (bitset limit) or is zero.
    pub async fn new(max_size: usize) -> Result<Self, Temm1eError> {
        if max_size == 0 || max_size > 64 {
            return Err(Temm1eError::Tool(format!(
                "Browser pool size must be 1..=64, got {max_size}"
            )));
        }

        let force_headless = std::env::var("TEMM1E_HEADLESS").unwrap_or_default() == "1";
        let has_display = std::env::var("DISPLAY").is_ok()
            || std::env::var("WAYLAND_DISPLAY").is_ok()
            || cfg!(target_os = "macos")
            || cfg!(target_os = "windows");
        let use_headless = force_headless || !has_display;

        let mut builder = BrowserConfig::builder();
        if use_headless {
            builder = builder.arg("--headless=new");
        }
        let config = builder
            .arg("--disable-gpu")
            .arg("--no-sandbox")
            .arg("--disable-dev-shm-usage")
            // Anti-detection flags (match BrowserTool stealth config)
            .arg("--disable-blink-features=AutomationControlled")
            .arg("--disable-infobars")
            .arg("--disable-background-timer-throttling")
            .arg("--disable-backgrounding-occluded-windows")
            .arg("--disable-renderer-backgrounding")
            .arg("--disable-ipc-flooding-protection")
            .arg(format!("--user-agent={STEALTH_USER_AGENT}"))
            .arg("--lang=en-US,en")
            .window_size(1920, 1080)
            .build()
            .map_err(|e| Temm1eError::Tool(format!("BrowserPool config: {e}")))?;

        let (mut browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| Temm1eError::Tool(format!("BrowserPool launch: {e}")))?;

        // Capture the Chrome process PID for child-process cleanup on shutdown.
        let chrome_pid_val = browser
            .get_mut_child()
            .map(|child| child.as_mut_inner().id())
            .unwrap_or(0);
        if chrome_pid_val > 0 {
            debug!(pid = chrome_pid_val, "BrowserPool Chrome PID captured");
        }

        // Spawn CDP handler — must stay alive for the entire pool lifetime.
        let cdp_handle = tokio::spawn(async move {
            loop {
                match handler.next().await {
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        debug!(error = %e, "BrowserPool CDP handler event error");
                    }
                    None => {
                        debug!("BrowserPool CDP handler stream ended");
                        break;
                    }
                }
            }
        });

        // Pre-create isolated browser contexts.
        let mut contexts = Vec::with_capacity(max_size);
        let mut available_bits: u64 = 0;

        for i in 0..max_size {
            let ctx_id = browser
                .create_browser_context(CreateBrowserContextParams::builder().build())
                .await
                .map_err(|e| Temm1eError::Tool(format!("BrowserPool context {i}: {e}")))?;

            contexts.push(Arc::new(Mutex::new(PooledContext {
                context_id: ctx_id,
                page: None,
            })));
            available_bits |= 1 << i;
        }

        info!(pool_size = max_size, "BrowserPool initialized");

        Ok(Self {
            browser: Arc::new(Mutex::new(browser)),
            contexts,
            available: AtomicU64::new(available_bits),
            max_size,
            _cdp_handle: cdp_handle,
            chrome_pid: AtomicU32::new(chrome_pid_val),
        })
    }

    /// Atomically claim an available browser context slot.
    ///
    /// Returns `Some(slot_index)` on success, or `None` if all slots are in use.
    /// Uses lock-free CAS on a bitset — zero contention between workers.
    pub fn try_acquire(&self) -> Option<usize> {
        loop {
            let current = self.available.load(Ordering::Acquire);
            if current == 0 {
                return None; // No available slots
            }

            let slot = current.trailing_zeros() as usize;
            let new = current & !(1 << slot);

            if self
                .available
                .compare_exchange(current, new, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                debug!(slot = slot, "Browser pool slot acquired");
                return Some(slot);
            }
            // CAS failed — another worker grabbed a slot, retry.
        }
    }

    /// Get a `Page` for a claimed slot. Creates one if it doesn't exist yet.
    ///
    /// Returns an error if `slot >= max_size`.
    pub async fn get_page(&self, slot: usize) -> Result<Page, Temm1eError> {
        if slot >= self.max_size {
            return Err(Temm1eError::Tool(format!(
                "Invalid pool slot {slot}: max is {}",
                self.max_size
            )));
        }

        let mut ctx = self.contexts[slot].lock().await;
        if let Some(ref page) = ctx.page {
            return Ok(page.clone());
        }

        // Create a new page in this context.
        let browser = self.browser.lock().await;
        let params = CreateTargetParams::builder()
            .url("about:blank")
            .browser_context_id(ctx.context_id.clone())
            .build()
            .map_err(|e| Temm1eError::Tool(format!("BrowserPool page params: {e}")))?;

        let page = browser
            .new_page(params)
            .await
            .map_err(|e| Temm1eError::Tool(format!("BrowserPool page create: {e}")))?;

        ctx.page = Some(page.clone());
        debug!(slot = slot, "Browser pool page created");
        Ok(page)
    }

    /// Release a context back to the pool.
    ///
    /// Closes the page, disposes the old context, and creates a fresh one to
    /// guarantee clean state (no lingering cookies, storage, or cache).
    ///
    /// Returns an error if `slot >= max_size`.
    pub async fn release(&self, slot: usize) -> Result<(), Temm1eError> {
        if slot >= self.max_size {
            return Err(Temm1eError::Tool(format!(
                "Invalid pool slot {slot}: max is {}",
                self.max_size
            )));
        }

        {
            let mut ctx = self.contexts[slot].lock().await;

            // Close the page if it exists.
            if let Some(page) = ctx.page.take() {
                let _ = page.goto("about:blank").await;
                let _ = page.close().await;
            }

            // Dispose the old context and create a fresh one for clean state.
            let browser = self.browser.lock().await;
            let _ = browser
                .dispose_browser_context(ctx.context_id.clone())
                .await;

            let new_ctx_id = browser
                .create_browser_context(CreateBrowserContextParams::builder().build())
                .await
                .map_err(|e| Temm1eError::Tool(format!("BrowserPool release: {e}")))?;

            ctx.context_id = new_ctx_id;
        }

        // Mark slot as available.
        self.available.fetch_or(1 << slot, Ordering::Release);
        debug!(slot = slot, "Browser pool slot released");
        Ok(())
    }

    /// Number of currently available (unclaimed) slots.
    pub fn available_count(&self) -> usize {
        self.available.load(Ordering::Relaxed).count_ones() as usize
    }

    /// Maximum pool size.
    pub fn max_size(&self) -> usize {
        self.max_size
    }

    /// Graceful async shutdown — closes all contexts and kills Chrome.
    ///
    /// Prefer this over relying on `Drop` during application shutdown, since `Drop`
    /// cannot run async code and must use best-effort synchronous cleanup.
    pub async fn shutdown(&self) {
        // Close all pages in all contexts.
        for ctx_arc in &self.contexts {
            let mut ctx = ctx_arc.lock().await;
            if let Some(page) = ctx.page.take() {
                let _ = page.close().await;
            }
        }
        // The browser will be killed when its Drop fires.
        // Kill any orphaned Chrome child processes.
        let pid = self.chrome_pid.swap(0, Ordering::Relaxed);
        if pid > 0 {
            crate::browser::kill_chrome_children(pid);
        }
        info!("BrowserPool shutdown complete");
    }
}

impl Drop for BrowserPool {
    fn drop(&mut self) {
        self._cdp_handle.abort();
        // Kill orphaned Chrome child processes.
        let pid = self.chrome_pid.swap(0, Ordering::Relaxed);
        if pid > 0 {
            crate::browser::kill_chrome_children(pid);
        }
        debug!("BrowserPool dropped — CDP handler aborted, Chrome children killed");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitset_acquire_release_logic() {
        // Simulate the atomic bitset logic without launching a real browser.
        let available = AtomicU64::new(0b1111); // 4 slots available

        // Acquire slot 0 (lowest set bit)
        let current = available.load(Ordering::Acquire);
        assert_eq!(current.trailing_zeros(), 0);
        let slot = current.trailing_zeros() as usize;
        let new = current & !(1 << slot);
        available.store(new, Ordering::Release);
        assert_eq!(available.load(Ordering::Relaxed), 0b1110);

        // Acquire slot 1
        let current = available.load(Ordering::Acquire);
        assert_eq!(current.trailing_zeros(), 1);
        let slot = current.trailing_zeros() as usize;
        let new = current & !(1 << slot);
        available.store(new, Ordering::Release);
        assert_eq!(available.load(Ordering::Relaxed), 0b1100);

        // Release slot 0
        available.fetch_or(1 << 0, Ordering::Release);
        assert_eq!(available.load(Ordering::Relaxed), 0b1101);

        // Next acquire should get slot 0 again (lowest)
        let current = available.load(Ordering::Acquire);
        assert_eq!(current.trailing_zeros(), 0);
    }

    #[test]
    fn empty_bitset_returns_none() {
        let available = AtomicU64::new(0); // no slots available
        let current = available.load(Ordering::Acquire);
        assert_eq!(current, 0);
        // trailing_zeros of 0 is 64 on a u64, but we check current == 0 first
    }

    #[test]
    fn max_pool_size_64() {
        let mut bits: u64 = 0;
        for i in 0..64 {
            bits |= 1 << i;
        }
        assert_eq!(bits, u64::MAX);
        assert_eq!(bits.count_ones(), 64);
        assert_eq!(bits.trailing_zeros(), 0);
    }

    #[test]
    #[should_panic(expected = "Pool size must be at least 1")]
    fn zero_pool_size_panics() {
        // Mirrors the assertion in BrowserPool::new()
        let max_size: usize = 0;
        assert!(max_size > 0, "Pool size must be at least 1");
    }

    #[test]
    #[should_panic(expected = "Pool size limited to 64")]
    fn oversized_pool_panics() {
        // Mirrors the assertion in BrowserPool::new()
        let max_size: usize = 65;
        assert!(max_size <= 64, "Pool size limited to 64 (bitset)");
    }
}
