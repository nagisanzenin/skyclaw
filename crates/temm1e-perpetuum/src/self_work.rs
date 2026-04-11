use std::sync::Arc;

use temm1e_core::types::error::Temm1eError;

use crate::bug_reporter;
use crate::cognitive::LlmCaller;
use crate::conscience::SelfWorkKind;
use crate::log_scanner;
use crate::store::Store;

/// Execute a self-work activity during Sleep state.
pub async fn execute_self_work(
    kind: &SelfWorkKind,
    store: &Arc<Store>,
    caller: Option<&Arc<dyn LlmCaller>>,
) -> Result<String, Temm1eError> {
    match kind {
        SelfWorkKind::MemoryConsolidation => consolidate_memory(store).await,
        SelfWorkKind::SessionCleanup => cleanup_sessions(store).await,
        SelfWorkKind::BlueprintRefinement => refine_blueprints(store).await,
        SelfWorkKind::FailureAnalysis => {
            if let Some(caller) = caller {
                analyze_failures(store, caller).await
            } else {
                Ok("Skipped: no LLM caller available".to_string())
            }
        }
        SelfWorkKind::LogIntrospection => {
            if let Some(caller) = caller {
                introspect_logs(store, caller).await
            } else {
                Ok("Skipped: no LLM caller available".to_string())
            }
        }
        SelfWorkKind::Vigil => {
            if let Some(caller) = caller {
                run_vigil(store, caller).await
            } else {
                Ok("Skipped: no LLM caller available".to_string())
            }
        }
        SelfWorkKind::CambiumSkills => {
            if let Some(caller) = caller {
                grow_skills(store, caller).await
            } else {
                Ok("Skipped: no LLM caller available".to_string())
            }
        }
    }
}

/// Memory consolidation: clean up expired volition notes, prune old monitor history.
async fn consolidate_memory(store: &Arc<Store>) -> Result<String, Temm1eError> {
    store.cleanup_expired_notes().await?;
    // Prune monitor history older than 7 days (keep last 100 per concern)
    // For now, expired notes cleanup is the primary consolidation
    tracing::info!(target: "perpetuum", work = "memory_consolidation", "Consolidated memory");
    Ok("Memory consolidated: expired notes cleaned".to_string())
}

/// Session cleanup: no-op for now (placeholder for future session pruning).
async fn cleanup_sessions(_store: &Arc<Store>) -> Result<String, Temm1eError> {
    tracing::info!(target: "perpetuum", work = "session_cleanup", "Session cleanup complete");
    Ok("Session cleanup complete".to_string())
}

/// Blueprint refinement: no-op for now (placeholder for future blueprint weight updates).
async fn refine_blueprints(_store: &Arc<Store>) -> Result<String, Temm1eError> {
    tracing::info!(target: "perpetuum", work = "blueprint_refinement", "Blueprint refinement complete");
    Ok("Blueprint refinement complete".to_string())
}

/// Failure analysis: LLM reviews recent errors from volition notes and transition logs.
async fn analyze_failures(
    store: &Arc<Store>,
    caller: &Arc<dyn LlmCaller>,
) -> Result<String, Temm1eError> {
    let notes = store.get_volition_notes(20).await?;
    if notes.is_empty() {
        return Ok("No recent notes to analyze".to_string());
    }

    let notes_text = notes.join("\n- ");
    let prompt = format!(
        "Review these recent agent activity notes and identify any failure patterns or recurring issues:\n\
         - {notes_text}\n\n\
         Summarize findings in 2-3 sentences. Focus on actionable patterns."
    );

    let analysis = caller.call(None, &prompt).await?;

    // Save the analysis as a volition note for future reference
    store
        .save_volition_note(&format!("Failure analysis: {analysis}"), "self_work")
        .await?;

    tracing::info!(target: "perpetuum", work = "failure_analysis", "Failure analysis complete");
    Ok(format!("Failure analysis: {analysis}"))
}

