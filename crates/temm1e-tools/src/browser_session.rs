//! Interactive browser session for OTK (One-Time Key) credential capture.
//!
//! Provides a screenshot-based interactive flow where the user interacts with
//! a browser page through numbered element annotations. The user sees annotated
//! screenshots via their messaging channel and replies with numbers to click
//! elements or text to type into focused fields.
//!
//! ## Security Properties
//!
//! - **Credential non-transit:** User types credentials in chat; they flow
//!   directly into `Input.insertText` via CDP. The LLM never sees them.
//! - **Zeroize on drop:** All text input is wrapped in `Zeroizing<String>` so
//!   passwords are zeroed from memory after injection.
//! - **Encrypted at rest:** Captured session state (cookies + storage) is
//!   encrypted via the Vault trait (ChaCha20-Poly1305).
//!
//! ## Flow
//!
//! 1. `InteractiveBrowseSession::new()` — launches page, navigates to URL
//! 2. `capture_annotated()` — get AX tree, inject JS overlays, screenshot, remove overlays
//! 3. `handle_input()` — user sends number (click) or text (type) or "done"
//! 4. `capture_session()` — extract cookies + storage, encrypt, store in vault

use std::collections::HashMap;
use std::fmt::Write as _;

use chromiumoxide::browser::Browser;
// CDP Accessibility API removed — using JS-based extraction instead (chromiumoxide 0.7 compat)
use chromiumoxide::cdp::browser_protocol::dom::BackendNodeId;
use chromiumoxide::cdp::browser_protocol::dom_storage::{GetDomStorageItemsParams, StorageId};
use chromiumoxide::cdp::browser_protocol::network::{
    CookieParam, CookieSameSite, GetCookiesParams, SetCookiesParams, TimeSinceEpoch,
};
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::page::Page;
use serde::{Deserialize, Serialize};
use temm1e_core::types::error::Temm1eError;
use temm1e_core::Vault;
use zeroize::Zeroizing;

// ── Constants ────────────────────────────────────────────────────────

/// Roles considered interactive for the numbered overlay annotation.
#[cfg(test)]
const SESSION_INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "link",
    "textbox",
    "combobox",
    "checkbox",
    "radio",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "menuitem",
    "option",
    "searchbox",
    "textarea",
];

/// JavaScript template to inject a single numbered overlay label at a given position.
/// Parameters: index (number label), x (px), y (px).
const OVERLAY_INJECT_JS: &str = r#"
(() => {
    const label = document.createElement('div');
    label.className = 'prowl-overlay-label';
    label.textContent = '{INDEX}';
    label.style.cssText = `
        position: fixed;
        left: {X}px;
        top: {Y}px;
        width: 22px;
        height: 22px;
        background: #e53e3e;
        color: white;
        font-size: 12px;
        font-weight: bold;
        line-height: 22px;
        text-align: center;
        border-radius: 50%;
        z-index: 2147483647;
        pointer-events: none;
        box-shadow: 0 1px 3px rgba(0,0,0,0.4);
        font-family: Arial, sans-serif;
    `;
    document.body.appendChild(label);
})();
"#;

/// JavaScript to remove all overlay labels injected by this module.
const OVERLAY_REMOVE_JS: &str = r#"
document.querySelectorAll('.prowl-overlay-label').forEach(el => el.remove());
"#;

// ── Types ────────────────────────────────────────────────────────────

/// Result of a user input action during the interactive session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionAction {
    /// Take a new screenshot and send to user.
    Continue,
    /// User is done — capture session, close browser.
    Done,
}

/// Captured web session state — cookies and storage, encrypted at rest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    /// CDP cookie objects serialized as JSON values.
    pub cookies: Vec<serde_json::Value>,
    /// localStorage key-value pairs.
    pub local_storage: Vec<Vec<String>>,
    /// sessionStorage key-value pairs.
    pub session_storage: Vec<Vec<String>>,
    /// URL at the time of capture.
    pub url: String,
    /// ISO 8601 timestamp of capture.
    pub captured_at: String,
    /// Service name (e.g., "amazon", "github").
    pub service: String,
}

