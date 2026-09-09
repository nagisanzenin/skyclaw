//! TEMM1E Interactive TUI — Claude Code-level terminal experience.
//!
//! Launch via `temm1e tui`. Provides:
//! - Markdown rendering with syntax highlighting
//! - Real-time agent observability (collapsible activity panel)
//! - Arrow-key onboarding wizard
//! - Slash command system with tab completion
//! - Tem's 7-color design palette
//!
//! # Architecture
//!
//! Uses the TEA (The Elm Architecture) pattern:
//! - `AppState` is the single source of truth
//! - `update()` processes events and mutates state
//! - `view()` renders state to the terminal via ratatui

pub mod agent_bridge;
pub mod app;
pub mod channel;
pub mod commands;
mod connection;
pub mod event;
pub mod input;
pub mod onboarding;
pub mod streaming;
pub mod theme;
pub mod views;
pub mod widgets;

#[cfg(test)]
mod testing;

use std::io;
use std::time::Duration;

use crossterm::event::EventStream;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{cursor, execute};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

use temm1e_agent::agent_task_status::AgentTaskStatus;
use temm1e_core::config::credentials::{load_credentials_file, save_credentials};
use temm1e_core::types::config::Temm1eConfig;
use temm1e_core::types::model_registry::default_model;

use agent_bridge::{validate_provider_key, AgentHandle, AgentSetup};
use app::{update, ApiKeyEntry, AppState, GitInfo, MouseSelection, Overlay, Screen};
use event::Event;
use onboarding::steps::{model_select_items, OnboardingStep};
use widgets::select_list::SelectState;

/// Restore the terminal to normal mode. Safe to call multiple times.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        crossterm::event::DisableMouseCapture,
        LeaveAlternateScreen,
        cursor::Show
    );
    use std::io::Write;
    let _ = io::stdout().flush();
    // disable_raw_mode restores the terminal attributes captured on entry.
    // Do not invoke `stty sane`: it overwrites the caller's intentional settings.
    // Do not drain input here: a continuously readable stream can prevent exit,
    // and buffered input may belong to the caller after we leave the TUI.
}

