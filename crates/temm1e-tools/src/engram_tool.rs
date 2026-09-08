//! Engram tool — natural-language permanent memory.
//!
//! The agent calls this in-loop (when the user says "remember…") and at
//! end-of-loop (to persist a durable learning). Permanence is curated by the
//! Engram scoring engine + the post-run curator; this tool is the write path.
//!
//! Actions:
//! - `remember` — store a durable fact (upsert; a `subject_key` supersedes an
//!   earlier fact with the same key). User-stated facts pin (`pinned=user`,
//!   sacrosanct); agent self-notes use `pinned=agent` (auto-demotable).
//! - `recall`   — search permanent facts visible in this scope.
//! - `forget`   — delete the best-matching fact for a query.
//!
//! Scopes: global (legacy default), chat, or the authenticated user's scope.
//! User identity comes from ToolContext, never from model-supplied arguments.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use async_trait::async_trait;
use temm1e_core::types::error::Temm1eError;
use temm1e_core::{
    EngramFact, FactType, Memory, MemoryScope, PinnedBy, Tool, ToolContext, ToolDeclarations,
    ToolInput, ToolOutput,
};

pub struct EngramTool {
    memory: Arc<dyn Memory>,
}

impl EngramTool {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn parse_scope(scope: &str, ctx: &ToolContext) -> Result<MemoryScope, Temm1eError> {
        match scope {
            "global" => Ok(MemoryScope::Global),
            "chat" => Ok(MemoryScope::Chat(ctx.chat_id.clone())),
            "user" if !ctx.user_id.is_empty() => Ok(MemoryScope::User(ctx.user_id.clone())),
            "user" => Err(Temm1eError::Tool(
                "User-scoped memory requires an authenticated user identity".into(),
            )),
            _ => Err(Temm1eError::Tool(
                "Unknown memory scope; use global, user or chat".into(),
            )),
        }
    }

    fn parse_type(t: &str) -> FactType {
        match t {
            "identity" => FactType::Identity,
            "preference" => FactType::Preference,
            "project" => FactType::Project,
            "constraint" => FactType::Constraint,
            _ => FactType::Reference,
        }
    }

    /// Stable id: derived from scope + subject_key (so a newer fact with the
    /// same subject supersedes via upsert), or from scope + content otherwise.
    /// `DefaultHasher` uses fixed keys, so ids are stable across runs/sessions.
    fn make_id(scope: &MemoryScope, subject_key: Option<&str>, content: &str) -> String {
        let basis = match subject_key {
            Some(s) => format!("{}|subj|{s}", scope.as_key()),
            None => format!("{}|body|{content}", scope.as_key()),
        };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        basis.hash(&mut h);
        format!("eg{:016x}", h.finish())
    }

    async fn handle_remember(
        &self,
        input: &serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: content".into()))?;
        let scope_s = input
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("global");
        let type_s = input
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("reference");
        let subject_key = input.get("subject_key").and_then(|v| v.as_str());
        let pinned = match input.get("pinned").and_then(|v| v.as_str()) {
            Some("agent") => PinnedBy::Agent,
            _ => PinnedBy::User,
        };
        let importance = input
            .get("importance")
            .and_then(|v| v.as_f64())
            .map(|x| x as f32)
            .unwrap_or(if pinned == PinnedBy::User { 5.0 } else { 4.0 })
            .clamp(0.0, 5.0);
        let tags: Vec<String> = input
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let scope = Self::parse_scope(scope_s, ctx)?;
        let id = Self::make_id(&scope, subject_key, content);
        let now = Self::now();
        let summary: String = content
            .lines()
            .next()
            .unwrap_or(content)
            .chars()
            .take(160)
            .collect();
        let essence: String = summary
            .split_whitespace()
            .take(6)
            .collect::<Vec<_>>()
            .join(" ");

        let fact = EngramFact {
            id: id.clone(),
            content: content.to_string(),
            summary,
            essence,
            fact_type: Self::parse_type(type_s),
            scope,
            pinned_by: pinned,
            subject_key: subject_key.map(String::from),
            importance,
            created_at: now,
            last_accessed: now,
            tags,
            links: Vec::new(),
        };
        self.memory.engram_store(fact).await?;
        tracing::info!(id = %id, scope = %scope_s, "Engram fact remembered");

        Ok(ToolOutput {
            content: format!(
                "Remembered permanently (scope: {scope_s}): \"{content}\". \
                 Briefly let the user know you'll remember this.",
            ),
            is_error: false,
        })
    }