/// Interactive browser session for OTK credential capture.
///
/// Manages a browser page with numbered element annotations. The user
/// interacts via screenshot-based commands (number to click, text to type,
/// "done" to finish).
pub struct InteractiveBrowseSession {
    page: Page,
    session_id: String,
    service: String,
    /// Maps overlay index (1-based) to the AX node's `BackendNodeId`.
    element_map: HashMap<usize, BackendNodeId>,
    /// Maps overlay index to click coordinates (center x, center y).
    click_coords: HashMap<usize, (i32, i32)>,
    /// Hold the Browser to prevent Chrome from being killed when dropped.
    _browser: Option<Browser>,
    _profile: Option<crate::browser_profile::BrowserProfile>,
    cdp_handle: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for InteractiveBrowseSession {
    fn drop(&mut self) {
        if let Some(handle) = self.cdp_handle.take() {
            handle.abort();
        }
        // Browser owns its child; the profile is removed only within Tem's directory.
        self._browser.take();
    }
}

impl InteractiveBrowseSession {
    /// Create a new interactive session, navigating to the given URL.
    ///
    /// The browser should already be launched (via `BrowserTool::ensure_browser`
    /// or directly via `Browser::launch`). This creates a new page and navigates.
    pub async fn new(browser: &Browser, service: &str, url: &str) -> Result<Self, Temm1eError> {
        let page = browser
            .new_page(url)
            .await
            .map_err(|e| Temm1eError::Tool(format!("Failed to create page for session: {}", e)))?;

        // Wait for page to load
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let session_id = format!("{}-{}", chrono::Utc::now().timestamp_millis(), service);

        tracing::info!(
            session_id = %session_id,
            service = %service,
            url = %url,
            "Interactive browse session created"
        );

        Ok(Self {
            page,
            session_id,
            service: service.to_string(),
            element_map: HashMap::new(),
            click_coords: HashMap::new(),
            _browser: None,
            _profile: None,
            cdp_handle: None,
        })
    }

    /// Convenience: launch a fresh browser and create a session in one call.
    /// This handles Chrome launch with stealth flags so callers don't need chromiumoxide.
    pub async fn launch(service: &str, url: &str) -> Result<Self, Temm1eError> {
        use chromiumoxide::browser::{Browser, BrowserConfig};
        use futures::StreamExt;

        // Headed with headless fallback (same logic as BrowserTool)
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
        builder = builder
            .arg("--disable-gpu")
            .arg("--no-sandbox")
            .arg("--disable-dev-shm-usage")
            .arg("--disable-blink-features=AutomationControlled")
            .arg("--window-size=1280,900");

        let profile = crate::browser_profile::BrowserProfile::create("login", None).await?;
        builder = builder
            .user_data_dir(profile.path())
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disk-cache-size=67108864")
            .arg("--media-cache-size=16777216");

        let config = builder
            .build()
            .map_err(|e| Temm1eError::Tool(format!("Browser config: {}", e)))?;

        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| Temm1eError::Tool(format!("Browser launch: {}", e)))?;

        // CDP handler — continue on WS errors (chromiumoxide 0.7 compat)
        let cdp_handle = tokio::spawn(async move { while handler.next().await.is_some() {} });
        let mut session = match Self::new(&browser, service, url).await {
            Ok(session) => session,
            Err(error) => {
                cdp_handle.abort();
                return Err(error);
            }
        };
        session._browser = Some(browser);
        session._profile = Some(profile);
        session.cdp_handle = Some(cdp_handle);
        Ok(session)
    }

    /// Get the session ID.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Get the service name.
    pub fn service(&self) -> &str {
        &self.service
    }