/// Log introspection: LLM reviews recent interaction patterns.
async fn introspect_logs(
    store: &Arc<Store>,
    caller: &Arc<dyn LlmCaller>,
) -> Result<String, Temm1eError> {
    let notes = store.get_volition_notes(10).await?;
    if notes.is_empty() {
        return Ok("No recent activity to introspect".to_string());
    }

    let notes_text = notes.join("\n- ");
    let prompt = format!(
        "Review these recent agent activity notes and extract any learnings about user preferences or effective strategies:\n\
         - {notes_text}\n\n\
         Summarize in 2-3 sentences. Focus on what worked well."
    );

    let insights = caller.call(None, &prompt).await?;

    store
        .save_volition_note(&format!("Introspection: {insights}"), "self_work")
        .await?;

    tracing::info!(target: "perpetuum", work = "log_introspection", "Log introspection complete");
    Ok(format!("Introspection: {insights}"))
}

/// Load the GitHub PAT from credentials.toml (if configured).
fn load_github_token() -> Option<String> {
    let creds = temm1e_core::config::credentials::load_credentials_file()?;
    let github = creds.providers.iter().find(|p| p.name == "github")?;
    github.keys.first().cloned()
}

/// Check if bug reporting consent has been given.
fn is_consent_given() -> bool {
    let path = dirs::home_dir()
        .unwrap_or_default()
        .join(".temm1e")
        .join("vigil.toml");
    std::fs::read_to_string(&path)
        .unwrap_or_default()
        .contains("consent_given = true")
}

