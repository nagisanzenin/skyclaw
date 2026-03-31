use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveTime, Utc};
use temm1e_core::traits::{Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};
use temm1e_core::types::error::Temm1eError;

use crate::cortex::Cortex;
use crate::types::{ConcernConfig, ExtractMode, MonitorCheck, Schedule};

/// Create all Perpetuum agent tools.
pub fn create_tools(cortex: Arc<Cortex>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(CreateAlarmTool(cortex.clone())),
        Arc::new(CreateMonitorTool(cortex.clone())),
        Arc::new(CreateRecurringTool(cortex.clone())),
        Arc::new(ListConcernsTool(cortex.clone())),
        Arc::new(CancelConcernTool(cortex.clone())),
        Arc::new(AdjustScheduleTool(cortex)),
    ]
}

fn no_access() -> ToolDeclarations {
    ToolDeclarations {
        file_access: vec![],
        network_access: vec![],
        shell_access: false,
    }
}

// ---- CreateAlarm ----

struct CreateAlarmTool(Arc<Cortex>);

#[async_trait]
impl Tool for CreateAlarmTool {
    fn name(&self) -> &str {
        "create_alarm"
    }
    fn description(&self) -> &str {
        "Create a one-time alarm that fires at a specific time and sends a message to the user. Supports absolute time (ISO 8601), relative time (5m, 2h, 1d), or time of day (6:30 AM)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Short name for this alarm"},
                "fire_at": {"type": "string", "description": "When to fire: ISO 8601, relative (5m, 2h), or time of day (6:30 AM)"},
                "message": {"type": "string", "description": "Message to send when alarm fires"}
            },
            "required": ["name", "fire_at", "message"]
        })
    }
    fn declarations(&self) -> ToolDeclarations {
        no_access()
    }
    async fn execute(
        &self,
        input: ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let args = &input.arguments;
        let name = args["name"].as_str().unwrap_or("alarm");
        let fire_at_str = args["fire_at"].as_str().unwrap_or("");
        let message = args["message"].as_str().unwrap_or("Alarm!");

        let fire_at = parse_time_expression(fire_at_str)?;

        let id = self
            .0
            .create_concern(
                ConcernConfig::Alarm {
                    name: name.to_string(),
                    fire_at,
                    message: message.to_string(),
                    notify_chat_id: ctx.chat_id.clone(),
                    notify_channel: None,
                },
                "user",
            )
            .await?;

        Ok(ToolOutput {
            content: format!(
                "Alarm \"{}\" set for {} (ID: {})",
                name,
                fire_at.format("%Y-%m-%d %H:%M:%S UTC"),
                id
            ),
            is_error: false,
        })
    }
}

// ---- CreateMonitor ----

struct CreateMonitorTool(Arc<Cortex>);

#[async_trait]
impl Tool for CreateMonitorTool {
    fn name(&self) -> &str {
        "create_monitor"
    }
    fn description(&self) -> &str {
        "Create a persistent monitor that periodically checks a URL, runs a command, or watches a file, and notifies the user when changes are detected."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Short name for this monitor"},
                "intent": {"type": "string", "description": "What the user wants to monitor and why (stored for LLM interpretation)"},
                "check_type": {"type": "string", "enum": ["web", "command", "file"], "description": "Type of check"},
                "target": {"type": "string", "description": "URL, command, or file path to monitor"},
                "selector": {"type": "string", "description": "CSS selector for web checks (optional)"},
                "interval_secs": {"type": "integer", "description": "Check interval in seconds (default 300 = 5 minutes)"}
            },
            "required": ["name", "intent", "check_type", "target"]
        })
    }
    fn declarations(&self) -> ToolDeclarations {
        ToolDeclarations {
            file_access: vec![],
            network_access: vec!["*".to_string()],
            shell_access: false,
        }
    }
    async fn execute(
        &self,
        input: ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let args = &input.arguments;
        let name = args["name"].as_str().unwrap_or("monitor");
        let intent = args["intent"].as_str().unwrap_or("");
        let check_type = args["check_type"].as_str().unwrap_or("web");
        let target = args["target"].as_str().unwrap_or("");
        let selector = args["selector"].as_str().map(String::from);
        let interval = args["interval_secs"].as_u64().unwrap_or(300);

        let check = match check_type {
            "command" => MonitorCheck::Command {
                command: target.to_string(),
                working_dir: None,
            },
            "file" => MonitorCheck::File {
                path: target.to_string(),
            },
            _ => MonitorCheck::Web {
                url: target.to_string(),
                selector,
                extract: ExtractMode::FullText,
            },
        };

        let id = self
            .0
            .create_concern(
                ConcernConfig::Monitor {
                    name: name.to_string(),
                    user_intent: intent.to_string(),
                    schedule: Schedule::Every(std::time::Duration::from_secs(interval)),
                    check,
                    notify_chat_id: ctx.chat_id.clone(),
                    notify_channel: None,
                },
                "user",
            )
            .await?;

        Ok(ToolOutput {
            content: format!(
                "Monitor \"{}\" created: checking {} every {}s (ID: {})",
                name, target, interval, id
            ),
            is_error: false,
        })
    }
}