    /// Take an annotated screenshot with numbered overlay labels on interactive elements.
    ///
    /// Returns `(png_bytes, text_description)` where `text_description` is a
    /// human-readable list of numbered elements (e.g., `[1] textbox "Email"`).
    ///
    /// The process:
    /// 1. Get the full accessibility tree
    /// 2. Identify interactive elements with backend DOM nodes
    /// 3. Get bounding boxes via `DOM.getBoxModel`
    /// 4. Inject numbered overlay labels via JavaScript
    /// 5. Take a screenshot
    /// 6. Remove the overlay labels
    pub async fn capture_annotated(&mut self) -> Result<(Vec<u8>, String), Temm1eError> {
        // Use JS-based element discovery (CDP Accessibility API has deserialization
        // issues with chromiumoxide 0.7 on newer Chrome versions)
        let js = r#"(() => {
            const results = [];
            let idx = 1;
            const interactiveTags = ['a','button','input','select','textarea'];
            const interactiveRoles = ['button','link','textbox','combobox','checkbox','radio','tab','menuitem','searchbox','slider','switch'];
            // Skip elements inside footer/nav noise
            const isInNoise = (el) => {
                let p = el.parentElement;
                while (p && p !== document.body) {
                    const ptag = p.tagName.toLowerCase();
                    const prole = (p.getAttribute('role') || '').toLowerCase();
                    if (ptag === 'footer' || prole === 'contentinfo') return true;
                    p = p.parentElement;
                }
                return false;
            };
            const walk = (el) => {
                if (!el || el.nodeType !== 1) return;
                const tag = el.tagName.toLowerCase();
                const role = el.getAttribute('role') || '';
                const type = el.getAttribute('type') || '';
                const isInteractive = interactiveTags.includes(tag)
                    || interactiveRoles.includes(role);
                if (isInteractive) {
                    // Skip hidden/invisible elements
                    const rect = el.getBoundingClientRect();
                    if (rect.width === 0 || rect.height === 0) { for (const c of el.children) walk(c); return; }
                    if (tag === 'input' && type === 'hidden') { for (const c of el.children) walk(c); return; }
                    // Categorize: primary (form fields, main buttons) vs secondary (footer links)
                    const isPrimary = ['input','textarea','select'].includes(tag)
                        || tag === 'button'
                        || role === 'button'
                        || (tag === 'a' && !isInNoise(el));
                    // Skip footer links entirely for cleaner UX
                    if (tag === 'a' && isInNoise(el)) { return; }
                    const ariaLabel = el.getAttribute('aria-label') || '';
                    let label = ariaLabel || el.title || '';
                    if (!label && tag === 'a') label = (el.textContent || '').trim().substring(0,60);
                    if (!label && tag === 'button') label = (el.textContent || '').trim().substring(0,60);
                    if (!label && ['input','textarea','select'].includes(tag)) {
                        const id = el.id;
                        if (id) { const lbl = document.querySelector('label[for="'+id+'"]'); if (lbl) label = lbl.textContent.trim(); }
                        if (!label) label = el.placeholder || el.name || '';
                    }
                    // Build user-friendly description
                    let desc = '';
                    if (['input','textarea'].includes(tag)) {
                        if (type === 'password') desc = '🔒 Password field';
                        else if (type === 'email') desc = '📧 Email field';
                        else if (type === 'submit') desc = '➡️ Submit';
                        else desc = '✏️ Text field';
                        if (label) desc += ': ' + label;
                    } else if (tag === 'button' || role === 'button') {
                        desc = '🔘 ' + (label || 'Button');
                    } else if (tag === 'a') {
                        desc = '🔗 ' + (label || 'Link');
                    } else if (tag === 'select') {
                        desc = '📋 Dropdown: ' + (label || '');
                    } else {
                        desc = (role || tag) + (label ? ': ' + label : '');
                    }
                    results.push({idx: idx, desc: desc, x: Math.round(rect.x), y: Math.round(rect.y), w: Math.round(rect.width), h: Math.round(rect.height), primary: isPrimary});
                    idx++;
                }
                for (const child of el.children) walk(child);
            };
            walk(document.body);
            return JSON.stringify(results);
        })()"#;

        let js_result = self
            .page
            .evaluate(js)
            .await
            .map_err(|e| Temm1eError::Tool(format!("Session element scan failed: {}", e)))?;

        let json_str = js_result
            .into_value::<String>()
            .unwrap_or_else(|_| "[]".to_string());

        #[derive(serde::Deserialize)]
        struct JsElement {
            idx: usize,
            desc: String,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            #[serde(default)]
            _primary: bool,
        }

        let elements: Vec<JsElement> = serde_json::from_str(&json_str).unwrap_or_default();

        // Build description — show all elements with clear formatting
        self.element_map.clear();
        let mut description = String::new();

        for el in &elements {
            let _ = writeln!(&mut description, "[{}] {}", el.idx, el.desc);
        }

        // Add usage hint
        let _ = writeln!(&mut description);
        let _ = writeln!(&mut description, "💡 Commands:");
        let _ = writeln!(
            &mut description,
            "  1 your@email.com  → select field & type"
        );
        let _ = writeln!(&mut description, "  3                 → click button");
        let _ = writeln!(&mut description, "  done              → save & exit");

        self.click_coords.clear();
        for el in &elements {
            let cx = el.x + el.w / 2;
            let cy = el.y + el.h / 2;
            self.click_coords.insert(el.idx, (cx, cy));
        }

        // 3. Inject overlay labels using JS-extracted coordinates
        for el in &elements {
            let label_x = (el.x - 12).max(0);
            let label_y = (el.y - 12).max(0);

            let js = OVERLAY_INJECT_JS
                .replace("{INDEX}", &el.idx.to_string())
                .replace("{X}", &label_x.to_string())
                .replace("{Y}", &label_y.to_string());

            if let Err(e) = self.page.evaluate(js).await {
                tracing::debug!(
                    index = el.idx,
                    error = %e,
                    "Failed to inject overlay label — skipping"
                );
            }
        }

        // Small delay to let overlays render
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // 4. Take screenshot
        let png_data = self
            .page
            .screenshot(
                chromiumoxide::page::ScreenshotParams::builder()
                    .format(CaptureScreenshotFormat::Png)
                    .build(),
            )
            .await
            .map_err(|e| Temm1eError::Tool(format!("Session screenshot failed: {}", e)))?;

        // 5. Remove overlay labels
        if let Err(e) = self.page.evaluate(OVERLAY_REMOVE_JS).await {
            tracing::debug!(error = %e, "Failed to remove overlay labels");
        }

        // 6. QR code detection — if found, prepend a prominent note so the
        //    user knows to scan the QR code visible in the screenshot above.
        let qr_detected = match self.page.evaluate(crate::browser::QR_DETECT_JS).await {
            Ok(result) => {
                let detection = result
                    .into_value::<String>()
                    .unwrap_or_else(|_| "no_qr".to_string());
                detection != "no_qr"
            }
            Err(_) => false,
        };

        if qr_detected {
            let qr_note = "\u{1F4F1} QR code detected! Scan the image above to log in.\n\n";
            description.insert_str(0, qr_note);
            tracing::info!(
                session_id = %self.session_id,
                "QR code detected during interactive session"
            );
        }

        if description.is_empty() {
            description = "(no interactive elements found on this page)".to_string();
        }

        tracing::debug!(
            session_id = %self.session_id,
            elements = self.element_map.len(),
            screenshot_bytes = png_data.len(),
            "Annotated screenshot captured"
        );

        Ok((png_data, description))
    }

    /// Handle user input during the interactive session.
    ///
    /// Supported formats:
    /// - **"done"**: Signal session completion — caller should call `capture_session()`.
    /// - **"3"**: Click element [3] (button, link).
    /// - **"1 myemail@gmail.com"**: Click element [1] then type text into it.
    /// - **"mytext"**: Type into the currently focused element.
    pub async fn handle_input(&mut self, input: &str) -> Result<SessionAction, Temm1eError> {
        let trimmed = input.trim();

        if trimmed.eq_ignore_ascii_case("done") {
            return Ok(SessionAction::Done);
        }

        // Parse input: "3" = click only, "1 mytext" = click then type
        let (num_part, text_part) = if let Some(space_idx) = trimmed.find(' ') {
            let left = &trimmed[..space_idx];
            if let Ok(n) = left.parse::<usize>() {
                (Some(n), Some(trimmed[space_idx + 1..].trim()))
            } else {
                (None, None)
            }
        } else if let Ok(n) = trimmed.parse::<usize>() {
            (Some(n), None)
        } else {
            (None, None)
        };

        // Handle number (click) + optional text (type)
        if let Some(num) = num_part {
            if self.click_coords.contains_key(&num) {
                // Click via JS — find the nth interactive element and click it
                let click_js = format!(
                    r#"(() => {{
                        const interactiveTags = ['a','button','input','select','textarea'];
                        const interactiveRoles = ['button','link','textbox','combobox','checkbox','radio','tab','menuitem','searchbox','slider','switch'];
                        let idx = 0;
                        const walk = (el) => {{
                            if (!el || el.nodeType !== 1) return null;
                            const tag = el.tagName.toLowerCase();
                            const role = el.getAttribute('role') || '';
                            if (interactiveTags.includes(tag) || interactiveRoles.includes(role)) {{
                                idx++;
                                if (idx === {}) {{
                                    el.scrollIntoView({{block:'center'}});
                                    el.focus();
                                    el.click();
                                    // For buttons/submit: also try form submission and Enter key
                                    if (tag === 'button' || (tag === 'input' && (el.type === 'submit' || el.type === 'button'))) {{
                                        // Dispatch Enter key on the form or focused input
                                        const form = el.closest('form');
                                        if (form) {{
                                            // Try submitting via Enter key on the last input
                                            const inputs = form.querySelectorAll('input[type="text"],input[type="password"],input[type="email"]');
                                            const lastInput = inputs[inputs.length - 1];
                                            if (lastInput) {{
                                                lastInput.focus();
                                                lastInput.dispatchEvent(new KeyboardEvent('keydown', {{key:'Enter',code:'Enter',keyCode:13,which:13,bubbles:true}}));
                                                lastInput.dispatchEvent(new KeyboardEvent('keypress', {{key:'Enter',code:'Enter',keyCode:13,which:13,bubbles:true}}));
                                                lastInput.dispatchEvent(new KeyboardEvent('keyup', {{key:'Enter',code:'Enter',keyCode:13,which:13,bubbles:true}}));
                                            }}
                                            // Also try direct form submit as fallback
                                            try {{ form.submit(); }} catch(e) {{}}
                                        }}
                                    }}
                                    return 'clicked';
                                }}
                            }}
                            for (const child of el.children) {{
                                const r = walk(child);
                                if (r) return r;
                            }}
                            return null;
                        }};
                        return walk(document.body) || 'not_found';
                    }})()"#,
                    num
                );

                let result = self
                    .page
                    .evaluate(click_js)
                    .await
                    .map_err(|e| Temm1eError::Tool(format!("Click element: {}", e)))?;

                let status = result.into_value::<String>().unwrap_or_default();
                if status != "clicked" {
                    return Err(Temm1eError::Tool(format!(
                        "Could not click element [{}]",
                        num
                    )));
                }

                // If text was provided, type it into the now-focused element
                if let Some(text) = text_part {
                    // Short wait for focus to take effect after click
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

                    let zeroizing = zeroize::Zeroizing::new(text.to_string());
                    // Type via CDP insertText
                    use chromiumoxide::cdp::browser_protocol::input::DispatchKeyEventParams;
                    use chromiumoxide::cdp::browser_protocol::input::DispatchKeyEventType;
                    for ch in zeroizing.chars() {
                        let _ = self
                            .page
                            .execute(
                                DispatchKeyEventParams::builder()
                                    .r#type(DispatchKeyEventType::KeyDown)
                                    .text(ch.to_string())
                                    .build()
                                    .unwrap(),
                            )
                            .await;
                        let _ = self
                            .page
                            .execute(
                                DispatchKeyEventParams::builder()
                                    .r#type(DispatchKeyEventType::KeyUp)
                                    .build()
                                    .unwrap(),
                            )
                            .await;
                    }
                    tracing::debug!(
                        session_id = %self.session_id,
                        element = num,
                        text_len = text.len(),
                        "Clicked element and typed text"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                } else {
                    // Button click only — wait longer for potential navigation
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    tracing::debug!(
                        session_id = %self.session_id,
                        element = num,
                        "Clicked element via JS"
                    );
                }
                return Ok(SessionAction::Continue);
            }
            return Err(Temm1eError::Tool(format!(
                "Element [{}] not found on current page",
                num
            )));
        }

        // Text input — type into currently focused element
        // Wrap in Zeroizing in case it's a password
        let zeroizing_input = Zeroizing::new(trimmed.to_string());

        // Check if there's a focused element (not body)
        let focus_check: String = self
            .page
            .evaluate(
                "document.activeElement && document.activeElement.tagName !== 'BODY' ? 'has_focus' : 'no_focus'"
            )
            .await
            .map_err(|e| Temm1eError::Tool(format!("Focus check failed: {}", e)))?
            .into_value()
            .unwrap_or_else(|_| "no_focus".to_string());

        if focus_check == "has_focus" {
            // Use Input.insertText for the whole string — this is the cleanest CDP method
            // for injecting text into a focused input field.
            use chromiumoxide::cdp::browser_protocol::input::InsertTextParams;
            self.page
                .execute(InsertTextParams::new(zeroizing_input.as_str()))
                .await
                .map_err(|e| Temm1eError::Tool(format!("Text insertion failed: {}", e)))?;

            // zeroizing_input drops here -> zeros the typed text from memory
            tracing::debug!(
                session_id = %self.session_id,
                chars = trimmed.len(),
                "Typed text into focused element"
            );
            return Ok(SessionAction::Continue);
        }

        Err(Temm1eError::Tool(
            "No element is focused. Tap a number first to select an element.".into(),
        ))
    }

    /// Capture the current session state (cookies + storage) and encrypt to vault.
    ///
    /// Extracts:
    /// - All browser cookies via `Network.getCookies`
    /// - localStorage items via `DOMStorage.getDOMStorageItems`
    /// - sessionStorage items via `DOMStorage.getDOMStorageItems`
    ///
    /// The state is serialized to JSON and stored in the vault under
    /// `web_session:{service}`.
    pub async fn capture_session(&self, vault: &dyn Vault) -> Result<(), Temm1eError> {
        // ── Cookies via CDP ─────────────────────────────────────────
        let cookies_response = self
            .page
            .execute(GetCookiesParams::default())
            .await
            .map_err(|e| Temm1eError::Tool(format!("Session: get cookies failed: {}", e)))?;

        // Serialize cookies as JSON values since the CDP Cookie type has
        // complex fields (CookiePriority, CookieSourceScheme) that may not
        // roundtrip cleanly through our own struct.
        let cookies: Vec<serde_json::Value> = cookies_response
            .result
            .cookies
            .iter()
            .filter_map(|c| serde_json::to_value(c).ok())
            .collect();

        // ── Storage via CDP ─────────────────────────────────────────
        let origin = self.get_page_origin().await;

        let local_storage = if !origin.is_empty() {
            self.get_storage_items(&origin, true).await
        } else {
            Vec::new()
        };

        let session_storage = if !origin.is_empty() {
            self.get_storage_items(&origin, false).await
        } else {
            Vec::new()
        };

        // ── Build and store session state ───────────────────────────
        let current_url = self.page.url().await.ok().flatten().unwrap_or_default();

        let state = SessionState {
            cookies,
            local_storage,
            session_storage,
            url: current_url,
            captured_at: chrono::Utc::now().to_rfc3339(),
            service: self.service.clone(),
        };

        let json = serde_json::to_vec(&state)
            .map_err(|e| Temm1eError::Tool(format!("Session state serialization failed: {}", e)))?;

        let vault_key = format!("web_session:{}", self.service);
        vault.store_secret(&vault_key, &json).await?;

        tracing::info!(
            session_id = %self.session_id,
            service = %self.service,
            cookie_count = cookies_response.result.cookies.len(),
            local_storage_items = state.local_storage.len(),
            session_storage_items = state.session_storage.len(),
            "Session state captured and encrypted to vault"
        );

        Ok(())
    }

    /// Get the current page's origin (scheme + host).
    async fn get_page_origin(&self) -> String {
        self.page
            .url()
            .await
            .ok()
            .flatten()
            .and_then(|u| extract_origin(&u))
            .unwrap_or_default()
    }

    /// Get storage items (local or session) for the given origin.
    async fn get_storage_items(&self, origin: &str, is_local: bool) -> Vec<Vec<String>> {
        let storage_id = StorageId {
            security_origin: Some(origin.to_string()),
            storage_key: None,
            is_local_storage: is_local,
        };

        match self
            .page
            .execute(GetDomStorageItemsParams::new(storage_id))
            .await
        {
            Ok(result) => result
                .result
                .entries
                .into_iter()
                .map(|item| item.inner().clone())
                .collect(),
            Err(e) => {
                let kind = if is_local { "local" } else { "session" };
                tracing::debug!(
                    origin = %origin,
                    storage = kind,
                    error = %e,
                    "Failed to get {} storage items — skipping",
                    kind
                );
                Vec::new()
            }
        }
    }
}