/// Terminal cleanup guard — restores terminal even on panic.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Launch the interactive TUI.
///
/// This is the main entry point called from `temm1e tui`.
pub async fn launch_tui(config: Temm1eConfig) -> anyhow::Result<()> {
    // 1. Set up terminal (logging already redirected to file by main.rs)
    // Install panic hook that restores terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        original_hook(info);
    }));

    enable_raw_mode()?;
    // Cover failures while entering the alternate screen as well as the loop.
    let _guard = TerminalGuard;
    let mut stdout = io::stdout();
    // Mouse capture is ON so the TUI owns the whole terminal buffer
    // (exclusive alt-screen, scroll wheel scrolls the message list,
    // no scrollback bleed-through). Users select text natively by
    // holding the terminal's modifier override while dragging:
    //
    //   - macOS (iTerm2, Terminal.app): Option+drag
    //   - Linux (alacritty, kitty, GNOME): Shift+drag
    //   - Windows Terminal: Shift+drag
    //
    // For terminals that don't support the override, Alt+S toggles
    // capture entirely (disables scroll wheel, enables native drag).
    execute!(
        stdout,
        EnterAlternateScreen,
        cursor::Hide,
        crossterm::event::EnableMouseCapture
    )?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // 2. Create event channels
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<Event>();

    // 3. Initialize app state + agent
    let mut agent_handle: Option<AgentHandle> = None;
    // Priority: temm1e.toml [provider] > saved credentials > onboarding.
    // Mirrors the `start` / `chat` command logic (src/main.rs:1987-2007)
    // so the TUI honors the same config the other commands do.
    let saved = load_credentials_file();
    let resolved = AgentSetup::resolve(&config, saved.as_ref());

    let mut state = if let Some(setup) = resolved {
        let provider = setup.provider_name.clone();
        let model = setup.model.clone();
        match agent_bridge::spawn_agent(setup, event_tx.clone()).await {
            Ok(handle) => {
                agent_handle = Some(handle);
                AppState::new().with_chat(provider, model)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to start agent — entering onboarding");
                AppState::new().with_onboarding()
            }
        }
    } else {
        AppState::new().with_onboarding()
    };

    // Populate startup caches (git repo info, API keys cache)
    state.git_info = detect_git_info();
    state.api_keys_cache = load_api_keys_cache();

    // Show welcome message if agent is ready
    if agent_handle.is_some() {
        use widgets::markdown::render_markdown_with_width;
        use widgets::message_list::{DisplayMessage, MessageRole};
        let welcome = format!(
            "Welcome to TEMM1E TUI! Provider: {} | Model: {}\n\
             Type a message and press Enter. /help for commands.",
            state.current_provider.as_deref().unwrap_or("?"),
            state.current_model.as_deref().unwrap_or("?"),
        );
        let (tw, _) = crossterm::terminal::size().unwrap_or((80, 24));
        let lines = render_markdown_with_width(
            &welcome,
            state.theme.info,
            state.theme.heading,
            state.theme.code_bg,
            state.theme.info,
            state.theme.secondary,
            tw as usize,
        );
        state.message_list.push(DisplayMessage {
            tool_id: None,
            role: MessageRole::System,
            content: lines,
            timestamp: chrono::Utc::now(),
            usage: None,
        });
    }

    // 4. Set up crossterm event stream
    let mut crossterm_events = EventStream::new();

    // 5. Tick timer for animations (30 FPS)
    let mut tick_interval = tokio::time::interval(Duration::from_millis(33));
    tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // 6. Initial draw
    terminal.draw(|frame| view(&mut state, frame))?;
    state.needs_redraw = false;

    // Draw throttle — cap render rate at ~60 FPS so rapid-fire
    // mouse drag events don't pile up into back-to-back full
    // re-renders. Deferred draws catch up via the tick interval
    // (every 33ms), so the final frame after a drag always happens.
    // Combined with the event-coalescing drain in the event loop,
    // this keeps drag responsiveness smooth even on slower terminals.
    let mut last_draw_at = std::time::Instant::now();
    const MIN_DRAW_INTERVAL: Duration = Duration::from_millis(16);

    // 7. Main event loop
    loop {
        // Build select branches dynamically based on whether we have an agent
        let status_changed = async {
            if let Some(ref mut handle) = agent_handle {
                handle.status_rx.changed().await.ok();
                Some(handle.status_rx.borrow().clone())
            } else {
                // Never resolves if no agent
                std::future::pending::<Option<AgentTaskStatus>>().await
            }
        };

        tokio::select! {
            // Crossterm terminal events
            maybe_event = crossterm_events.next() => {
                if let Some(Ok(event)) = maybe_event {
                    update(&mut state, Event::Terminal(event));
                }
            }
            // Internal events (agent responses, status updates)
            Some(event) = event_rx.recv() => {
                update(&mut state, event);
            }
            // Agent status updates via watch channel
            Some(status) = status_changed => {
                update(&mut state, Event::AgentStatus(status));
            }
            // Tick timer
            _ = tick_interval.tick() => {
                update(&mut state, Event::Tick);
            }
        }

        // ── Event coalescing — drain any immediately-available
        // terminal events so a burst of mouse Drag events (drag
        // speed can exceed 120 Hz) collapses into a single draw
        // instead of one draw per event. The draw throttle below
        // provides the final rate cap; this drain prevents
        // unnecessary intermediate state updates that would just
        // be overwritten. Max 128 events per drain to keep the
        // loop responsive to other event sources.
        use futures::FutureExt;
        for _ in 0..128 {
            match crossterm_events.next().now_or_never() {
                Some(Some(Ok(event))) => {
                    update(&mut state, Event::Terminal(event));
                }
                _ => break,
            }
        }

        // Handle mouse capture toggle (A4) — must run on the real stdout
        if state.needs_mouse_toggle {
            let mut stdout = io::stdout();
            if state.mouse_capture_enabled {
                let _ = execute!(stdout, crossterm::event::EnableMouseCapture);
            } else {
                let _ = execute!(stdout, crossterm::event::DisableMouseCapture);
            }
            state.needs_mouse_toggle = false;
        }

        // Handle Escape/Ctrl+C cancel (Tier C) — fire the interrupt flag
        // that the agent loop polls between rounds.
        if state.pending_cancel {
            if let Some(ref handle) = agent_handle {
                handle
                    .interrupt_flag
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            state.pending_cancel = false;
        }

        // Handle /model hot-swap — tear down the current agent task
        // and spawn a new one with the new model. Only runs while
        // idle (the command handler rejects mid-turn switches).
        if let Some(new_model) = state.pending_model_switch.take() {
            handle_model_switch(&mut state, &mut agent_handle, &event_tx, new_model).await;
        }

        // Handle user message submission → send to agent
        if let Some(text) = state.pending_user_message.take() {
            if let Some(ref handle) = agent_handle {
                // Detect file paths in user input and attach them
                let (msg_text, attachments) = parse_file_references(&text);
                let msg = temm1e_core::types::message::InboundMessage {
                    id: uuid::Uuid::new_v4().to_string(),
                    channel: "tui".to_string(),
                    chat_id: "tui".to_string(),
                    user_id: "local".to_string(),
                    username: Some(whoami()),
                    text: Some(msg_text),
                    attachments,
                    reply_to: None,
                    timestamp: chrono::Utc::now(),
                };
                let _ = handle.inbound_tx.send(msg).await;
            }
        }

        // Honor exit before starting a new onboarding network operation.
        if state.should_quit {
            break;
        }

        // Handle onboarding async operations
        handle_onboarding_async(&mut state, &mut agent_handle, &event_tx, &config).await;

        // Check quit
        if state.should_quit {
            break;
        }

        // Handle terminal resize — clear buffer for clean redraw
        if state.needs_clear {
            terminal.clear()?;
            state.needs_clear = false;
        }

        // Render if needed — throttled to MIN_DRAW_INTERVAL
        if state.needs_redraw && last_draw_at.elapsed() >= MIN_DRAW_INTERVAL {
            terminal.draw(|frame| view(&mut state, frame))?;
            state.needs_redraw = false;
            last_draw_at = std::time::Instant::now();
        }
    }

    if let Some(handle) = agent_handle.take() {
        if !handle
            .shutdown(std::time::Duration::from_secs(6))
            .await
            .loop_joined
        {
            tracing::warn!("TUI shutdown did not fully drain; interrupted evidence is preserved");
        }
    }

    // Explicit terminal restoration before returning
    drop(terminal); // drop ratatui terminal first — releases stdout
    restore_terminal();
    // Guard will also call restore_terminal() — safe to call multiple times

    Ok(())
}

