//! # LlmCodeGenerator: production code generator backed by a real LLM provider.
//!
//! This is the implementation of `CodeGenerator` that closes the gap between
//! the verification harness (deterministic, mechanical) and the code-writing
//! capability (probabilistic, LLM-driven). It is the **pluggable** half of
//! Cambium's pluggable-generator-fixed-harness architecture.
//!
//! ## What it does
//!
//! 1. Takes a `GrowthTrigger` (the gap to close), `GrowthKind` (the shape of
//!    the change), and a list of relevant existing files to include as context.
//! 2. Builds a prompt that asks the LLM to generate one or more file changes
//!    in a strict JSON format.
//! 3. Calls the LLM via the `Provider` trait.
//! 4. Parses the response (handles markdown code-fence wrapping and prose).
//! 5. Validates each file change (path safety, no parent traversal, no
//!    absolute paths) and writes it into the sandbox.
//!
//! ## What it does NOT do
//!
//! - It does not run cargo or any verification — that is the pipeline's job.
//! - It does not decide which files to include as context — the caller does
//!   (typically by reading the trigger and the codebase self-model).
//! - It does not retry on parse failure — that is the pipeline's
//!   `CodeGeneration` stage with `MAX_STAGE_RETRIES`.
//! - It does not commit anything — the sandbox handles git.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use temm1e_core::traits::Provider;
use temm1e_core::types::cambium::{GrowthKind, GrowthTrigger};
use temm1e_core::types::message::{
    ChatMessage, CompletionRequest, ContentPart, MessageContent, Role,
};

use crate::pipeline::CodeGenerator;
use crate::sandbox::Sandbox;

/// A single file change in the LLM's response.
#[derive(Debug, Deserialize)]
struct FileChange {
    /// Sandbox-relative path. Must not escape the sandbox.
    path: String,
    /// "create" for new files, "modify" for replacing an existing file.
    #[serde(default = "default_action")]
    action: String,
    /// The full content of the file after the change.
    content: String,
}

fn default_action() -> String {
    "modify".to_string()
}

/// Production LlmCodeGenerator backed by any TEMM1E `Provider`.
pub struct LlmCodeGenerator {
    provider: Arc<dyn Provider>,
    model: String,
    /// Optional fixed file contents to include as context in every call.
    /// In production this is populated from the codebase self-model.
    context_files: Vec<(String, String)>,
    /// Maximum number of files the LLM is allowed to change in one session.
    max_files: usize,
}

impl LlmCodeGenerator {
    pub fn new(provider: Arc<dyn Provider>, model: String) -> Self {
        Self {
            provider,
            model,
            context_files: Vec::new(),
            max_files: 5,
        }
    }

    /// Include a file's content as context for the LLM call.
    /// Use this to give the model existing code to read before it writes.
    pub fn with_context_file(mut self, path: String, content: String) -> Self {
        self.context_files.push((path, content));
        self
    }

    /// Set the maximum files the LLM may modify in one session.
    pub fn with_max_files(mut self, max: usize) -> Self {
        self.max_files = max;
        self
    }

    /// Build the system prompt for code generation.
    fn system_prompt(&self) -> String {
        "You are Cambium, the self-grow code generator for the TEMM1E project. \
         Your job is to write Rust code that closes a specific gap. \
         \n\n\
         Constraints (these are hard requirements):\n\
         1. The code MUST compile under `cargo check`.\n\
         2. The code MUST pass `cargo clippy --all-targets -- -D warnings`.\n\
         3. The code MUST pass `cargo test`.\n\
         4. NO `unsafe` blocks under any circumstances.\n\
         5. Every new public function MUST have at least one `#[cfg(test)]` test.\n\
         6. NO new external crate dependencies (do not add to Cargo.toml).\n\
         7. Follow Rust 2021 edition conventions.\n\
         \n\
         Response format: respond with ONLY a JSON array of file changes, no prose, no markdown fences. \
         Each element has the shape:\n\
         {\"path\": \"<sandbox-relative-path>\", \"action\": \"create\"|\"modify\", \"content\": \"<full file content>\"}\n\
         For \"modify\", the content must be the COMPLETE file after the change, not a diff. \
         For \"create\", path must not exist yet."
            .to_string()
    }

    /// Build the user prompt with task and context.
    fn user_prompt(&self, trigger: &GrowthTrigger, kind: &GrowthKind) -> String {
        let task = describe_trigger(trigger);
        let kind_str = describe_kind(kind);
        let mut prompt = format!(
            "Task ({kind_str}): {task}\n\n\
             Constraints:\n\
             - You may modify or create at most {} files in this session.\n\
             - Stay within the temm1e-cambium crate unless the task requires otherwise.\n\
             - Tests must be in #[cfg(test)] mod tests blocks at the bottom of the file.\n\
             - Use #[tokio::test] for async tests.\n\n",
            self.max_files
        );

        if !self.context_files.is_empty() {
            prompt.push_str("Existing files for context:\n\n");
            for (path, content) in &self.context_files {
                prompt.push_str(&format!(
                    "=== {path} ===\n{content}\n=== end {path} ===\n\n"
                ));
            }
        }

        prompt.push_str(
            "Now respond with the JSON array of file changes. \
             Remember: ONLY the JSON, no markdown fences, no prose.",
        );
        prompt
    }