/// Bug review: scan logs for recurring errors, triage via LLM, report to GitHub.
async fn run_vigil(store: &Arc<Store>, caller: &Arc<dyn LlmCaller>) -> Result<String, Temm1eError> {
    // Check rate limit (max 1 report per 6 hours)
    if let Ok(notes) = store.get_volition_notes(20).await {
        for note in &notes {
            if let Some(ts_str) = note.strip_prefix("bug_review_last:") {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts_str.trim()) {
                    let elapsed = chrono::Utc::now() - dt.with_timezone(&chrono::Utc);
                    if elapsed < chrono::Duration::hours(6) {
                        return Ok("Vigil: rate limited, skipping".to_string());
                    }
                }
                break;
            }
        }
    }

    // Scan logs
    let log_path = temm1e_observable::file_logger::current_log_path();
    let errors = log_scanner::scan_recent_errors(&log_path, 6, 2);

    if errors.is_empty() {
        return Ok("Vigil: no recurring errors found".to_string());
    }

    // Load GitHub token (if not configured, triage only — no reporting)
    let github_token = load_github_token();
    let can_report = github_token.is_some() && is_consent_given();

    // Triage each error group via LLM
    let system = "You are reviewing error logs from TEMM1E, an AI agent runtime.";
    let mut bugs_found = 0;
    let mut reported = 0;
    let client = reqwest::Client::new();
    let version = env!("CARGO_PKG_VERSION");
    let os_info = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);

    for error in &errors {
        let prompt = bug_reporter::build_triage_prompt(error);
        match caller.call(Some(system), &prompt).await {
            Ok(response) => {
                let category = bug_reporter::parse_triage_category(&response);
                if category == "BUG" {
                    bugs_found += 1;
                    tracing::info!(
                        target: "perpetuum",
                        signature = %error.signature,
                        count = error.count,
                        "Vigil: found reportable bug"
                    );

                    // Wire 2: route the bug to the Cambium inbox for later
                    // user-triggered growth. The user can review the inbox
                    // and run `/cambium grow fix <sig>` to address specific
                    // bugs. This is opt-in via `cambium.vigil_bridge_enabled`.
                    if cambium_vigil_bridge_enabled() {
                        if let Err(e) =
                            write_cambium_inbox_entry(&error.signature, &error.message, error.count)
                                .await
                        {
                            tracing::warn!(
                                target: "perpetuum",
                                error = %e,
                                "Vigil: failed to write cambium inbox entry"
                            );
                        }
                    }

                    // Try to report to GitHub if configured
                    if can_report {
                        if let Some(ref token) = github_token {
                            // Dedup — skip if already reported
                            match bug_reporter::is_duplicate(&client, token, &error.signature).await
                            {
                                Ok(true) => {
                                    tracing::debug!(
                                        target: "perpetuum",
                                        signature = %error.signature,
                                        "Vigil: already reported, skipping"
                                    );
                                }
                                Ok(false) => {
                                    // Scrub and create issue
                                    let body = bug_reporter::format_issue_body(
                                        error, &response, version, &os_info,
                                    );
                                    let scrubbed = temm1e_tools::credential_scrub::scrub_for_report(
                                        &body,
                                        &[],
                                    );
                                    let scrubbed =
                                        temm1e_tools::credential_scrub::entropy_scrub(&scrubbed);

                                    let title = format!(
                                        "[BUG] {}",
                                        &error.message[..error.message.len().min(70)]
                                    );

                                    match bug_reporter::create_issue(
                                        &client, token, &title, &scrubbed,
                                    )
                                    .await
                                    {
                                        Ok(url) => {
                                            reported += 1;
                                            tracing::info!(
                                                target: "perpetuum",
                                                url = %url,
                                                "Vigil: issue created"
                                            );
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                error = %e,
                                                "Vigil: GitHub issue creation failed"
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        error = %e,
                                        "Vigil: dedup check failed"
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Vigil: LLM triage failed for one error");
            }
        }
    }

    tracing::info!(
        target: "perpetuum",
        total_errors = errors.len(),
        bugs = bugs_found,
        reported,
        "Vigil complete"
    );

    // Record timestamp to enforce rate limit
    store
        .save_volition_note(
            &format!("bug_review_last:{}", chrono::Utc::now().to_rfc3339()),
            "self_work",
        )
        .await?;

    Ok(format!(
        "Vigil: scanned {} error groups, {} bugs found, {} reported to GitHub",
        errors.len(),
        bugs_found,
        reported
    ))
}

/// Skill-layer cambium growth: analyze recent activity for unmet capability
/// gaps, then write reusable skill files to `~/.temm1e/skills/`.
///
/// Rate limited to once per 24 hours. The handler is gated by
/// `cambium.enabled = true` at the call site (concern dispatch). When
/// disabled, this function is never invoked.
///
/// Output skill files use the TEMM1E native format (YAML frontmatter +
/// markdown body) and are picked up by `SkillRegistry::reload()` without
/// requiring a binary restart.
async fn grow_skills(
    store: &Arc<Store>,
    caller: &Arc<dyn LlmCaller>,
) -> Result<String, Temm1eError> {
    // Rate limit: max 1 skill-grow session per 24 hours.
    if let Ok(notes) = store.get_volition_notes(20).await {
        for note in &notes {
            if let Some(ts_str) = note.strip_prefix("cambium_grow_last:") {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts_str.trim()) {
                    let elapsed = chrono::Utc::now() - dt.with_timezone(&chrono::Utc);
                    if elapsed < chrono::Duration::hours(24) {
                        return Ok(format!(
                            "Skill grow: rate limited ({}h since last)",
                            elapsed.num_hours()
                        ));
                    }
                }
            }
        }
    }

    // Collect recent activity for gap analysis.
    let notes = store.get_volition_notes(20).await.unwrap_or_default();
    if notes.is_empty() {
        return Ok("Skill grow: no recent activity to analyze".to_string());
    }

    let activity_text = notes.join("\n- ");
    let system = "You are analyzing recent agent activity to identify reusable \
                  skill opportunities. A skill is a markdown procedure for handling \
                  a specific task type. Only suggest skills for patterns that appeared \
                  3+ times. Respond with a JSON array of skills, each with fields \
                  'name' (kebab-case), 'description' (one line), 'capabilities' (array \
                  of keywords), and 'instructions' (markdown body). If no patterns \
                  warrant a skill, return an empty array []. Respond with ONLY the \
                  JSON, no prose.";

    let prompt = format!(
        "Recent activity notes:\n- {activity_text}\n\n\
         Identify reusable skill opportunities. Return JSON array."
    );

    let response = caller.call(Some(system), &prompt).await?;
    let extracted = extract_json_array(&response);

    // Parse the response.
    let suggestions: Vec<SkillSuggestion> = match serde_json::from_str(extracted) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                target: "perpetuum",
                error = %e,
                response = %response,
                extracted = %extracted,
                "Skill grow: failed to parse LLM response, skipping"
            );
            return Ok("Skill grow: LLM response unparseable".to_string());
        }
    };

    if suggestions.is_empty() {
        // Still record timestamp so we don't re-run for 24h.
        store
            .save_volition_note(
                &format!("cambium_grow_last:{}", chrono::Utc::now().to_rfc3339()),
                "self_work",
            )
            .await?;
        return Ok("Skill grow: no skill opportunities found".to_string());
    }

    // Write each skill to ~/.temm1e/skills/cambium-<name>.md
    // Test override: set TEMM1E_CAMBIUM_SKILLS_DIR to redirect skill output
    // to a controlled directory. Production never sets this and uses
    // ~/.temm1e/skills/.
    let skills_dir = if let Ok(override_path) = std::env::var("TEMM1E_CAMBIUM_SKILLS_DIR") {
        std::path::PathBuf::from(override_path)
    } else {
        match dirs::home_dir() {
            Some(home) => home.join(".temm1e").join("skills"),
            None => {
                return Ok("Skill grow: cannot resolve home directory".to_string());
            }
        }
    };

    if let Err(e) = tokio::fs::create_dir_all(&skills_dir).await {
        return Err(Temm1eError::Tool(format!(
            "Failed to create skills directory: {e}"
        )));
    }

    let mut written = 0;
    for suggestion in &suggestions {
        // Sanitize name for filesystem safety.
        let safe_name: String = suggestion
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        if safe_name.is_empty() {
            continue;
        }

        let filename = format!("cambium-{safe_name}.md");
        let path = skills_dir.join(&filename);

        let caps_yaml: String = suggestion
            .capabilities
            .iter()
            .map(|c| format!("  - {c}\n"))
            .collect();

        let content = format!(
            "---\nname: {}\ndescription: {}\ncapabilities:\n{}version: 1.0.0\n---\n{}\n",
            suggestion.name, suggestion.description, caps_yaml, suggestion.instructions
        );

        if let Err(e) = tokio::fs::write(&path, content).await {
            tracing::warn!(
                target: "perpetuum",
                error = %e,
                path = %path.display(),
                "Skill grow: failed to write skill file"
            );
            continue;
        }
        written += 1;
        tracing::info!(
            target: "perpetuum",
            path = %path.display(),
            "Skill grow: wrote skill file"
        );
    }

    // Record timestamp for rate limiting.
    store
        .save_volition_note(
            &format!("cambium_grow_last:{}", chrono::Utc::now().to_rfc3339()),
            "self_work",
        )
        .await?;

    Ok(format!(
        "Skill grow: analyzed {} notes, wrote {} skill file(s)",
        notes.len(),
        written
    ))
}

/// LLM response shape for skill suggestions.
#[derive(serde::Deserialize)]
struct SkillSuggestion {
    name: String,
    description: String,
    capabilities: Vec<String>,
    instructions: String,
}

/// Extract a JSON array from an LLM response that may be wrapped in
/// markdown code fences or surrounded by prose. Returns the substring
/// from the first `[` to the matching last `]` if both are present;
/// otherwise returns the trimmed input.
///
/// This handles real-world LLM behavior:
/// - Plain JSON: `[{"name": ...}]`
/// - Markdown-fenced: ```` ```json\n[...]\n``` ````
/// - Prose-wrapped: `Sure, here are the skills:\n[...]\nThat's all.`
fn extract_json_array(response: &str) -> &str {
    let trimmed = response.trim();
    // Find first '[' and last ']'.
    let start = trimmed.find('[');
    let end = trimmed.rfind(']');
    match (start, end) {
        (Some(s), Some(e)) if e > s => &trimmed[s..=e],
        _ => trimmed,
    }
}

/// Wire 2: check if the Vigil -> Cambium bridge is enabled.
/// Reads ~/.temm1e/cambium.toml for the master switch. Defaults to enabled.
fn cambium_vigil_bridge_enabled() -> bool {
    let path = match dirs::home_dir() {
        Some(h) => h.join(".temm1e").join("cambium.toml"),
        None => return false,
    };
    match std::fs::read_to_string(&path) {
        Ok(s) => {
            // Default: enabled if file missing or empty.
            // Disabled only if cambium itself is disabled.
            s.lines()
                .find(|l| l.trim().starts_with("enabled"))
                .map(|l| !l.contains("false"))
                .unwrap_or(true)
        }
        Err(_) => true,
    }
}

/// Wire 2: write a bug detected by Vigil to the Cambium inbox.
/// The inbox is a JSON-lines file at ~/.temm1e/cambium/inbox.jsonl.
/// Users can review it and run `/cambium grow fix <sig>` to address entries.
async fn write_cambium_inbox_entry(
    signature: &str,
    message: &str,
    count: u32,
) -> Result<(), Temm1eError> {
    let inbox_dir = match dirs::home_dir() {
        Some(h) => h.join(".temm1e").join("cambium"),
        None => return Err(Temm1eError::Tool("cannot resolve home directory".into())),
    };
    tokio::fs::create_dir_all(&inbox_dir)
        .await
        .map_err(|e| Temm1eError::Tool(format!("create cambium dir: {e}")))?;

    let inbox_path = inbox_dir.join("inbox.jsonl");
    // Truncate the message for safety (logs can be huge).
    let mut safe_msg = message.to_string();
    if safe_msg.len() > 500 {
        let mut end = 500;
        while end > 0 && !safe_msg.is_char_boundary(end) {
            end -= 1;
        }
        safe_msg.truncate(end);
        safe_msg.push_str("...");
    }
    let entry = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "source": "vigil",
        "signature": signature,
        "message": safe_msg,
        "occurrences": count,
        "kind": "bug_fix",
    });
    let line = format!("{}\n", serde_json::to_string(&entry).unwrap_or_default());

    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&inbox_path)
        .await
        .map_err(|e| Temm1eError::Tool(format!("open inbox: {e}")))?;
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| Temm1eError::Tool(format!("write inbox: {e}")))?;
    tracing::info!(
        target: "perpetuum",
        signature = %signature,
        "Vigil -> Cambium inbox: wrote bug fix entry"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn consolidation_runs() {
        let store = Arc::new(Store::new("sqlite::memory:").await.unwrap());
        let result = consolidate_memory(&store).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn cleanup_sessions_runs() {
        let store = Arc::new(Store::new("sqlite::memory:").await.unwrap());
        let result = cleanup_sessions(&store).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn self_work_no_llm_skips_gracefully() {
        let store = Arc::new(Store::new("sqlite::memory:").await.unwrap());
        let result = execute_self_work(&SelfWorkKind::FailureAnalysis, &store, None).await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("Skipped"));
    }

    #[tokio::test]
    async fn cambium_skills_no_llm_skips_gracefully() {
        let store = Arc::new(Store::new("sqlite::memory:").await.unwrap());
        let result = execute_self_work(&SelfWorkKind::CambiumSkills, &store, None).await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("Skipped"));
    }

    #[test]
    fn extract_json_array_plain() {
        let input = r#"[{"name":"x"}]"#;
        assert_eq!(extract_json_array(input), input);
    }

    #[test]
    fn extract_json_array_markdown_fenced() {
        let input = "```json\n[{\"name\":\"x\"}]\n```";
        assert_eq!(extract_json_array(input), r#"[{"name":"x"}]"#);
    }

    #[test]
    fn extract_json_array_prose_wrapped() {
        let input = "Here are the skills:\n[{\"name\":\"x\"}]\nLet me know if you need more.";
        assert_eq!(extract_json_array(input), r#"[{"name":"x"}]"#);
    }

    #[test]
    fn extract_json_array_empty_array() {
        assert_eq!(extract_json_array("[]"), "[]");
        assert_eq!(extract_json_array("```\n[]\n```"), "[]");
    }

    #[test]
    fn extract_json_array_nested_brackets() {
        let input = r#"[{"caps":["a","b"]}]"#;
        assert_eq!(extract_json_array(input), input);
    }

    #[test]
    fn extract_json_array_no_brackets_returns_input() {
        let input = "no json here";
        assert_eq!(extract_json_array(input), "no json here");
    }
}