/// Handle async onboarding transitions (key validation, credential saving).
async fn handle_onboarding_async(
    state: &mut AppState,
    agent_handle: &mut Option<AgentHandle>,
    event_tx: &mpsc::UnboundedSender<Event>,
    config: &Temm1eConfig,
) {
    match &state.onboarding_step {
        OnboardingStep::ValidatingKey { provider } => {
            if let Some(ref api_key) = state.onboarding_api_key.clone() {
                let provider = provider.clone();
                let base_url = state.onboarding_base_url.clone();
                // For custom-endpoint providers the default_model may be empty
                // (openai-compatible) — use a sentinel the validator accepts.
                // validate_provider_key short-circuits for base_url.is_some()
                // so the model string isn't actually sent anywhere.
                let default_model_str = default_model(&provider).to_string();
                let model_for_validation = if default_model_str.is_empty() {
                    "custom".to_string()
                } else {
                    default_model_str
                };

                match validate_provider_key(
                    &provider,
                    api_key,
                    &model_for_validation,
                    base_url.as_deref(),
                )
                .await
                {
                    Ok(()) => {
                        state.current_provider = Some(provider.clone());
                        // Providers that expose arbitrary models (openai-compatible,
                        // any name with an empty model list) prompt for free-text
                        // entry; everyone else gets the curated SelectModel list.
                        let items = model_select_items(&provider);
                        if items.is_empty()
                            || crate::onboarding::steps::provider_needs_custom_model(&provider)
                        {
                            state.onboarding_step = OnboardingStep::EnterCustomModel {
                                provider,
                                input: String::new(),
                                error: None,
                            };
                        } else {
                            state.onboarding_step =
                                OnboardingStep::SelectModel(SelectState::new(items));
                        }
                    }
                    Err(e) => {
                        state.onboarding_step = OnboardingStep::EnterApiKey {
                            provider,
                            input: String::new(),
                            error: Some(format!("Validation failed: {}", e)),
                        };
                    }
                }
                state.needs_redraw = true;
            }
        }
        OnboardingStep::Saving => {
            if let (Some(ref provider), Some(ref api_key)) =
                (&state.current_provider, &state.onboarding_api_key.clone())
            {
                let model = state
                    .current_model
                    .clone()
                    .unwrap_or_else(|| default_model(provider).to_string());
                let base_url = state.onboarding_base_url.clone();

                match save_credentials(provider, api_key, &model, base_url.as_deref()).await {
                    Ok(()) => {
                        match agent_bridge::spawn_agent(
                            AgentSetup {
                                provider_name: provider.clone(),
                                api_key: api_key.clone(),
                                keys: connection::onboarding_keys(
                                    load_credentials_file().as_ref(),
                                    provider,
                                    &base_url,
                                    api_key,
                                ),
                                model: model.clone(),
                                base_url: base_url.clone(),
                                config: config.clone(),
                                mode: state.selected_mode.clone(),
                            },
                            event_tx.clone(),
                        )
                        .await
                        {
                            Ok(handle) => {
                                *agent_handle = Some(handle);
                                // Skip Done screen — go straight to chat
                                state.screen = Screen::Chat;
                                state.onboarding_step = OnboardingStep::Done;
                            }
                            Err(e) => {
                                state.onboarding_step = OnboardingStep::EnterApiKey {
                                    provider: provider.clone(),
                                    input: String::new(),
                                    error: Some(format!("Agent failed to start: {}", e)),
                                };
                            }
                        }
                    }
                    Err(e) => {
                        state.onboarding_step = OnboardingStep::EnterApiKey {
                            provider: provider.clone(),
                            input: String::new(),
                            error: Some(format!("Failed to save: {}", e)),
                        };
                    }
                }
                state.onboarding_api_key = None;
                state.needs_redraw = true;
            }
        }
        _ => {}
    }
}