// ---- CreateRecurring ----

struct CreateRecurringTool(Arc<Cortex>);

#[async_trait]
impl Tool for CreateRecurringTool {
    fn name(&self) -> &str {
        "create_recurring"
    }
    fn description(&self) -> &str {
        "Create a recurring scheduled task using cron expression (e.g., '0 9 * * 1-5' for weekdays at 9 AM)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Short name for this task"},
                "cron": {"type": "string", "description": "5-field cron expression (min hr dom mon dow)"},
                "action": {"type": "string", "description": "Description of what to do when triggered"}
            },
            "required": ["name", "cron", "action"]
        })
    }
    fn declarations(&self) -> ToolDeclarations {
        no_access()
    }
    async fn execute(
        &self,
        input: ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let args = &input.arguments;
        let name = args["name"].as_str().unwrap_or("recurring");
        let cron_expr = args["cron"].as_str().unwrap_or("0 * * * *");
        let action = args["action"].as_str().unwrap_or("Task");

        // Validate cron expression
        let cron7 = crate::pulse::cron5_to_cron7(cron_expr);
        if cron::Schedule::from_str(&cron7).is_err() {
            return Ok(ToolOutput {
                content: format!("Invalid cron expression: {cron_expr}"),
                is_error: true,
            });
        }

        let id = self
            .0
            .create_concern(
                ConcernConfig::Recurring {
                    name: name.to_string(),
                    cron_expr: cron_expr.to_string(),
                    action_description: action.to_string(),
                    notify_chat_id: ctx.chat_id.clone(),
                    notify_channel: None,
                },
                "user",
            )
            .await?;

        Ok(ToolOutput {
            content: format!(
                "Recurring task \"{}\" created with cron '{}': {} (ID: {})",
                name, cron_expr, action, id
            ),
            is_error: false,
        })
    }
}

// ---- ListConcerns ----

struct ListConcernsTool(Arc<Cortex>);

#[async_trait]
impl Tool for ListConcernsTool {
    fn name(&self) -> &str {
        "list_concerns"
    }
    fn description(&self) -> &str {
        "List all active Perpetuum concerns (alarms, monitors, recurring tasks)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    fn declarations(&self) -> ToolDeclarations {
        no_access()
    }
    async fn execute(
        &self,
        _input: ToolInput,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let concerns = self.0.list_concerns().await;
        if concerns.is_empty() {
            return Ok(ToolOutput {
                content: "No active concerns.".to_string(),
                is_error: false,
            });
        }

        let lines: Vec<String> = concerns
            .iter()
            .map(|c| {
                let sched = c.schedule_desc.as_deref().unwrap_or("once");
                let source_tag = if c.source == "volition" {
                    " [volition]"
                } else {
                    ""
                };
                format!(
                    "- [{}] {} ({}){} — {}",
                    c.id.chars().take(8).collect::<String>(),
                    c.name,
                    c.concern_type,
                    source_tag,
                    sched
                )
            })
            .collect();

        Ok(ToolOutput {
            content: format!("{} active concerns:\n{}", concerns.len(), lines.join("\n")),
            is_error: false,
        })
    }
}

