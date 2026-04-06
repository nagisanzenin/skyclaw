//! Skill invocation tool — lets the agent discover and use installed skills.
//!
//! Two actions:
//! - `list`   — returns all available skills with names, descriptions, and versions
//! - `invoke` — returns the full instructions for a named skill
//!
//! Skills are Markdown files with YAML frontmatter loaded from
//! `~/.temm1e/skills/` (global) and `<workspace>/skills/` (per-project).

use std::sync::Arc;

use async_trait::async_trait;
use temm1e_core::types::error::Temm1eError;
use temm1e_core::{Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};
use temm1e_skills::SkillRegistry;
use tokio::sync::RwLock;

pub struct SkillTool {
    registry: Arc<RwLock<SkillRegistry>>,
}

impl SkillTool {
    pub fn new(registry: Arc<RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "use_skill"
    }

    fn description(&self) -> &str {
        "Discover and invoke installed skills. Three actions: \
         'list' = skill names + one-line descriptions (lightweight catalog), \
         'info' = full metadata for a specific skill (version, capabilities), \
         'invoke' = get the complete skill instructions to follow."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "info", "invoke"],
                    "description": "Action: 'list' = catalog of names + descriptions, 'info' = metadata for a skill, 'invoke' = full instructions"
                },
                "name": {
                    "type": "string",
                    "description": "Skill name (required for 'info' and 'invoke')"
                }
            },
            "required": ["action"]
        })
    }

    fn declarations(&self) -> ToolDeclarations {
        ToolDeclarations {
            file_access: vec![],
            network_access: vec![],
            shell_access: false,
        }
    }

    async fn execute(
        &self,
        input: ToolInput,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let args = &input.arguments;
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");

        match action {
            // Layer 1: Catalog — names + one-line descriptions only.
            // Cheap to scan. Agent uses this to decide which skill to look at.
            "list" => {
                let reg = self.registry.read().await;
                let skills = reg.list_skills();
                if skills.is_empty() {
                    return Ok(ToolOutput {
                        content: "No skills installed. Place .md skill files in ~/.temm1e/skills/ or <workspace>/skills/ to add skills.".to_string(),
                        is_error: false,
                    });
                }
                let mut out = format!("{} skill(s) available:\n", skills.len());
                for s in skills {
                    out.push_str(&format!("- {} — {}\n", s.name, s.description));
                }
                Ok(ToolOutput {
                    content: out,
                    is_error: false,
                })
            }
            // Layer 2: Summary — version, capabilities, description.
            // Agent uses this to confirm a skill is relevant before loading full content.
            "info" => {
                let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                    Temm1eError::Tool("Missing required parameter 'name' for info action".into())
                })?;

                let reg = self.registry.read().await;
                match reg.get_skill(name) {
                    Some(skill) => Ok(ToolOutput {
                        content: format!(
                            "{} (v{})\n{}\ncapabilities: {}",
                            skill.name,
                            skill.version,
                            skill.description,
                            skill.capabilities.join(", ")
                        ),
                        is_error: false,
                    }),
                    None => skill_not_found(name, &reg),
                }
            }
            // Layer 3: Full content — complete skill instructions.
            // Agent invokes this when it has decided to use the skill.
            "invoke" => {
                let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                    Temm1eError::Tool("Missing required parameter 'name' for invoke action".into())
                })?;

                let reg = self.registry.read().await;
                match reg.get_skill(name) {
                    Some(skill) => Ok(ToolOutput {
                        content: format!(
                            "=== SKILL: {} (v{}) ===\n{}\n\n{}\n=== END SKILL ===",
                            skill.name, skill.version, skill.description, skill.instructions
                        ),
                        is_error: false,
                    }),
                    None => skill_not_found(name, &reg),
                }
            }
            other => Ok(ToolOutput {
                content: format!(
                    "Unknown action '{}'. Valid actions: 'list', 'info', 'invoke'",
                    other
                ),
                is_error: true,
            }),
        }
    }
}