/// Parse file references from user input.
///
/// Supports:
/// - `/file <path>` — explicit file attachment
/// - Bare paths starting with `/` or `~/` or `./` — auto-detected
fn parse_file_references(text: &str) -> (String, Vec<temm1e_core::types::message::AttachmentRef>) {
    let trimmed = text.trim();

    // Explicit /file command
    if let Some(path_str) = trimmed.strip_prefix("/file ") {
        let path = std::path::Path::new(path_str.trim());
        let expanded = if path_str.trim().starts_with('~') {
            dirs::home_dir()
                .unwrap_or_default()
                .join(path_str.trim().trim_start_matches("~/"))
        } else {
            path.to_path_buf()
        };
        if expanded.exists() {
            let att = temm1e_core::types::message::AttachmentRef {
                file_id: expanded.to_string_lossy().to_string(),
                file_name: expanded
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string()),
                mime_type: None,
                size: std::fs::metadata(&expanded).ok().map(|m| m.len() as usize),
            };
            return (format!("[file: {}]", expanded.display()), vec![att]);
        }
        return (format!("[file not found: {}]", expanded.display()), vec![]);
    }

    // Auto-detect bare file paths (e.g. dragged into terminal)
    // Check if the entire input looks like a file path
    let expanded = if trimmed.starts_with('~') {
        dirs::home_dir()
            .unwrap_or_default()
            .join(trimmed.trim_start_matches("~/"))
    } else {
        std::path::PathBuf::from(trimmed)
    };

    if (trimmed.starts_with('/') || trimmed.starts_with("~/"))
        && !trimmed.contains(' ')
        && expanded.exists()
    {
        let att = temm1e_core::types::message::AttachmentRef {
            file_id: expanded.to_string_lossy().to_string(),
            file_name: expanded
                .file_name()
                .map(|n| n.to_string_lossy().to_string()),
            mime_type: None,
            size: std::fs::metadata(&expanded).ok().map(|m| m.len() as usize),
        };
        return (
            format!("[file: {}] Analyze this file.", expanded.display()),
            vec![att],
        );
    }

    (trimmed.to_string(), vec![])
}