    async fn handle_recall(
        &self,
        input: &serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let query = input.get("query").and_then(|v| v.as_str()).unwrap_or("");
        // User facts include those captured by the automatic curator.
        let facts = self
            .memory
            .engram_recall(query, &ctx.user_id, &ctx.chat_id, 20)
            .await?;
        if facts.is_empty() {
            return Ok(ToolOutput {
                content: "No permanent facts found matching that.".to_string(),
                is_error: false,
            });
        }
        let mut out = format!("Found {} permanent fact(s):\n", facts.len());
        for f in &facts {
            out.push_str(&format!("\n  - [{}] {}", f.id, f.content));
        }
        Ok(ToolOutput {
            content: out,
            is_error: false,
        })
    }

    async fn handle_forget(
        &self,
        input: &serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let id = input
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let fact = match (id, query) {
            (Some(id), None) => {
                self.memory
                    .engram_get(id)
                    .await?
                    .filter(|fact| match &fact.scope {
                        MemoryScope::Global => true,
                        MemoryScope::User(user) => !ctx.user_id.is_empty() && user == &ctx.user_id,
                        MemoryScope::Chat(chat) => chat == &ctx.chat_id,
                    })
            }
            (None, Some(query)) => {
                let mut matches = self
                    .memory
                    .engram_recall(query, &ctx.user_id, &ctx.chat_id, 2)
                    .await?;
                if matches.len() > 1 {
                    return Ok(ToolOutput {
                        content: "Multiple permanent facts match; nothing was deleted. Use recall to inspect their IDs, then forget one exact id.".into(),
                        is_error: true,
                    });
                }
                matches.pop()
            }
            _ => {
                return Err(Temm1eError::Tool(
                    "Provide exactly one nonempty id or query for forget".into(),
                ))
            }
        };
        let Some(fact) = fact else {
            return Ok(ToolOutput {
                content: "No matching visible permanent fact. Nothing removed.".into(),
                is_error: false,
            });
        };
        if !self
            .memory
            .engram_forget_scoped(&fact, &ctx.user_id, &ctx.chat_id)
            .await?
        {
            return Ok(ToolOutput { content: "The selected fact changed or is no longer visible. Nothing removed; recall it again before deleting.".into(), is_error: true });
        }
        Ok(ToolOutput {
            content: format!("Forgotten: \"{}\".", fact.content),
            is_error: false,
        })
    }
}

#[async_trait]
impl Tool for EngramTool {
    fn name(&self) -> &str {
        "engram"
    }