    /// Extract the JSON array from a possibly-wrapped LLM response.
    fn extract_json_array(response: &str) -> &str {
        let trimmed = response.trim();
        let start = trimmed.find('[');
        let end = trimmed.rfind(']');
        match (start, end) {
            (Some(s), Some(e)) if e > s => &trimmed[s..=e],
            _ => trimmed,
        }
    }
}

#[async_trait]
impl CodeGenerator for LlmCodeGenerator {
    async fn generate(
        &self,
        sandbox: &Sandbox,
        trigger: &GrowthTrigger,
        kind: &GrowthKind,
    ) -> Result<(), String> {
        let system = self.system_prompt();
        let user = self.user_prompt(trigger, kind);

        let request = CompletionRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: MessageContent::Text(user),
            }],
            tools: vec![],
            max_tokens: None,
            temperature: Some(0.2),
            system: Some(system),
            system_volatile: None,
        };

        let response = self
            .provider
            .complete(request)
            .await
            .map_err(|e| format!("LLM call failed: {e}"))?;

        // Extract text from response content parts
        let text: String = response
            .content
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        if text.trim().is_empty() {
            return Err("LLM returned empty response".to_string());
        }

        let extracted = Self::extract_json_array(&text);
        let changes: Vec<FileChange> = serde_json::from_str(extracted).map_err(|e| {
            format!(
                "Failed to parse LLM response as JSON array of file changes: {e}\nResponse was:\n{text}"
            )
        })?;

        if changes.is_empty() {
            return Err("LLM returned empty file changes array".to_string());
        }

        if changes.len() > self.max_files {
            return Err(format!(
                "LLM proposed {} files, exceeds max of {}",
                changes.len(),
                self.max_files
            ));
        }

        // Validate and write each change.
        for change in &changes {
            let path = std::path::Path::new(&change.path);

            // Reject path traversal and absolute paths (sandbox.write_file
            // also enforces this, but we want a clearer error message here).
            if path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(format!("Path traversal rejected: {}", change.path));
            }
            if path.is_absolute() {
                return Err(format!("Absolute path rejected: {}", change.path));
            }
            if change.content.is_empty() {
                return Err(format!("Empty content for {}", change.path));
            }

            // Check for unsafe blocks (basic textual check; clippy will catch
            // anything sneakier later in the pipeline).
            if change.content.contains("unsafe ") || change.content.contains("unsafe{") {
                return Err(format!(
                    "Unsafe block detected in {} — Cambium rejects all unsafe code",
                    change.path
                ));
            }

            // Write into the sandbox. Sandbox::write_file enforces its own
            // path safety; this is defense in depth.
            sandbox
                .write_file(path, &change.content)
                .await
                .map_err(|e| format!("Failed to write {}: {e}", change.path))?;

            tracing::info!(
                target: "cambium",
                path = %change.path,
                action = %change.action,
                bytes = change.content.len(),
                "LlmCodeGenerator wrote file"
            );
        }

        Ok(())
    }
}

fn describe_trigger(trigger: &GrowthTrigger) -> String {
    match trigger {
        GrowthTrigger::BugDetected {
            error_signature,
            occurrences,
        } => format!("Fix the bug with signature '{error_signature}' (seen {occurrences} times)"),
        GrowthTrigger::UserRequest { description, .. } => description.clone(),
        GrowthTrigger::QualityDegradation {
            metric,
            current,
            threshold,
        } => format!("Improve metric '{metric}' (currently {current}, threshold {threshold})"),
        GrowthTrigger::UserCorrection { pattern, frequency } => {
            format!(
                "Address recurring user correction pattern '{pattern}' (seen {frequency} times)"
            )
        }
        GrowthTrigger::Manual { description } => description.clone(),
    }
}

fn describe_kind(kind: &GrowthKind) -> &'static str {
    match kind {
        GrowthKind::NewTool => "new tool",
        GrowthKind::BugFix => "bug fix",
        GrowthKind::Optimization => "optimization",
        GrowthKind::NewSkill => "new skill",
        GrowthKind::NewIntegration => "new integration",
        GrowthKind::NewCore => "new core",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_array_plain() {
        let input = r#"[{"path":"x.rs","content":"y"}]"#;
        assert_eq!(LlmCodeGenerator::extract_json_array(input), input);
    }

    #[test]
    fn extract_json_array_markdown_fenced() {
        let input = "```json\n[{\"path\":\"x.rs\",\"content\":\"y\"}]\n```";
        assert_eq!(
            LlmCodeGenerator::extract_json_array(input),
            r#"[{"path":"x.rs","content":"y"}]"#
        );
    }

    #[test]
    fn extract_json_array_prose_wrapped() {
        let input =
            "Sure, here are the changes:\n[{\"path\":\"x.rs\",\"content\":\"y\"}]\nLet me know.";
        assert_eq!(
            LlmCodeGenerator::extract_json_array(input),
            r#"[{"path":"x.rs","content":"y"}]"#
        );
    }

    #[test]
    fn describe_trigger_user_request() {
        let trigger = GrowthTrigger::UserRequest {
            description: "add a hello function".to_string(),
            chat_id: "1".to_string(),
        };
        assert_eq!(describe_trigger(&trigger), "add a hello function");
    }

    #[test]
    fn describe_kind_returns_static_str() {
        assert_eq!(describe_kind(&GrowthKind::NewTool), "new tool");
        assert_eq!(describe_kind(&GrowthKind::BugFix), "bug fix");
    }
}