/// Get the current OS username.
fn whoami() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Validate + apply a model hot-swap. Called from the event loop
/// when `state.pending_model_switch` is set. Drops the old agent
/// handle (its task exits when the sender is dropped), spawns a new
/// agent with the new model, saves the new model to credentials.toml
/// only when its saved connection still matches; updates `state.current_model`.
async fn handle_model_switch(
    state: &mut AppState,
    agent_handle: &mut Option<AgentHandle>,
    event_tx: &mpsc::UnboundedSender<Event>,
    new_model: String,
) {
    use temm1e_core::types::model_registry::available_models_for_provider;
    let Some(current) = agent_handle.as_ref() else {
        push_system_line_via_tx(
            event_tx,
            "Cannot switch model: no active connection. Restart or reconnect first.".into(),
        );
        return;
    };
    let previous = current.setup.clone();
    let budget = current.budget.clone();
    let provider = &previous.provider_name;
    let known = available_models_for_provider(provider);
    let registered =
        temm1e_core::config::custom_models::lookup_custom_model(provider, &new_model).is_some();
    if new_model.trim().is_empty()
        || (!known.is_empty()
            && !known.contains(&new_model.as_str())
            && !registered
            && previous.base_url.is_none()
            && provider != "openrouter")
    {
        push_system_line_via_tx(event_tx, format!("Unknown model '{new_model}' for provider '{provider}'. Register a custom model or choose a listed suggestion: {}", known.join(", ")));
        return;
    }
    let mut replacement = previous.clone();
    replacement.model.clone_from(&new_model);
    replacement.mode.clone_from(&state.selected_mode);
    if let Some(old) = agent_handle.take() {
        let report = old.shutdown(std::time::Duration::from_secs(6)).await;
        if !report.loop_joined || !report.background_drained {
            push_system_line_via_tx(event_tx, "Model switch stopped: previous runtime did not fully drain. Restart before sending another message.".into());
            return;
        }
    }
    match agent_bridge::spawn_agent_with_budget(
        replacement.clone(),
        event_tx.clone(),
        budget.clone(),
    )
    .await
    {
        Ok(handle) => {
            *agent_handle = Some(handle);
            state.current_provider = Some(replacement.provider_name.clone());
            state.current_model = Some(new_model.clone());
            let persisted = (|| -> Result<bool, temm1e_core::types::error::Temm1eError> {
                let Some(mut saved) = load_credentials_file() else {
                    return Ok(false);
                };
                if !connection::update_saved_model(&replacement, &mut saved) {
                    return Ok(false);
                }
                let content = toml::to_string_pretty(&saved).map_err(|_| {
                    temm1e_core::types::error::Temm1eError::Config(
                        "Could not encode model selection".into(),
                    )
                })?;
                temm1e_core::private_file::write_private_atomic(
                    &temm1e_core::config::credentials::credentials_path(),
                    content.as_bytes(),
                )?;
                Ok(true)
            })();
            let suffix = match persisted {
                Ok(true) => " (saved)",
                Ok(false) => " (this session; configured or saved connection differs)",
                Err(error) => {
                    tracing::warn!(%error, "Model selection could not be persisted");
                    " (this session; saving failed)"
                }
            };
            push_system_line_via_tx(
                event_tx,
                format!("✓ Switched to model '{new_model}'{suffix}"),
            );
        }
        Err(error) => {
            tracing::warn!(%error, "TUI replacement runtime failed; restoring previous connection");
            match agent_bridge::spawn_agent_with_budget(previous, event_tx.clone(), budget).await {
                Ok(handle) => {
                    *agent_handle = Some(handle);
                    push_system_line_via_tx(
                        event_tx,
                        "Model switch failed; previous model restored.".into(),
                    );
                }
                Err(restore_error) => {
                    tracing::warn!(%restore_error, "Previous TUI runtime could not be restored");
                    push_system_line_via_tx(event_tx, "Model switch failed and the previous runtime could not restart. Reconnect before sending messages.".into());
                }
            }
        }
    }
}

/// Push a system message through the event channel so the TUI
/// renders it on the next iteration. Used by async side effects
/// that don't hold a mutable `AppState` reference.
fn push_system_line_via_tx(event_tx: &mpsc::UnboundedSender<Event>, text: String) {
    let _ = event_tx.send(Event::AgentResponse(event::AgentResponseEvent {
        kind: crate::event::ResponseKind::Notice,
        message: temm1e_core::types::message::OutboundMessage {
            chat_id: "tui".to_string(),
            text,
            reply_to: None,
            parse_mode: None,
        },
        input_tokens: 0,
        output_tokens: 0,
        cost_usd: 0.0,
    }));
}

/// Detect git repository + branch. Returns `None` if:
/// - not inside a git repo
/// - `git` binary is missing
/// - any subcommand fails
///
/// Silent failure by design — status bar degrades gracefully.
fn detect_git_info() -> Option<GitInfo> {
    use std::process::Command;

    // Repo toplevel
    let toplevel = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !toplevel.status.success() {
        return None;
    }
    let path = String::from_utf8(toplevel.stdout).ok()?;
    let path_trimmed = path.trim();
    if path_trimmed.is_empty() {
        return None;
    }
    let repo_name = std::path::Path::new(path_trimmed)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());

    // Branch — three-stage fallback: branch --show-current, symbolic-ref, short hash
    let mut branch = String::new();
    if let Ok(out) = Command::new("git")
        .args(["branch", "--show-current"])
        .output()
    {
        if out.status.success() {
            branch = String::from_utf8(out.stdout)
                .unwrap_or_default()
                .trim()
                .to_string();
        }
    }
    if branch.is_empty() {
        if let Ok(out) = Command::new("git")
            .args(["symbolic-ref", "--short", "HEAD"])
            .output()
        {
            if out.status.success() {
                branch = String::from_utf8(out.stdout)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
            }
        }
    }
    if branch.is_empty() {
        if let Ok(out) = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
        {
            if out.status.success() {
                let h = String::from_utf8(out.stdout)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if !h.is_empty() {
                    branch = format!("@{h}");
                }
            }
        }
    }
    if branch.is_empty() {
        branch = "(unknown)".to_string();
    }

    Some(GitInfo { repo_name, branch })
}