// ── Session Restore ──────────────────────────────────────────────────

/// Restore a previously captured web session from the vault.
///
/// Loads cookies and storage from vault, sets them via CDP, navigates to the
/// saved URL, and checks the AX tree for login prompts to detect expiration.
///
/// Returns the formatted accessibility tree and a boolean indicating whether
/// the session appears to still be alive.
pub async fn restore_web_session(
    page: &Page,
    vault: &dyn Vault,
    service: &str,
) -> Result<(String, bool), Temm1eError> {
    let vault_key = format!("web_session:{}", service);
    let raw_bytes = vault
        .get_secret(&vault_key)
        .await?
        .ok_or_else(|| Temm1eError::Tool(format!("No saved session for '{}'", service)))?;

    let state: SessionState = serde_json::from_slice(&raw_bytes)
        .map_err(|e| Temm1eError::Tool(format!("Session state parse error: {}", e)))?;

    // ── Restore cookies ─────────────────────────────────────────────
    let cookie_params: Vec<CookieParam> = state
        .cookies
        .iter()
        .filter_map(|cv| {
            let name = cv.get("name")?.as_str()?;
            let value = cv.get("value")?.as_str()?;
            let mut param = CookieParam::new(name.to_string(), value.to_string());

            if let Some(domain) = cv.get("domain").and_then(|v| v.as_str()) {
                param.domain = Some(domain.to_string());
            }
            if let Some(path) = cv.get("path").and_then(|v| v.as_str()) {
                param.path = Some(path.to_string());
            }
            if let Some(expires) = cv.get("expires").and_then(|v| v.as_f64()) {
                param.expires = Some(TimeSinceEpoch::new(expires));
            }
            if let Some(http_only) = cv.get("httpOnly").and_then(|v| v.as_bool()) {
                param.http_only = Some(http_only);
            }
            if let Some(secure) = cv.get("secure").and_then(|v| v.as_bool()) {
                param.secure = Some(secure);
            }
            if let Some(ss) = cv.get("sameSite").and_then(|v| v.as_str()) {
                if let Ok(parsed) = ss.parse::<CookieSameSite>() {
                    param.same_site = Some(parsed);
                }
            }

            Some(param)
        })
        .collect();

    let cookie_count = cookie_params.len();
    if !cookie_params.is_empty() {
        page.execute(SetCookiesParams::new(cookie_params))
            .await
            .map_err(|e| {
                Temm1eError::Tool(format!("Session restore: set cookies failed: {}", e))
            })?;
    }

    // ── Restore localStorage ────────────────────────────────────────
    if !state.local_storage.is_empty() {
        let origin = extract_origin(&state.url).unwrap_or_default();

        if !origin.is_empty() {
            let storage_id = StorageId {
                security_origin: Some(origin.clone()),
                storage_key: None,
                is_local_storage: true,
            };

            for entry in &state.local_storage {
                if entry.len() >= 2 {
                    use chromiumoxide::cdp::browser_protocol::dom_storage::SetDomStorageItemParams;
                    if let Err(e) = page
                        .execute(SetDomStorageItemParams::new(
                            storage_id.clone(),
                            entry[0].clone(),
                            entry[1].clone(),
                        ))
                        .await
                    {
                        tracing::debug!(
                            key = %entry[0],
                            error = %e,
                            "Failed to restore localStorage item — skipping"
                        );
                    }
                }
            }
        }
    }

    // ── Navigate to saved URL ───────────────────────────────────────
    page.goto(&state.url)
        .await
        .map_err(|e| Temm1eError::Tool(format!("Session restore: navigation failed: {}", e)))?;

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // ── Verify session is alive (use JS since CDP AX tree has deserialization issues) ──
    // Check for login FORMS (password input fields), not just text —
    // many sites show "Log In" text in navbar/footer even when authenticated
    let verify_js = r#"(() => {
        const hasPasswordField = document.querySelector('input[type="password"]') !== null;
        const hasLoginForm = document.querySelector('form input[type="password"]') !== null;
        const url = window.location.href.toLowerCase();
        const onLoginPage = url.includes('/login') || url.includes('/signin') || url.includes('/sign-in');
        const isLoginPage = hasPasswordField && (hasLoginForm || onLoginPage);
        return JSON.stringify({ hasLogin: isLoginPage, title: document.title || '', url: window.location.href });
    })()"#;

    let verify_result = page
        .evaluate(verify_js)
        .await
        .map_err(|e| Temm1eError::Tool(format!("Session verify failed: {}", e)))?;

    let verify_json = verify_result
        .into_value::<String>()
        .unwrap_or_else(|_| r#"{"hasLogin":true,"title":"","url":""}"#.to_string());

    #[derive(serde::Deserialize)]
    struct VerifyResult {
        #[serde(rename = "hasLogin")]
        has_login: bool,
        title: String,
        url: String,
    }

    let verify: VerifyResult = serde_json::from_str(&verify_json).unwrap_or(VerifyResult {
        has_login: true,
        title: String::new(),
        url: String::new(),
    });

    let tree_text = format!("Page: {} ({})", verify.title, verify.url);
    let has_login_prompt = verify.has_login;

    let session_alive = !has_login_prompt;

    tracing::info!(
        service = %service,
        cookies_restored = cookie_count,
        local_storage_items = state.local_storage.len(),
        session_alive = session_alive,
        "Web session restore completed"
    );

    Ok((tree_text, session_alive))
}

