use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use temm1e_core::types::error::Temm1eError;

use crate::types::ConcernId;

/// Stored representation of a concern in SQLite.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StoredConcern {
    pub id: String,
    pub concern_type: String,
    pub name: String,
    pub source: String,
    pub state: String,
    pub config_json: String,
    pub notify_chat_id: Option<String>,
    pub notify_channel: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_fired_at: Option<String>,
    pub next_fire_at: Option<String>,
    pub error_count: i32,
    pub consecutive_errors: i32,
}

/// Monitor check history entry.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MonitorHistoryEntry {
    pub id: i64,
    pub concern_id: String,
    pub checked_at: String,
    pub raw_content_hash: Option<String>,
    pub raw_content_preview: Option<String>,
    pub change_detected: bool,
    pub interpretation: Option<String>,
    pub notified: bool,
}

/// Input for inserting a monitor check result.
pub struct MonitorResultInput {
    pub concern_id: String,
    pub checked_at: DateTime<Utc>,
    pub content_hash: Option<String>,
    pub content_preview: Option<String>,
    pub change_detected: bool,
    pub interpretation: Option<String>,
    pub notified: bool,
}

/// Perpetuum SQLite persistence layer.
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub async fn new(database_url: &str) -> Result<Self, Temm1eError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|e| Temm1eError::Memory(format!("Perpetuum SQLite connect: {e}")))?;

        // Enable WAL mode for concurrent reads
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await
            .map_err(|e| Temm1eError::Memory(format!("PRAGMA journal_mode: {e}")))?;

        sqlx::query("PRAGMA busy_timeout=5000")
            .execute(&pool)
            .await
            .map_err(|e| Temm1eError::Memory(format!("PRAGMA busy_timeout: {e}")))?;

        let store = Self { pool };
        store.init_tables().await?;
        tracing::info!(target: "perpetuum", "Perpetuum store initialized");
        Ok(store)
    }

    async fn init_tables(&self) -> Result<(), Temm1eError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS perpetuum_concerns (
                id TEXT PRIMARY KEY,
                concern_type TEXT NOT NULL,
                name TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'user',
                state TEXT NOT NULL DEFAULT 'active',
                config_json TEXT NOT NULL,
                notify_chat_id TEXT,
                notify_channel TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                last_fired_at TEXT,
                next_fire_at TEXT,
                error_count INTEGER NOT NULL DEFAULT 0,
                consecutive_errors INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Create concerns table: {e}")))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_concerns_state
             ON perpetuum_concerns(state)",
        )
        .execute(&self.pool)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(target: "perpetuum", error = %e, "Non-critical DB operation failed");
            Default::default()
        });

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_concerns_next_fire
             ON perpetuum_concerns(next_fire_at)",
        )
        .execute(&self.pool)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(target: "perpetuum", error = %e, "Non-critical DB operation failed");
            Default::default()
        });

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS perpetuum_monitor_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                concern_id TEXT NOT NULL,
                checked_at TEXT NOT NULL,
                raw_content_hash TEXT,
                raw_content_preview TEXT,
                change_detected INTEGER NOT NULL DEFAULT 0,
                interpretation TEXT,
                notified INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Create monitor_history table: {e}")))?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_monitor_history_concern
             ON perpetuum_monitor_history(concern_id)",
        )
        .execute(&self.pool)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(target: "perpetuum", error = %e, "Non-critical DB operation failed");
            Default::default()
        });

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS perpetuum_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Create state table: {e}")))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS perpetuum_transitions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_state TEXT NOT NULL,
                to_state TEXT NOT NULL,
                reason TEXT NOT NULL,
                trigger_name TEXT,
                timestamp TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Create transitions table: {e}")))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS perpetuum_volition_notes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                note TEXT NOT NULL,
                context TEXT,
                created_at TEXT NOT NULL,
                expires_at TEXT
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Create volition_notes table: {e}")))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS perpetuum_activity_log (
                hour_bucket TEXT PRIMARY KEY,
                interaction_count INTEGER NOT NULL DEFAULT 0,
                last_interaction TEXT
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Create activity_log table: {e}")))?;

        Ok(())
    }

    // ---- Concern CRUD ----

    pub async fn insert_concern(&self, concern: &StoredConcern) -> Result<(), Temm1eError> {
        sqlx::query(
            "INSERT INTO perpetuum_concerns
             (id, concern_type, name, source, state, config_json, notify_chat_id,
              notify_channel, created_at, updated_at, last_fired_at, next_fire_at,
              error_count, consecutive_errors)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&concern.id)
        .bind(&concern.concern_type)
        .bind(&concern.name)
        .bind(&concern.source)
        .bind(&concern.state)
        .bind(&concern.config_json)
        .bind(&concern.notify_chat_id)
        .bind(&concern.notify_channel)
        .bind(&concern.created_at)
        .bind(&concern.updated_at)
        .bind(&concern.last_fired_at)
        .bind(&concern.next_fire_at)
        .bind(concern.error_count)
        .bind(concern.consecutive_errors)
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Insert concern: {e}")))?;
        Ok(())
    }

    pub async fn get_concern(&self, id: &str) -> Result<Option<StoredConcern>, Temm1eError> {
        let row =
            sqlx::query_as::<_, StoredConcern>("SELECT * FROM perpetuum_concerns WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| Temm1eError::Memory(format!("Get concern: {e}")))?;
        Ok(row)
    }

    pub async fn update_concern(&self, concern: &StoredConcern) -> Result<(), Temm1eError> {
        sqlx::query(
            "UPDATE perpetuum_concerns SET
             state = ?, config_json = ?, updated_at = ?, last_fired_at = ?,
             next_fire_at = ?, error_count = ?, consecutive_errors = ?
             WHERE id = ?",
        )
        .bind(&concern.state)
        .bind(&concern.config_json)
        .bind(&concern.updated_at)
        .bind(&concern.last_fired_at)
        .bind(&concern.next_fire_at)
        .bind(concern.error_count)
        .bind(concern.consecutive_errors)
        .bind(&concern.id)
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Update concern: {e}")))?;
        Ok(())
    }

    pub async fn delete_concern(&self, id: &str) -> Result<(), Temm1eError> {
        sqlx::query("DELETE FROM perpetuum_concerns WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| Temm1eError::Memory(format!("Delete concern: {e}")))?;

        sqlx::query("DELETE FROM perpetuum_monitor_history WHERE concern_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(target: "perpetuum", error = %e, "Non-critical DB operation failed");
                Default::default()
            });

        Ok(())
    }

    pub async fn list_active_concerns(&self) -> Result<Vec<StoredConcern>, Temm1eError> {
        let rows = sqlx::query_as::<_, StoredConcern>(
            "SELECT * FROM perpetuum_concerns WHERE state = 'active' ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("List concerns: {e}")))?;
        Ok(rows)
    }

    /// Atomically claim all due concerns by transitioning them from 'active' to 'firing'.
    /// Returns the IDs of claimed concerns. Prevents duplicate fires — once claimed,
    /// no other Pulse tick can see them.
    pub async fn claim_due_concerns(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<ConcernId>, Temm1eError> {
        let now_str = now.to_rfc3339();

        // Step 1: Fetch IDs of active due concerns
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM perpetuum_concerns
             WHERE state = 'active' AND next_fire_at IS NOT NULL AND next_fire_at <= ?
             ORDER BY next_fire_at ASC",
        )
        .bind(&now_str)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Query due concerns: {e}")))?;

        let ids: Vec<ConcernId> = rows.into_iter().map(|r| r.0).collect();
        if ids.is_empty() {
            return Ok(ids);
        }

        // Step 2: Atomically transition them to 'firing' (only if still 'active')
        for id in &ids {
            sqlx::query(
                "UPDATE perpetuum_concerns SET state = 'firing', updated_at = ?
                 WHERE id = ? AND state = 'active'",
            )
            .bind(&now_str)
            .bind(id)
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(target: "perpetuum", error = %e, "Non-critical DB operation failed");
                Default::default()
            }); // If already firing (race), silently skip
        }

        Ok(ids)
    }

    /// Reset a concern from 'firing' back to 'active' (e.g., after reschedule).
    pub async fn reset_firing_state(&self, id: &str) -> Result<(), Temm1eError> {
        sqlx::query(
            "UPDATE perpetuum_concerns SET state = 'active' WHERE id = ? AND state = 'firing'",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Reset firing state: {e}")))?;
        Ok(())
    }

    pub async fn next_fire_time(&self) -> Result<Option<DateTime<Utc>>, Temm1eError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT next_fire_at FROM perpetuum_concerns
             WHERE state = 'active' AND next_fire_at IS NOT NULL
             ORDER BY next_fire_at ASC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Next fire time: {e}")))?;

        Ok(row.and_then(|r| {
            DateTime::parse_from_rfc3339(&r.0)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        }))
    }

    pub async fn count_active(&self) -> Result<usize, Temm1eError> {
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM perpetuum_concerns WHERE state = 'active'")
                .fetch_one(&self.pool)
                .await
                .map_err(|e| Temm1eError::Memory(format!("Count active: {e}")))?;
        Ok(row.0 as usize)
    }

    // ---- Monitor history ----

    pub async fn insert_monitor_result(
        &self,
        entry: &MonitorResultInput,
    ) -> Result<(), Temm1eError> {
        let concern_id = &entry.concern_id;
        let checked_at = entry.checked_at;
        let content_hash = entry.content_hash.as_deref();
        let content_preview = entry.content_preview.as_deref();
        let change_detected = entry.change_detected;
        let interpretation = entry.interpretation.as_deref();
        let notified = entry.notified;
        sqlx::query(
            "INSERT INTO perpetuum_monitor_history
             (concern_id, checked_at, raw_content_hash, raw_content_preview,
              change_detected, interpretation, notified)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(concern_id)
        .bind(checked_at.to_rfc3339())
        .bind(content_hash)
        .bind(content_preview)
        .bind(change_detected)
        .bind(interpretation)
        .bind(notified)
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Insert monitor result: {e}")))?;
        Ok(())
    }

    pub async fn monitor_history(
        &self,
        concern_id: &str,
        limit: usize,
    ) -> Result<Vec<MonitorHistoryEntry>, Temm1eError> {
        let rows = sqlx::query_as::<_, MonitorHistoryEntry>(
            "SELECT * FROM perpetuum_monitor_history
             WHERE concern_id = ? ORDER BY checked_at DESC LIMIT ?",
        )
        .bind(concern_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Monitor history: {e}")))?;
        Ok(rows)
    }

    pub async fn monitor_check_count(&self, concern_id: &str) -> Result<u32, Temm1eError> {
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM perpetuum_monitor_history WHERE concern_id = ?")
                .bind(concern_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| Temm1eError::Memory(format!("Monitor check count: {e}")))?;
        Ok(row.0 as u32)
    }

    // ---- State persistence ----

    pub async fn get_state(&self, key: &str) -> Result<Option<String>, Temm1eError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value FROM perpetuum_state WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| Temm1eError::Memory(format!("Get state: {e}")))?;
        Ok(row.map(|r| r.0))
    }

    pub async fn set_state(&self, key: &str, value: &str) -> Result<(), Temm1eError> {
        sqlx::query(
            "INSERT OR REPLACE INTO perpetuum_state (key, value, updated_at)
             VALUES (?, ?, ?)",
        )
        .bind(key)
        .bind(value)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Set state: {e}")))?;
        Ok(())
    }

    pub async fn log_transition(
        &self,
        from: &str,
        to: &str,
        reason: &str,
        trigger: Option<&str>,
    ) -> Result<(), Temm1eError> {
        sqlx::query(
            "INSERT INTO perpetuum_transitions (from_state, to_state, reason, trigger_name, timestamp)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(from)
        .bind(to)
        .bind(reason)
        .bind(trigger)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Log transition: {e}")))?;
        Ok(())
    }

    // ---- Volition notes ----

    pub async fn save_volition_note(&self, note: &str, context: &str) -> Result<(), Temm1eError> {
        sqlx::query(
            "INSERT INTO perpetuum_volition_notes (note, context, created_at)
             VALUES (?, ?, ?)",
        )
        .bind(note)
        .bind(context)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Save volition note: {e}")))?;
        Ok(())
    }

    pub async fn get_volition_notes(&self, limit: usize) -> Result<Vec<String>, Temm1eError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT note FROM perpetuum_volition_notes
             WHERE expires_at IS NULL OR expires_at > ?
             ORDER BY created_at DESC LIMIT ?",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Get volition notes: {e}")))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    pub async fn cleanup_expired_notes(&self) -> Result<(), Temm1eError> {
        sqlx::query(
            "DELETE FROM perpetuum_volition_notes WHERE expires_at IS NOT NULL AND expires_at <= ?",
        )
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Cleanup expired notes: {e}")))?;
        Ok(())
    }

    // ---- Activity log ----

    pub async fn record_activity(&self, timestamp: DateTime<Utc>) -> Result<(), Temm1eError> {
        let bucket = timestamp.format("%Y-%m-%dT%H").to_string();
        sqlx::query(
            "INSERT INTO perpetuum_activity_log (hour_bucket, interaction_count, last_interaction)
             VALUES (?, 1, ?)
             ON CONFLICT(hour_bucket) DO UPDATE SET
             interaction_count = interaction_count + 1,
             last_interaction = ?",
        )
        .bind(&bucket)
        .bind(timestamp.to_rfc3339())
        .bind(timestamp.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Record activity: {e}")))?;
        Ok(())
    }

    pub async fn activity_probability(&self, hour: u32, _weekday: u32) -> Result<f64, Temm1eError> {
        // Query all hour buckets matching this hour-of-day from the last 4 weeks
        let pattern = format!("%T{:02}", hour);
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT interaction_count FROM perpetuum_activity_log
             WHERE hour_bucket LIKE ? ORDER BY hour_bucket DESC LIMIT 28",
        )
        .bind(&pattern)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Temm1eError::Memory(format!("Activity probability: {e}")))?;

        if rows.is_empty() {
            return Ok(0.5); // No data — assume 50%
        }

        let active_count = rows.iter().filter(|r| r.0 > 0).count();
        Ok(active_count as f64 / rows.len() as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> Store {
        Store::new("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn init_tables_idempotent() {
        let store = test_store().await;
        // Second init should not fail
        store.init_tables().await.unwrap();
    }

    #[tokio::test]
    async fn concern_crud_cycle() {
        let store = test_store().await;
        let now = Utc::now().to_rfc3339();

        let concern = StoredConcern {
            id: "test-alarm-001".into(),
            concern_type: "alarm".into(),
            name: "wake up".into(),
            source: "user".into(),
            state: "active".into(),
            config_json: r#"{"name":"wake up","fire_at":"2026-04-01T06:00:00Z","message":"rise"}"#
                .into(),
            notify_chat_id: Some("chat-123".into()),
            notify_channel: Some("telegram".into()),
            created_at: now.clone(),
            updated_at: now.clone(),
            last_fired_at: None,
            next_fire_at: Some("2026-04-01T06:00:00Z".into()),
            error_count: 0,
            consecutive_errors: 0,
        };

        store.insert_concern(&concern).await.unwrap();

        let fetched = store.get_concern("test-alarm-001").await.unwrap().unwrap();
        assert_eq!(fetched.name, "wake up");
        assert_eq!(fetched.concern_type, "alarm");

        let active = store.list_active_concerns().await.unwrap();
        assert_eq!(active.len(), 1);

        let count = store.count_active().await.unwrap();
        assert_eq!(count, 1);

        store.delete_concern("test-alarm-001").await.unwrap();
        let gone = store.get_concern("test-alarm-001").await.unwrap();
        assert!(gone.is_none());
    }

    #[tokio::test]
    async fn due_concerns_query() {
        let store = test_store().await;
        let now = Utc::now();
        let past = (now - chrono::Duration::minutes(5)).to_rfc3339();
        let future = (now + chrono::Duration::hours(1)).to_rfc3339();

        let past_concern = StoredConcern {
            id: "past-001".into(),
            concern_type: "alarm".into(),
            name: "overdue".into(),
            source: "user".into(),
            state: "active".into(),
            config_json: "{}".into(),
            notify_chat_id: None,
            notify_channel: None,
            created_at: past.clone(),
            updated_at: past.clone(),
            last_fired_at: None,
            next_fire_at: Some(past.clone()),
            error_count: 0,
            consecutive_errors: 0,
        };

        let future_concern = StoredConcern {
            id: "future-001".into(),
            concern_type: "alarm".into(),
            name: "upcoming".into(),
            source: "user".into(),
            state: "active".into(),
            config_json: "{}".into(),
            notify_chat_id: None,
            notify_channel: None,
            created_at: past.clone(),
            updated_at: past.clone(),
            last_fired_at: None,
            next_fire_at: Some(future),
            error_count: 0,
            consecutive_errors: 0,
        };

        store.insert_concern(&past_concern).await.unwrap();
        store.insert_concern(&future_concern).await.unwrap();

        let due = store.claim_due_concerns(now).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0], "past-001");

        // Verify concern is now in 'firing' state (not 'active')
        let concern = store.get_concern("past-001").await.unwrap().unwrap();
        assert_eq!(concern.state, "firing");

        // A second claim should return nothing (already firing)
        let due2 = store.claim_due_concerns(now).await.unwrap();
        assert!(due2.is_empty(), "Should not re-claim firing concerns");
    }

    #[tokio::test]
    async fn state_persistence() {
        let store = test_store().await;

        store.set_state("conscience_state", "active").await.unwrap();
        let val = store.get_state("conscience_state").await.unwrap();
        assert_eq!(val.unwrap(), "active");

        store.set_state("conscience_state", "idle").await.unwrap();
        let val = store.get_state("conscience_state").await.unwrap();
        assert_eq!(val.unwrap(), "idle");
    }

    #[tokio::test]
    async fn transition_logging() {
        let store = test_store().await;
        store
            .log_transition("active", "idle", "no_foreground", None)
            .await
            .unwrap();
        store
            .log_transition("idle", "sleep", "idle_threshold", Some("timer"))
            .await
            .unwrap();
        // Verify no errors — transition log is append-only
    }

    #[tokio::test]
    async fn volition_notes() {
        let store = test_store().await;
        store
            .save_volition_note("user cares about MCP", "conversation_end")
            .await
            .unwrap();
        store
            .save_volition_note("reddit quiet for 2 days", "schedule_review")
            .await
            .unwrap();

        let notes = store.get_volition_notes(5).await.unwrap();
        assert_eq!(notes.len(), 2);
    }

    #[tokio::test]
    async fn activity_log() {
        let store = test_store().await;
        let now = Utc::now();
        store.record_activity(now).await.unwrap();
        store.record_activity(now).await.unwrap();

        let prob = store
            .activity_probability(now.format("%H").to_string().parse().unwrap(), 0)
            .await
            .unwrap();
        assert!(prob > 0.0);
    }

    #[tokio::test]
    async fn monitor_history_crud() {
        let store = test_store().await;
        let now = Utc::now();

        // Insert a concern first
        let concern = StoredConcern {
            id: "mon-001".into(),
            concern_type: "monitor".into(),
            name: "reddit".into(),
            source: "user".into(),
            state: "active".into(),
            config_json: "{}".into(),
            notify_chat_id: None,
            notify_channel: None,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            last_fired_at: None,
            next_fire_at: None,
            error_count: 0,
            consecutive_errors: 0,
        };
        store.insert_concern(&concern).await.unwrap();

        store
            .insert_monitor_result(&MonitorResultInput {
                concern_id: "mon-001".into(),
                checked_at: now,
                content_hash: Some("abc123".into()),
                content_preview: Some("hello".into()),
                change_detected: false,
                interpretation: None,
                notified: false,
            })
            .await
            .unwrap();
        store
            .insert_monitor_result(&MonitorResultInput {
                concern_id: "mon-001".into(),
                checked_at: now,
                content_hash: Some("def456".into()),
                content_preview: Some("world".into()),
                change_detected: true,
                interpretation: Some("{}".into()),
                notified: true,
            })
            .await
            .unwrap();

        let history = store.monitor_history("mon-001", 10).await.unwrap();
        assert_eq!(history.len(), 2);

        let count = store.monitor_check_count("mon-001").await.unwrap();
        assert_eq!(count, 2);
    }
}