/// Build the API keys cache from the credentials file. Returns an
/// empty Vec on any error — overlays degrade gracefully.
fn load_api_keys_cache() -> Vec<ApiKeyEntry> {
    let Some(creds) = load_credentials_file() else {
        return Vec::new();
    };
    let active = creds.active.clone();
    creds
        .providers
        .iter()
        .map(|p| {
            // Pick the first non-placeholder key for the fingerprint; if
            // all keys are placeholders, use "?" as a marker.
            // Proxy providers use lenient placeholder check so short
            // LM Studio / Ollama keys aren't shown as "????".
            let has_custom = p.base_url.is_some();
            let fingerprint = p
                .keys
                .iter()
                .find(|k| {
                    let is_ph = if has_custom {
                        temm1e_core::config::credentials::is_placeholder_key_lenient(k)
                    } else {
                        temm1e_core::config::credentials::is_placeholder_key(k)
                    };
                    !is_ph && k.len() >= 4
                })
                .map(|k| {
                    k.chars()
                        .rev()
                        .take(4)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>()
                })
                .unwrap_or_else(|| "????".to_string());
            ApiKeyEntry {
                provider: p.name.clone(),
                fingerprint,
                is_active: p.name == active,
            }
        })
        .collect()
}

/// TEA view function — renders AppState to a ratatui frame.
///
/// Takes `&mut AppState` so the post-render selection pass can:
/// - extract cell symbols when a drag-to-select just ended, and
/// - clear the corresponding pending flags.
fn view(state: &mut AppState, frame: &mut ratatui::Frame) {
    if let Some(renderer) = &mut state.streaming_renderer {
        renderer.refresh();
    }
    let area = frame.area();

    match state.screen {
        Screen::Onboarding => {
            views::onboarding::render_onboarding(
                &state.onboarding_step,
                &state.theme,
                area,
                frame.buffer_mut(),
            );
        }
        Screen::Chat => {
            views::chat::render_chat(state, area, frame.buffer_mut());

            // Render overlay on top
            match &state.overlay {
                Overlay::Help => {
                    views::help::render_help(
                        &state.command_registry,
                        &state.theme,
                        area,
                        frame.buffer_mut(),
                    );
                }
                Overlay::Config(kind) => {
                    views::config_panel::render_config_overlay(
                        kind,
                        state,
                        area,
                        frame.buffer_mut(),
                    );
                }
                Overlay::CopyPicker => {
                    widgets::copy_picker::render_copy_picker(state, area, frame.buffer_mut());
                }
                Overlay::None => {}
            }
        }
    }

    // ── Selection + click-to-copy pass ─────────────────────
    //
    // CRITICAL: clear any stale REVERSED modifier from the buffer
    // BEFORE applying a new highlight. ratatui's `Cell::set_style`
    // (invoked by `Buffer::set_line` during normal rendering) only
    // ADDS modifiers — it never removes them. So highlights from
    // previous frames persist on cells the new render pass writes
    // to, producing "ghost" highlights that accumulate over time.
    //
    // We always clear (even when there is no active selection) so
    // a selection that was cleared logically also disappears
    // visually on the next frame.
    let buf = frame.buffer_mut();
    clear_reverse_highlight(buf);

    // Drag selection: highlight + Ctrl+C copy
    //
    // Selection is stored in ABSOLUTE content line coordinates so
    // the highlight tracks content as the list scrolls. Extraction
    // reads from the message list directly (not the buffer), so
    // the entire logical selection is copied even if part of it
    // has scrolled off-screen during an auto-scroll drag.
    if let Some(sel) = state.mouse_selection.clone() {
        let (start, end) = normalized_selection(&sel);

        if state.pending_copy_selection {
            state.pending_copy_selection = false;
            // Release the buf borrow so we can borrow state immutably for extraction
            let text = extract_selection_text(state, start.0, start.1, end.0, end.1);
            if !text.trim().is_empty() {
                match widgets::copy_picker::copy_to_clipboard(&text) {
                    Ok(()) => {
                        let char_count = text.trim().chars().count();
                        state.copy_feedback =
                            Some(format!("Copied {char_count} chars to clipboard"));
                    }
                    Err(e) => {
                        state.copy_feedback = Some(format!("Copy failed: {e}"));
                    }
                }
            }
            state.mouse_selection = None;
        } else {
            let ctx = crate::app::compute_selection_ctx(state);
            let buf = frame.buffer_mut();
            highlight_selection_cells(buf, start.0, start.1, end.0, end.1, &ctx);
        }
    }

    // Single-click on a code block → copy whole block
    if let Some((_col, row)) = state.pending_code_click.take() {
        let buf = frame.buffer_mut();
        if let Some((top_row, bot_row, text)) = extract_code_block_at(buf, row) {
            match widgets::copy_picker::copy_to_clipboard(&text) {
                Ok(()) => {
                    let line_count = text.lines().count();
                    state.copy_feedback =
                        Some(format!("Copied {line_count} lines of code to clipboard"));
                    highlight_code_block(buf, top_row, bot_row);
                }
                Err(e) => {
                    state.copy_feedback = Some(format!("Copy failed: {e}"));
                }
            }
        }
    }
}