/// Extract the origin (scheme + host) from a URL string.
///
/// Returns e.g. `"https://www.example.com"` from `"https://www.example.com/path?query"`.
/// Uses simple string splitting to avoid a dependency on the `url` crate.
fn extract_origin(url_str: &str) -> Option<String> {
    // Find "://" to split scheme from the rest
    let scheme_end = url_str.find("://")?;
    let scheme = &url_str[..scheme_end];
    let after_scheme = &url_str[scheme_end + 3..];
    // Host ends at the first '/' or '?' or '#', or end of string
    let host_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let host = &after_scheme[..host_end];
    if host.is_empty() {
        return None;
    }
    Some(format!("{}://{}", scheme, host))
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── SessionAction tests ──────────────────────────────────────────

    #[test]
    fn session_action_continue_eq() {
        assert_eq!(SessionAction::Continue, SessionAction::Continue);
    }

    #[test]
    fn session_action_done_eq() {
        assert_eq!(SessionAction::Done, SessionAction::Done);
    }

    #[test]
    fn session_action_variants_differ() {
        assert_ne!(SessionAction::Continue, SessionAction::Done);
    }

    // ── SessionState serialization tests ─────────────────────────────

    #[test]
    fn session_state_serialization_roundtrip() {
        let state = SessionState {
            cookies: vec![serde_json::json!({
                "name": "session_id",
                "value": "abc123",
                "domain": ".example.com"
            })],
            local_storage: vec![vec!["key1".into(), "val1".into()]],
            session_storage: vec![vec!["skey".into(), "sval".into()]],
            url: "https://example.com/dashboard".to_string(),
            captured_at: "2026-03-19T12:00:00Z".to_string(),
            service: "example".to_string(),
        };

        let json = serde_json::to_vec(&state).unwrap();
        let restored: SessionState = serde_json::from_slice(&json).unwrap();

        assert_eq!(restored.service, "example");
        assert_eq!(restored.url, "https://example.com/dashboard");
        assert_eq!(restored.cookies.len(), 1);
        assert_eq!(restored.local_storage.len(), 1);
        assert_eq!(restored.session_storage.len(), 1);
        assert_eq!(restored.captured_at, "2026-03-19T12:00:00Z");
    }

    #[test]
    fn session_state_empty_storage() {
        let state = SessionState {
            cookies: vec![],
            local_storage: vec![],
            session_storage: vec![],
            url: "https://example.com".to_string(),
            captured_at: "2026-03-19T12:00:00Z".to_string(),
            service: "empty".to_string(),
        };

        let json = serde_json::to_string(&state).unwrap();
        let restored: SessionState = serde_json::from_str(&json).unwrap();

        assert!(restored.cookies.is_empty());
        assert!(restored.local_storage.is_empty());
        assert!(restored.session_storage.is_empty());
    }

    #[test]
    fn session_state_multiple_cookies() {
        let state = SessionState {
            cookies: vec![
                serde_json::json!({"name": "a", "value": "1"}),
                serde_json::json!({"name": "b", "value": "2"}),
                serde_json::json!({"name": "c", "value": "3"}),
            ],
            local_storage: vec![],
            session_storage: vec![],
            url: "https://example.com".to_string(),
            captured_at: "2026-03-19T12:00:00Z".to_string(),
            service: "multi".to_string(),
        };

        let json = serde_json::to_vec(&state).unwrap();
        let restored: SessionState = serde_json::from_slice(&json).unwrap();

        assert_eq!(restored.cookies.len(), 3);
        assert_eq!(
            restored.cookies[0].get("name").unwrap().as_str().unwrap(),
            "a"
        );
        assert_eq!(
            restored.cookies[2].get("name").unwrap().as_str().unwrap(),
            "c"
        );
    }

    #[test]
    fn session_state_multiple_storage_entries() {
        let state = SessionState {
            cookies: vec![],
            local_storage: vec![
                vec!["token".into(), "abc".into()],
                vec!["user".into(), "john".into()],
            ],
            session_storage: vec![vec!["csrf".into(), "xyz".into()]],
            url: "https://example.com".to_string(),
            captured_at: "2026-03-19T12:00:00Z".to_string(),
            service: "storage".to_string(),
        };

        let json = serde_json::to_vec(&state).unwrap();
        let restored: SessionState = serde_json::from_slice(&json).unwrap();

        assert_eq!(restored.local_storage.len(), 2);
        assert_eq!(restored.local_storage[0][0], "token");
        assert_eq!(restored.local_storage[0][1], "abc");
        assert_eq!(restored.session_storage.len(), 1);
        assert_eq!(restored.session_storage[0][0], "csrf");
    }

    #[test]
    fn session_state_cookie_with_full_cdp_fields() {
        // Simulate a real CDP cookie object with all fields
        let state = SessionState {
            cookies: vec![serde_json::json!({
                "name": "sid",
                "value": "encrypted_value",
                "domain": ".amazon.com",
                "path": "/",
                "expires": 1800000000.0_f64,
                "size": 42,
                "httpOnly": true,
                "secure": true,
                "session": false,
                "sameSite": "Lax",
                "priority": "Medium",
                "sourceScheme": "Secure",
                "sourcePort": 443
            })],
            local_storage: vec![],
            session_storage: vec![],
            url: "https://www.amazon.com/".to_string(),
            captured_at: "2026-03-19T12:00:00Z".to_string(),
            service: "amazon".to_string(),
        };

        let json = serde_json::to_vec(&state).unwrap();
        let restored: SessionState = serde_json::from_slice(&json).unwrap();

        let cookie = &restored.cookies[0];
        assert_eq!(cookie["name"].as_str().unwrap(), "sid");
        assert_eq!(cookie["domain"].as_str().unwrap(), ".amazon.com");
        assert!(cookie["httpOnly"].as_bool().unwrap());
        assert!(cookie["secure"].as_bool().unwrap());
    }

    // ── Overlay JS template tests ───────────────────────────────────

    #[test]
    fn overlay_inject_js_has_placeholders() {
        assert!(
            OVERLAY_INJECT_JS.contains("{INDEX}"),
            "Inject JS should have INDEX placeholder"
        );
        assert!(
            OVERLAY_INJECT_JS.contains("{X}"),
            "Inject JS should have X placeholder"
        );
        assert!(
            OVERLAY_INJECT_JS.contains("{Y}"),
            "Inject JS should have Y placeholder"
        );
    }

    #[test]
    fn overlay_inject_js_uses_fixed_positioning() {
        assert!(
            OVERLAY_INJECT_JS.contains("position: fixed"),
            "Overlay should use fixed positioning"
        );
    }

    #[test]
    fn overlay_inject_js_has_high_z_index() {
        assert!(
            OVERLAY_INJECT_JS.contains("2147483647"),
            "Overlay z-index should be max int"
        );
    }

    #[test]
    fn overlay_inject_js_has_class_name() {
        assert!(
            OVERLAY_INJECT_JS.contains("prowl-overlay-label"),
            "Overlay should have prowl-overlay-label class"
        );
    }

    #[test]
    fn overlay_remove_js_targets_class() {
        assert!(
            OVERLAY_REMOVE_JS.contains("prowl-overlay-label"),
            "Remove JS should target prowl-overlay-label class"
        );
    }

    #[test]
    fn overlay_remove_js_calls_remove() {
        assert!(
            OVERLAY_REMOVE_JS.contains(".remove()"),
            "Remove JS should call .remove() on elements"
        );
    }

    #[test]
    fn overlay_inject_js_placeholder_replacement() {
        let js = OVERLAY_INJECT_JS
            .replace("{INDEX}", "5")
            .replace("{X}", "100")
            .replace("{Y}", "200");
        assert!(js.contains("label.textContent = '5'"));
        assert!(js.contains("left: 100px"));
        assert!(js.contains("top: 200px"));
        assert!(!js.contains("{INDEX}"));
        assert!(!js.contains("{X}"));
        assert!(!js.contains("{Y}"));
    }

    // ── Interactive roles tests ──────────────────────────────────────

    #[test]
    fn session_interactive_roles_contains_textbox() {
        assert!(SESSION_INTERACTIVE_ROLES.contains(&"textbox"));
    }

    #[test]
    fn session_interactive_roles_contains_button() {
        assert!(SESSION_INTERACTIVE_ROLES.contains(&"button"));
    }

    #[test]
    fn session_interactive_roles_contains_link() {
        assert!(SESSION_INTERACTIVE_ROLES.contains(&"link"));
    }

    #[test]
    fn session_interactive_roles_does_not_contain_heading() {
        assert!(
            !SESSION_INTERACTIVE_ROLES.contains(&"heading"),
            "heading is not interactive"
        );
    }

    #[test]
    fn session_interactive_roles_does_not_contain_img() {
        assert!(
            !SESSION_INTERACTIVE_ROLES.contains(&"img"),
            "img is not interactive"
        );
    }
}