// ---- CancelConcern ----

struct CancelConcernTool(Arc<Cortex>);

#[async_trait]
impl Tool for CancelConcernTool {
    fn name(&self) -> &str {
        "cancel_concern"
    }
    fn description(&self) -> &str {
        "Cancel an active concern (alarm, monitor, or recurring task) by ID."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Concern ID (or first 8 chars)"}
            },
            "required": ["id"]
        })
    }
    fn declarations(&self) -> ToolDeclarations {
        no_access()
    }
    async fn execute(
        &self,
        input: ToolInput,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let id = input.arguments["id"].as_str().unwrap_or("");

        // Support partial ID matching
        let concerns = self.0.list_concerns().await;
        let matched = concerns.iter().find(|c| c.id.starts_with(id) || c.id == id);

        match matched {
            Some(c) => {
                let full_id = c.id.clone();
                let name = c.name.clone();
                self.0.cancel_concern(&full_id).await?;
                Ok(ToolOutput {
                    content: format!("Cancelled concern \"{name}\" (ID: {full_id})"),
                    is_error: false,
                })
            }
            None => Ok(ToolOutput {
                content: format!("No concern found matching ID: {id}"),
                is_error: true,
            }),
        }
    }
}

// ---- AdjustSchedule ----

struct AdjustScheduleTool(Arc<Cortex>);

#[async_trait]
impl Tool for AdjustScheduleTool {
    fn name(&self) -> &str {
        "adjust_schedule"
    }
    fn description(&self) -> &str {
        "Adjust the check interval of an active monitor concern."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Concern ID (or prefix)"},
                "interval_secs": {"type": "integer", "description": "New interval in seconds"}
            },
            "required": ["id", "interval_secs"]
        })
    }
    fn declarations(&self) -> ToolDeclarations {
        no_access()
    }
    async fn execute(
        &self,
        input: ToolInput,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let id = input.arguments["id"].as_str().unwrap_or("");
        let interval = input.arguments["interval_secs"].as_u64().unwrap_or(300);

        let concerns = self.0.list_concerns().await;
        let matched = concerns.iter().find(|c| c.id.starts_with(id) || c.id == id);

        match matched {
            Some(c) => {
                let full_id = c.id.clone();
                self.0
                    .adjust_schedule(
                        &full_id,
                        Schedule::Every(std::time::Duration::from_secs(interval)),
                    )
                    .await?;
                Ok(ToolOutput {
                    content: format!(
                        "Adjusted schedule for \"{}\" to every {}s",
                        c.name, interval
                    ),
                    is_error: false,
                })
            }
            None => Ok(ToolOutput {
                content: format!("No concern found matching ID: {id}"),
                is_error: true,
            }),
        }
    }
}

// ---- Time expression parser ----

/// Parse time expressions: ISO 8601, relative (5m, 2h, 1d), or time of day (6:30 AM).
fn parse_time_expression(expr: &str) -> Result<DateTime<Utc>, Temm1eError> {
    let expr = expr.trim();

    // ISO 8601
    if let Ok(dt) = DateTime::parse_from_rfc3339(expr) {
        return Ok(dt.with_timezone(&Utc));
    }

    // Relative: 5m, 2h, 1d, 30s
    if let Some(duration) = parse_relative(expr) {
        return Ok(Utc::now() + duration);
    }

    // Time of day: "6:30 AM", "14:00", "6 AM"
    if let Some(time) = parse_time_of_day(expr) {
        let today = Utc::now().date_naive();
        let naive = today.and_time(time);
        let dt = naive.and_utc();
        // If the time has passed today, schedule for tomorrow
        if dt <= Utc::now() {
            return Ok(dt + Duration::days(1));
        }
        return Ok(dt);
    }

    Err(Temm1eError::Config(format!(
        "Cannot parse time expression: {expr}"
    )))
}