/// Clear any stale REVERSED modifier from every cell in the buffer.
/// Required before applying a new selection highlight because
/// ratatui's `Cell::set_style` (invoked by `Buffer::set_line` during
/// render_chat) only ADDS modifiers — it does not clear existing
/// ones. Without this explicit clear, every cell that was ever
/// highlighted accumulates REVERSED forever, producing ghost
/// highlights scattered across the screen as the user drags or
/// scrolls.
///
/// Touches `buf.area` which is the full frame — single pass over
/// the terminal's cells, `O(width * height)`. Fast.
fn clear_reverse_highlight(buf: &mut ratatui::buffer::Buffer) {
    use ratatui::style::Modifier;
    let area = buf.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].modifier.remove(Modifier::REVERSED);
        }
    }
}

/// Normalize a `MouseSelection` into a (start, end) pair in reading
/// order (earlier line first; within same line, earlier column).
/// Both the anchor and the current are in ABSOLUTE content line
/// coordinates — terminal row mapping happens separately.
fn normalized_selection(sel: &MouseSelection) -> ((u16, i64), (u16, i64)) {
    let a = sel.anchor;
    let c = sel.current;
    if (a.1, a.0) <= (c.1, c.0) {
        (a, c)
    } else {
        (c, a)
    }
}

/// Extract the selected content as plain text, using ABSOLUTE line
/// indices into the full rendered content (not the visible buffer).
/// This means the entire selection is copied even if part of it has
/// scrolled off-screen during an auto-scroll drag — a critical win
/// over buffer-based extraction.
///
/// Strips code-block decoration added by `widgets/markdown.rs`:
///
///   - Rows whose first non-space char is `╭` or `╰` are top/bottom
///     borders of a rendered code block — skipped entirely.
///   - Rows starting with ` │ ` (space + U+2502 + space) are code
///     content with a gutter prefix — the prefix is stripped,
///     indentation inside the prefix is preserved.
///
/// Non-code rows honor the selection's column range at the start
/// and end rows so the user can select fragments of prose.
fn extract_selection_text(
    state: &AppState,
    start_col: u16,
    start_abs: i64,
    end_col: u16,
    end_abs: i64,
) -> String {
    // Re-materialize the full flattened line list so we can index
    // by absolute position. Styles don't matter for extraction, so
    // pass defaults.
    let style = ratatui::style::Style::default();
    let mut all_lines = state.message_list.render_lines(style, style, style, style);

    // Append streaming content if active, matching chat.rs so the
    // absolute indices used by the selection are consistent with
    // what's actually rendered.
    if state.is_agent_working {
        if let Some(ref renderer) = state.streaming_renderer {
            for rl in renderer.lines() {
                all_lines.push(ratatui::text::Line::from(rl.spans.clone()));
            }
        }
    }

    if all_lines.is_empty() {
        return String::new();
    }

    let start_idx = start_abs.max(0) as usize;
    let end_idx = (end_abs.max(0) as usize).min(all_lines.len().saturating_sub(1));
    if start_idx > end_idx {
        return String::new();
    }

    let mut rows: Vec<String> = Vec::new();
    for (offset, line) in all_lines[start_idx..=end_idx].iter().enumerate() {
        let idx = start_idx + offset;
        // Flatten the Line's spans to plain text. Each span's
        // `content` is a `Cow<str>` over the original rendered
        // text, so this preserves every grapheme exactly.
        let full_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let trimmed_end = full_text.trim_end();

        // Skip code-block borders
        let leading = trimmed_end.trim_start();
        if leading.starts_with('\u{256d}') || leading.starts_with('\u{2570}') {
            continue;
        }

        // Strip code-row gutter prefix
        if let Some(code) = trimmed_end.strip_prefix(" \u{2502} ") {
            rows.push(code.to_string());
            continue;
        }

        // Non-code row: honor the selection's column range at the
        // start and end rows. Middle rows are captured in full.
        let x_start = if idx == start_idx {
            start_col as usize
        } else {
            0
        };
        let x_end = if idx == end_idx {
            (end_col as usize).saturating_add(1)
        } else {
            usize::MAX
        };
        let partial: String = trimmed_end
            .chars()
            .skip(x_start)
            .take(x_end.saturating_sub(x_start))
            .collect();
        rows.push(partial.trim_end().to_string());
    }

    rows.join("\n")
}