    fn description(&self) -> &str {
        "Your permanent long-term memory for durable facts about the user and project \
         (identity, preferences, standing constraints, project details). Use 'remember' \
         when the user says to remember something OR when you learn a durable fact worth \
         keeping across sessions; reuse the same 'subject_key' to update/correct a fact. \
         'recall' to inspect facts and IDs, 'forget' to remove one exact id or unambiguous query. \
         Eligible facts are injected within the memory budget when enabled; only remember durable facts."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["remember", "recall", "forget"],
                    "description": "The permanent-memory operation. For forget, supply one exact id or an unambiguous query."
                },
                "content": {
                    "type": "string",
                    "description": "The durable fact to remember (required for 'remember')."
                },
                "query": {
                    "type": "string",
                    "description": "Search text (required for 'recall' and 'forget')."
                },
                "type": {
                    "type": "string",
                    "enum": ["identity", "preference", "project", "constraint", "reference"],
                    "description": "Category of the fact (default 'reference')."
                },
                "subject_key": {
                    "type": "string",
                    "description": "Stable key (e.g. 'pref:gpu-provider'); reuse it to update/supersede an earlier fact."
                },
                "id": {"type": "string", "description": "Exact fact ID from recall; use instead of query for precise deletion."},
                "scope": {
                    "type": "string",
                    "enum": ["global", "user", "chat"],
                    "description": "'global' (legacy default) is shared; 'user' uses your authenticated caller, and 'chat' is limited to this conversation."
                },
                "pinned": {
                    "type": "string",
                    "enum": ["user", "agent"],
                    "description": "'user' (default) = the user asked, never auto-forgotten; 'agent' = your own note, may fade if unused."
                }
            },
            "required": ["action"]
        })
    }

    fn declarations(&self) -> ToolDeclarations {
        ToolDeclarations {
            file_access: Vec::new(),
            network_access: Vec::new(),
            shell_access: false,
        }
    }

    async fn execute(
        &self,
        input: ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        if !self.memory.supports_engram() {
            return Err(Temm1eError::Tool("The configured memory backend does not support Engram; no permanent fact was stored or deleted".into()));
        }
        let action = input
            .arguments
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: action".into()))?;
        tracing::info!(action = %action, "Executing engram tool");
        match action {
            "remember" => self.handle_remember(&input.arguments, ctx).await,
            "recall" => self.handle_recall(&input.arguments, ctx).await,
            "forget" => self.handle_forget(&input.arguments, ctx).await,
            _ => Ok(ToolOutput {
                content: format!(
                    "Unknown action '{action}'. Valid actions: remember, recall, forget"
                ),
                is_error: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use temm1e_test_utils::MockMemory;

    fn ctx() -> ToolContext {
        ToolContext {
            user_id: "test-user".into(),
            role: temm1e_core::types::rbac::Role::Admin,
            channel: "cli".into(),
            workspace_path: PathBuf::from("/tmp/test"),
            session_id: "tg-123".to_string(),
            chat_id: "chat-1".to_string(),
            read_tracker: None,
        }
    }

    fn input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            name: "engram".to_string(),
            arguments: args,
        }
    }

    #[tokio::test]
    async fn remember_requires_content() {
        let tool = EngramTool::new(Arc::new(
            temm1e_memory::SqliteMemory::new("sqlite::memory:")
                .await
                .unwrap(),
        ));
        let r = tool
            .execute(input(serde_json::json!({"action": "remember"})), &ctx())
            .await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn remember_succeeds_and_reports() {
        let memory = Arc::new(
            temm1e_memory::SqliteMemory::new("sqlite::memory:")
                .await
                .unwrap(),
        );
        let tool = EngramTool::new(memory.clone());
        let out = tool
            .execute(
                input(serde_json::json!({
                    "action": "remember",
                    "content": "User's birthday is 1994-03-02",
                    "type": "identity"
                })),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("Remembered permanently"));
        let persisted = memory.engram_list("test-user", "chat-1", 10).await.unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].content, "User's birthday is 1994-03-02");
    }

    #[tokio::test]
    async fn unknown_action_is_error() {
        let tool = EngramTool::new(Arc::new(
            temm1e_memory::SqliteMemory::new("sqlite::memory:")
                .await
                .unwrap(),
        ));
        let out = tool
            .execute(input(serde_json::json!({"action": "wat"})), &ctx())
            .await
            .unwrap();
        assert!(out.is_error);
    }

    #[test]
    fn make_id_is_stable_and_subject_keyed() {
        let s = MemoryScope::Global;
        // same subject -> same id (supersession via upsert)
        assert_eq!(
            EngramTool::make_id(&s, Some("pref:gpu"), "Lambda"),
            EngramTool::make_id(&s, Some("pref:gpu"), "RunPod")
        );
        // different content, no subject -> different id
        assert_ne!(
            EngramTool::make_id(&s, None, "a"),
            EngramTool::make_id(&s, None, "b")
        );
    }
    #[test]
    fn user_scope_uses_context_identity_and_unknown_scopes_fail() {
        let mut context = ctx();
        assert_eq!(
            EngramTool::parse_scope("user", &context).unwrap(),
            MemoryScope::User("test-user".into())
        );
        assert_eq!(
            EngramTool::parse_scope("global", &context).unwrap(),
            MemoryScope::Global
        );
        assert!(EngramTool::parse_scope("users", &context).is_err());
        context.user_id.clear();
        assert!(EngramTool::parse_scope("user", &context).is_err());
    }
    #[tokio::test]
    async fn unsupported_backend_never_reports_a_successful_permanent_write() {
        let directory = tempfile::tempdir().unwrap();
        let memory = Arc::new(
            temm1e_memory::MarkdownMemory::new(directory.path())
                .await
                .unwrap(),
        );
        assert!(!memory.supports_engram());
        memory
            .store(temm1e_core::MemoryEntry {
                id: "generic-note".into(),
                content: "Existing generic Markdown memory".into(),
                metadata: serde_json::json!({}),
                timestamp: chrono::Utc::now(),
                session_id: None,
                entry_type: temm1e_core::MemoryEntryType::LongTerm,
            })
            .await
            .unwrap();
        let before = std::fs::read(directory.path().join("MEMORY.md")).unwrap();
        let error = EngramTool::new(memory.clone())
            .execute(
                input(serde_json::json!({
                    "action":"remember", "content":"Never stored as an Engram fact"
                })),
                &ctx(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("does not support"));
        assert_eq!(
            std::fs::read(directory.path().join("MEMORY.md")).unwrap(),
            before
        );
        assert!(memory.engram_forget("missing").await.is_err());
        let fact = EngramFact {
            id: "unsupported-fact".into(),
            content: "not stored".into(),
            summary: "not stored".into(),
            essence: "not stored".into(),
            fact_type: FactType::Reference,
            scope: MemoryScope::Global,
            pinned_by: PinnedBy::User,
            subject_key: None,
            importance: 5.0,
            created_at: 0,
            last_accessed: 0,
            tags: vec![],
            links: vec![],
        };
        assert!(memory.engram_store(fact.clone()).await.is_err());
        assert_eq!(
            std::fs::read(directory.path().join("MEMORY.md")).unwrap(),
            before
        );
        let unsupported = MockMemory::new();
        assert!(unsupported.engram_store(fact).await.is_err());
        assert!(unsupported
            .engram_recall("anything", "alice", "room", 10)
            .await
            .is_err());
    }
}