fn skill_not_found(name: &str, reg: &SkillRegistry) -> Result<ToolOutput, Temm1eError> {
    let available: Vec<&str> = reg.list_skills().iter().map(|s| s.name.as_str()).collect();
    Ok(ToolOutput {
        content: format!(
            "Skill '{}' not found. Available: {}",
            name,
            if available.is_empty() {
                "(none)".to_string()
            } else {
                available.join(", ")
            }
        ),
        is_error: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as std_fs;
    use tempfile::tempdir;

    fn write_skill(dir: &std::path::Path, filename: &str, name: &str, desc: &str, body: &str) {
        let content = format!(
            "---\nname: {name}\ndescription: {desc}\ncapabilities:\n  - {name}\nversion: 1.0.0\n---\n{body}"
        );
        std_fs::write(dir.join(filename), content).unwrap();
    }

    fn ctx() -> ToolContext {
        ToolContext {
            workspace_path: std::path::PathBuf::from("/tmp"),
            session_id: "test".into(),
            chat_id: "test".into(),
        }
    }

    async fn make_registry(workspace: &std::path::Path) -> Arc<RwLock<SkillRegistry>> {
        let mut reg = SkillRegistry::new(workspace.to_path_buf());
        reg.load_skills().await.unwrap();
        Arc::new(RwLock::new(reg))
    }

    #[tokio::test]
    async fn list_empty() {
        let tmp = tempdir().unwrap();
        let reg = make_registry(tmp.path()).await;
        let tool = SkillTool::new(reg);

        let out = tool
            .execute(
                ToolInput {
                    name: "use_skill".into(),
                    arguments: serde_json::json!({"action": "list"}),
                },
                &ctx(),
            )
            .await
            .unwrap();

        assert!(!out.is_error);
        assert!(out.content.contains("No skills installed"));
    }

    #[tokio::test]
    async fn list_with_skills() {
        let tmp = tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std_fs::create_dir_all(&skills_dir).unwrap();
        write_skill(
            &skills_dir,
            "deploy.md",
            "deploy",
            "Deploy to cloud",
            "Run deploy.",
        );

        let reg = make_registry(tmp.path()).await;
        let tool = SkillTool::new(reg);

        let out = tool
            .execute(
                ToolInput {
                    name: "use_skill".into(),
                    arguments: serde_json::json!({"action": "list"}),
                },
                &ctx(),
            )
            .await
            .unwrap();

        assert!(!out.is_error);
        assert!(out.content.contains("deploy"));
        assert!(out.content.contains("Deploy to cloud"));
        assert!(out.content.contains("1 skill(s) available"));
        // Layer 1: catalog should NOT contain capabilities or version (lightweight)
        assert!(!out.content.contains("capabilities"));
        assert!(!out.content.contains("v1.0.0"));
    }

    #[tokio::test]
    async fn info_shows_metadata_not_instructions() {
        let tmp = tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std_fs::create_dir_all(&skills_dir).unwrap();
        write_skill(
            &skills_dir,
            "deploy.md",
            "deploy",
            "Deploy to cloud",
            "Step 1: build\nStep 2: ship",
        );

        let reg = make_registry(tmp.path()).await;
        let tool = SkillTool::new(reg);

        let out = tool
            .execute(
                ToolInput {
                    name: "use_skill".into(),
                    arguments: serde_json::json!({"action": "info", "name": "deploy"}),
                },
                &ctx(),
            )
            .await
            .unwrap();

        assert!(!out.is_error);
        // Layer 2: has version and capabilities
        assert!(out.content.contains("v1.0.0"));
        assert!(out.content.contains("capabilities"));
        assert!(out.content.contains("Deploy to cloud"));
        // Layer 2: does NOT contain full instructions
        assert!(!out.content.contains("Step 1: build"));
    }

    #[tokio::test]
    async fn invoke_existing_skill() {
        let tmp = tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        std_fs::create_dir_all(&skills_dir).unwrap();
        write_skill(
            &skills_dir,
            "deploy.md",
            "deploy",
            "Deploy to cloud",
            "Step 1: build\nStep 2: ship",
        );

        let reg = make_registry(tmp.path()).await;
        let tool = SkillTool::new(reg);

        let out = tool
            .execute(
                ToolInput {
                    name: "use_skill".into(),
                    arguments: serde_json::json!({"action": "invoke", "name": "deploy"}),
                },
                &ctx(),
            )
            .await
            .unwrap();

        assert!(!out.is_error);
        assert!(out.content.contains("SKILL: deploy"));
        assert!(out.content.contains("Step 1: build"));
        assert!(out.content.contains("Step 2: ship"));
    }

    #[tokio::test]
    async fn invoke_missing_skill() {
        let tmp = tempdir().unwrap();
        let reg = make_registry(tmp.path()).await;
        let tool = SkillTool::new(reg);

        let out = tool
            .execute(
                ToolInput {
                    name: "use_skill".into(),
                    arguments: serde_json::json!({"action": "invoke", "name": "nope"}),
                },
                &ctx(),
            )
            .await
            .unwrap();

        assert!(out.is_error);
        assert!(out.content.contains("not found"));
    }

    #[tokio::test]
    async fn invoke_missing_name_param() {
        let tmp = tempdir().unwrap();
        let reg = make_registry(tmp.path()).await;
        let tool = SkillTool::new(reg);

        let result = tool
            .execute(
                ToolInput {
                    name: "use_skill".into(),
                    arguments: serde_json::json!({"action": "invoke"}),
                },
                &ctx(),
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn unknown_action() {
        let tmp = tempdir().unwrap();
        let reg = make_registry(tmp.path()).await;
        let tool = SkillTool::new(reg);

        let out = tool
            .execute(
                ToolInput {
                    name: "use_skill".into(),
                    arguments: serde_json::json!({"action": "delete"}),
                },
                &ctx(),
            )
            .await
            .unwrap();

        assert!(out.is_error);
        assert!(out.content.contains("Unknown action"));
    }
}