/// Detect whether `row` is inside a rendered code block (identified
/// by the ` │ ` gutter prefix added by `widgets/markdown.rs`), and
/// if so, walk up/down to find the block's extent and extract the
/// gutter-stripped code. Returns `(top_row, bot_row, text)` on hit.
fn extract_code_block_at(buf: &ratatui::buffer::Buffer, row: u16) -> Option<(u16, u16, String)> {
    let left = buf.area.left();
    let right = buf.area.right();
    let top_edge = buf.area.top();
    let bot_edge = buf.area.bottom();

    if buf.area.width < 3 {
        return None;
    }

    // A code row has ` │ ` (U+0020 U+2502 U+0020) as its first 3 cells.
    let is_code_row = |y: u16| -> bool {
        if y < top_edge || y >= bot_edge {
            return false;
        }
        buf[(left, y)].symbol() == " "
            && buf[(left + 1, y)].symbol() == "\u{2502}"
            && buf[(left + 2, y)].symbol() == " "
    };

    if !is_code_row(row) {
        return None;
    }

    // Walk up while rows are still code rows
    let mut top = row;
    while top > top_edge && is_code_row(top - 1) {
        top -= 1;
    }
    // Walk down
    let mut bot = row;
    while bot + 1 < bot_edge && is_code_row(bot + 1) {
        bot += 1;
    }

    // Extract: for each row, skip the ` │ ` prefix (3 cells) and
    // capture content up to the right edge, trimming trailing spaces.
    let content_start = left + 3;
    let mut out = String::new();
    for y in top..=bot {
        let mut line = String::new();
        for x in content_start..right {
            line.push_str(buf[(x, y)].symbol());
        }
        out.push_str(line.trim_end());
        if y < bot {
            out.push('\n');
        }
    }

    if out.trim().is_empty() {
        None
    } else {
        Some((top, bot, out))
    }
}

/// Apply REVERSED to every cell of a code block's rendered rows
/// (including the gutter prefix). Used as a visual flash when the
/// user clicks to copy a block.
///
/// Mutates the `modifier` field directly (not via `set_style`)
/// because `Cell::set_style` has add-only semantics for modifiers
/// — using it would still work in isolation but is misleading.
fn highlight_code_block(buf: &mut ratatui::buffer::Buffer, top: u16, bot: u16) {
    use ratatui::style::Modifier;
    let left = buf.area.left();
    let right = buf.area.right();
    for y in top..=bot {
        if y >= buf.area.bottom() {
            break;
        }
        for x in left..right {
            buf[(x, y)].modifier.insert(Modifier::REVERSED);
        }
    }
}

/// Apply REVERSED to the cells of the selection that are currently
/// visible in the viewport. Uses absolute line indices, clipping
/// any part of the selection that's scrolled off-screen. The clip
/// means the VISIBLE portion follows content as the user scrolls —
/// content that scrolls back into view regains its highlight.
///
/// Assumes `clear_reverse_highlight` was called earlier in the
/// same frame so only the current selection ends up highlighted.
fn highlight_selection_cells(
    buf: &mut ratatui::buffer::Buffer,
    start_col: u16,
    start_abs: i64,
    end_col: u16,
    end_abs: i64,
    ctx: &crate::app::SelectionCtx,
) {
    use ratatui::style::Modifier;
    let left_edge = buf.area.left();
    let right_edge = buf.area.right();

    // Iterate only over VISIBLE terminal rows for efficiency.
    for term_row in ctx.msg_top..ctx.msg_bottom {
        if term_row >= buf.area.bottom() {
            break;
        }
        let abs = ctx.term_to_abs(term_row);
        if abs < start_abs || abs > end_abs {
            continue;
        }

        // Determine column range for this row
        let x_start = if abs == start_abs {
            start_col
        } else {
            left_edge
        };
        let x_end = if abs == end_abs {
            (end_col.saturating_add(1)).min(right_edge)
        } else {
            right_edge
        };

        for x in x_start..x_end {
            buf[(x, term_row)].modifier.insert(Modifier::REVERSED);
        }
    }
}