fn parse_relative(expr: &str) -> Option<Duration> {
    let expr = expr.trim().to_lowercase();
    let (num_str, unit) = if expr.ends_with('s') && !expr.ends_with("cs") {
        (&expr[..expr.len() - 1], "s")
    } else if expr.ends_with('m') {
        (&expr[..expr.len() - 1], "m")
    } else if expr.ends_with('h') {
        (&expr[..expr.len() - 1], "h")
    } else if expr.ends_with('d') {
        (&expr[..expr.len() - 1], "d")
    } else {
        return None;
    };

    let num: i64 = num_str.parse().ok()?;
    match unit {
        "s" => Some(Duration::seconds(num)),
        "m" => Some(Duration::minutes(num)),
        "h" => Some(Duration::hours(num)),
        "d" => Some(Duration::days(num)),
        _ => None,
    }
}

fn parse_time_of_day(expr: &str) -> Option<NaiveTime> {
    let expr = expr.trim().to_uppercase();

    // "6:30 AM", "11:00 PM"
    if expr.contains("AM") || expr.contains("PM") {
        let is_pm = expr.contains("PM");
        let time_part = expr.replace("AM", "").replace("PM", "").trim().to_string();

        let parts: Vec<&str> = time_part.split(':').collect();
        let hour: u32 = parts.first()?.trim().parse().ok()?;
        let minute: u32 = parts
            .get(1)
            .and_then(|m| m.trim().parse().ok())
            .unwrap_or(0);

        let hour24 = if is_pm && hour != 12 {
            hour + 12
        } else if !is_pm && hour == 12 {
            0
        } else {
            hour
        };

        return NaiveTime::from_hms_opt(hour24, minute, 0);
    }

    // "14:00", "6:30"
    let parts: Vec<&str> = expr.split(':').collect();
    if parts.len() == 2 {
        let hour: u32 = parts.first()?.trim().parse().ok()?;
        let minute: u32 = parts.get(1)?.trim().parse().ok()?;
        return NaiveTime::from_hms_opt(hour, minute, 0);
    }

    None
}

use std::str::FromStr;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_relative_durations() {
        assert_eq!(parse_relative("5m"), Some(Duration::minutes(5)));
        assert_eq!(parse_relative("2h"), Some(Duration::hours(2)));
        assert_eq!(parse_relative("1d"), Some(Duration::days(1)));
        assert_eq!(parse_relative("30s"), Some(Duration::seconds(30)));
        assert!(parse_relative("abc").is_none());
    }

    #[test]
    fn parse_time_of_day_12h() {
        let t = parse_time_of_day("6:30 AM").unwrap();
        assert_eq!(t, NaiveTime::from_hms_opt(6, 30, 0).unwrap());

        let t = parse_time_of_day("11:00 PM").unwrap();
        assert_eq!(t, NaiveTime::from_hms_opt(23, 0, 0).unwrap());

        let t = parse_time_of_day("12:00 AM").unwrap();
        assert_eq!(t, NaiveTime::from_hms_opt(0, 0, 0).unwrap());

        let t = parse_time_of_day("12:00 PM").unwrap();
        assert_eq!(t, NaiveTime::from_hms_opt(12, 0, 0).unwrap());
    }

    #[test]
    fn parse_time_of_day_24h() {
        let t = parse_time_of_day("14:00").unwrap();
        assert_eq!(t, NaiveTime::from_hms_opt(14, 0, 0).unwrap());

        let t = parse_time_of_day("6:30").unwrap();
        assert_eq!(t, NaiveTime::from_hms_opt(6, 30, 0).unwrap());
    }

    #[test]
    fn parse_time_expression_relative() {
        let dt = parse_time_expression("5m").unwrap();
        assert!(dt > Utc::now());
        assert!(dt < Utc::now() + Duration::minutes(6));
    }

    #[test]
    fn parse_time_expression_iso() {
        use chrono::Datelike;
        let dt = parse_time_expression("2030-01-01T00:00:00Z").unwrap();
        assert_eq!(dt.year(), 2030);
    }

    #[test]
    fn parse_time_expression_invalid() {
        assert!(parse_time_expression("not a time").is_err());
    }
}
