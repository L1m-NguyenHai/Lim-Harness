use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};
use uuid::Uuid;

pub fn now() -> String {
    Utc::now().to_rfc3339()
}
fn preview_source() -> String {
    "ui_preview".into()
}
fn check_window(context_size: usize, settings: &InferenceSettings) -> Result<(), String> {
    if ![4096, 8192, 16384].contains(&context_size) {
        return Err("Context phải là 4096, 8192 hoặc 16384.".into());
    }
    if context_size.saturating_sub(settings.output_reserve() + settings.system_prompt.len() + 256) < 64 {
        return Err("Cấu hình phải còn ít nhất 64 byte UTF-8 cho context đầu vào sau khi dành suy nghĩ, trả lời, system prompt và framing. Giảm giới hạn/system prompt hoặc tăng context.".into());
    }
    Ok(())
}
// Kept short: rules for Obsidian notes travel with the notes (vault::memory_context).
pub const DEFAULT_SYSTEM_PROMPT:&str="Bạn là Lim, trợ lý AI chạy trên máy của người dùng. Mặc định trả lời bằng tiếng Việt, tự nhiên và ngắn gọn. Bạn chỉ trò chuyện bằng văn bản, không chạy lệnh và không đọc hay sửa tệp.";
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InferenceSettings {
    pub system_prompt: String,
    pub temperature: f64,
    pub top_p: f64,
    pub top_k: usize,
    pub repeat_penalty: f64,
    /// Output tokens per turn, shared by thinking and answer (Ollama's num_predict).
    pub num_predict: usize,
    /// Older builds stored a separate thinking budget; `normalize` folds it into num_predict.
    #[serde(default, rename = "thinkPredict", skip_serializing)]
    pub(crate) legacy_think_predict: usize,
    pub seed: Option<i64>,
}
impl Default for InferenceSettings {
    fn default() -> Self {
        Self {
            system_prompt: DEFAULT_SYSTEM_PROMPT.into(),
            temperature: 0.7,
            top_p: 0.9,
            top_k: 40,
            repeat_penalty: 1.1,
            num_predict: 2048,
            legacy_think_predict: 0,
            seed: None,
        }
    }
}
impl InferenceSettings {
    /// Tokens the window must keep free for generation (thinking and answer share it).
    pub fn output_reserve(&self) -> usize {
        self.num_predict
    }
    fn normalize(&mut self) {
        if self.legacy_think_predict > 0 {
            self.num_predict = (self.num_predict + self.legacy_think_predict).min(8192);
            self.legacy_think_predict = 0;
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.system_prompt.len() > 12_000 {
            return Err("System prompt không được vượt quá 12.000 byte UTF-8.".into());
        }
        for (name, value, maximum) in [
            ("temperature", self.temperature, 2.0),
            ("topP", self.top_p, 1.0),
            ("repeatPenalty", self.repeat_penalty, 2.0),
        ] {
            if !value.is_finite() || !(0.0..=maximum).contains(&value) {
                return Err(format!("{name} phải là số hữu hạn từ 0 đến {maximum}."));
            }
        }
        if self.top_k > 100 {
            return Err("topK phải là số nguyên từ 0 đến 100.".into());
        }
        if !(128..=8192).contains(&self.num_predict) {
            return Err("numPredict phải là số nguyên từ 128 đến 8192.".into());
        }
        if self
            .seed
            .is_some_and(|seed| !(0..=2_147_483_647).contains(&seed))
        {
            return Err("Seed phải là số nguyên từ 0 đến 2147483647 hoặc để trống.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMetrics {
    pub output_tokens: Option<u64>,
    pub prompt_tokens: Option<u64>,
    pub eval_duration_ms: Option<u64>,
    pub total_duration_ms: Option<u64>,
    pub tokens_per_second: Option<f64>,
    /// Streamed-chunk counts (≈ tokens) per phase; estimates, not Ollama counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_tokens: Option<u64>,
    /// Output limit granted to this answer so far; each "continue" adds that run's limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_limit: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub role: String,
    pub text: String,
    #[serde(default = "preview_source")]
    pub source: String,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<crate::vault::VaultHit>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<ChatMetrics>,
    /// Failure reason of the run that produced a failed assistant message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
/// Every run setting of one session: its context window plus all inference parameters.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    pub context_size: usize,
    #[serde(flatten)]
    pub settings: InferenceSettings,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created: String,
    pub messages: Vec<Message>,
    /// None: the session follows the global defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<SessionConfig>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub sessions: Vec<Session>,
    pub model: String,
    pub vault_path: String,
    pub context_size: usize,
    pub inference_settings: InferenceSettings,
}

pub struct Store {
    connection: Mutex<Connection>,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        let connection = Connection::open(path).map_err(|e| e.to_string())?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
          CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY,title TEXT NOT NULL,created TEXT NOT NULL,updated TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS runs(id TEXT PRIMARY KEY,session_id TEXT NOT NULL REFERENCES sessions(id),model TEXT NOT NULL,status TEXT NOT NULL,started TEXT NOT NULL,ended TEXT,error TEXT);
          CREATE UNIQUE INDEX IF NOT EXISTS one_running_session ON runs(session_id) WHERE status='running';
          CREATE TABLE IF NOT EXISTS messages(id TEXT PRIMARY KEY,session_id TEXT NOT NULL REFERENCES sessions(id),run_id TEXT REFERENCES runs(id),role TEXT NOT NULL,text TEXT NOT NULL,source TEXT NOT NULL,status TEXT,created TEXT NOT NULL,position INTEGER NOT NULL,sources TEXT,model TEXT,thinking TEXT NOT NULL DEFAULT '',metrics TEXT);
          CREATE INDEX IF NOT EXISTS ordered_messages ON messages(session_id,position);").map_err(|e| e.to_string())?;
        for migration in [
            "ALTER TABLE messages ADD COLUMN model TEXT",
            "ALTER TABLE messages ADD COLUMN thinking TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE messages ADD COLUMN metrics TEXT",
            "ALTER TABLE sessions ADD COLUMN config TEXT",
        ] {
            let _ = connection.execute(migration, []);
        }
        connection
            .execute(
                "UPDATE messages SET status='interrupted' WHERE status='running'",
                [],
            )
            .map_err(|e| e.to_string())?;
        connection.execute("UPDATE runs SET status='interrupted',ended=?1,error='Ứng dụng đóng trước khi lượt hoàn tất' WHERE status='running'", [now()]).map_err(|e|e.to_string())?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
    fn db(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.connection
            .lock()
            .map_err(|_| "Kho hội thoại không khả dụng".into())
    }
    pub fn setting(&self, key: &str) -> Result<String, String> {
        self.db()?
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .map(|v| v.unwrap_or_default())
            .map_err(|e| e.to_string())
    }
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.db()?.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value]).map(|_|()).map_err(|e|e.to_string())
    }
    pub fn set_model_config(&self, model: &str, context_size: usize) -> Result<(), String> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        for (key, value) in [
            ("model", model.to_string()),
            ("context_size", context_size.to_string()),
        ] {
            tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn snapshot(&self) -> Result<Snapshot, String> {
        let sessions = {
            let db = self.db()?;
            let mut stmt = db
                .prepare("SELECT id,title,created,config FROM sessions ORDER BY updated DESC,rowid DESC")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| {
                    let config: Option<String> = r.get(3).unwrap_or(None);
                    Ok(Session {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        created: r.get(2)?,
                        messages: vec![],
                        config: config.and_then(|raw| serde_json::from_str(&raw).ok()),
                    })
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        let mut populated = Vec::new();
        for mut session in sessions {
            session.messages = self.messages(&session.id)?;
            // A damaged session config must not block loading the app; show global values instead.
            let (context_size, settings) = self.session_config(&session.id).or_else(|_| self.inference_config())?;
            session.config = Some(SessionConfig { context_size, settings });
            populated.push(session);
        }
        let (context_size, inference_settings) = self.inference_config()?;
        Ok(Snapshot {
            sessions: populated,
            model: self.setting("model")?,
            vault_path: self.setting("vault")?,
            context_size,
            inference_settings,
        })
    }
    pub fn inference_config(&self) -> Result<(usize, InferenceSettings), String> {
        let db = self.db()?;
        let mut statement = db
            .prepare(
                "SELECT key,value FROM settings WHERE key IN ('context_size','inference_settings')",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut context_size = 4096;
        let mut settings = InferenceSettings::default();
        for row in rows {
            let (key, value) = row.map_err(|e| e.to_string())?;
            match key.as_str() {
                "context_size" => {
                    let value = value.parse().unwrap_or(4096);
                    context_size = if [4096, 8192, 16384].contains(&value) {
                        value
                    } else {
                        4096
                    };
                }
                "inference_settings" if !value.is_empty() => {
                    settings = serde_json::from_str(&value).map_err(|_| {
                        "Cấu hình inference đã lưu không hợp lệ; cần lưu lại cấu hình."
                    })?;
                    settings.normalize();
                }
                _ => {}
            }
        }
        settings.validate()?;
        Ok((context_size, settings))
    }
    pub fn set_inference_config(
        &self,
        settings: &InferenceSettings,
        context_size: usize,
    ) -> Result<(), String> {
        settings.validate()?;
        check_window(context_size, settings)?;
        let encoded = serde_json::to_string(settings).map_err(|e| e.to_string())?;
        let mut db = self.db()?;
        let transaction = db.transaction().map_err(|e| e.to_string())?;
        for (key, value) in [
            ("inference_settings", encoded),
            ("context_size", context_size.to_string()),
        ] {
            transaction.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value]).map_err(|e|e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())
    }
    pub fn messages(&self, session: &str) -> Result<Vec<Message>, String> {
        let db = self.db()?;
        let mut stmt=db.prepare("SELECT m.id,m.role,m.text,m.source,m.run_id,m.status,m.sources,m.model,m.thinking,m.metrics,CASE WHEN m.role='assistant' AND m.status='failed' THEN r.error END FROM messages m LEFT JOIN runs r ON r.id=m.run_id WHERE m.session_id=?1 ORDER BY m.position,m.rowid").map_err(|e|e.to_string())?;
        let rows = stmt
            .query_map([session], |r| {
                let raw: Option<String> = r.get(6)?;
                let model: Option<String> = r.get(7).unwrap_or(None);
                let thinking: Option<String> = r.get(8).unwrap_or_default();
                let metrics_raw: Option<String> = r.get(9).unwrap_or(None);
                Ok(Message {
                    id: r.get(0)?,
                    role: r.get(1)?,
                    text: r.get(2)?,
                    source: r.get(3)?,
                    run_id: r.get(4)?,
                    status: r.get(5)?,
                    sources: raw.and_then(|s| serde_json::from_str(&s).ok()),
                    model,
                    thinking: thinking.filter(|t| !t.is_empty()),
                    metrics: metrics_raw.and_then(|s| serde_json::from_str(&s).ok()),
                    error: r.get(10).unwrap_or(None),
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    pub fn create_session(&self) -> Result<Session, String> {
        // Each new session starts from the built-in defaults, independent of other sessions.
        let config = SessionConfig { context_size: 4096, settings: InferenceSettings::default() };
        let session = Session {
            id: Uuid::new_v4().to_string(),
            title: "Phiên mới".into(),
            created: now(),
            messages: vec![],
            config: Some(config.clone()),
        };
        let encoded = serde_json::to_string(&config).map_err(|e| e.to_string())?;
        self.db()?
            .execute(
                "INSERT INTO sessions(id,title,created,updated,config) VALUES(?1,?2,?3,?3,?4)",
                params![session.id, session.title, session.created, encoded],
            )
            .map_err(|e| e.to_string())?;
        Ok(session)
    }
    /// Run settings of one session. Sessions saved before per-session settings existed (no or
    /// partial config) keep the previously global values for any field they do not store.
    pub fn session_config(&self, session: &str) -> Result<(usize, InferenceSettings), String> {
        let (context_size, settings) = self.inference_config()?;
        let raw: Option<String> = self
            .db()?
            .query_row("SELECT config FROM sessions WHERE id=?1", [session], |r| r.get(0))
            .map_err(|_| "Không tìm thấy phiên".to_string())?;
        let Some(stored) = raw.and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok()) else {
            return Ok((context_size, settings));
        };
        let mut merged = serde_json::to_value(SessionConfig { context_size, settings }).map_err(|e| e.to_string())?;
        if let (Some(target), Some(fields)) = (merged.as_object_mut(), stored.as_object()) {
            for (key, value) in fields {
                target.insert(key.clone(), value.clone());
            }
        }
        let mut config: SessionConfig = serde_json::from_value(merged).map_err(|_| "Cấu hình của phiên không hợp lệ; cần lưu lại.")?;
        config.settings.normalize();
        config.settings.validate()?;
        Ok((config.context_size, config.settings))
    }
    pub fn set_session_config(&self, session: &str, config: &SessionConfig) -> Result<(), String> {
        let mut config = config.clone();
        config.settings.normalize();
        let config = &config;
        config.settings.validate()?;
        check_window(config.context_size, &config.settings)?;
        let encoded = serde_json::to_string(config).map_err(|e| e.to_string())?;
        let changed = self
            .db()?
            .execute("UPDATE sessions SET config=?2 WHERE id=?1", params![session, encoded])
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("Không tìm thấy phiên".into());
        }
        Ok(())
    }
    /// Deletes a session with its messages and runs. Refuses while a run is still active.
    pub fn delete_session(&self, session: &str) -> Result<(), String> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let running: i64 = tx
            .query_row("SELECT COUNT(*) FROM runs WHERE session_id=?1 AND status='running'", [session], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if running > 0 {
            return Err("Phiên đang có lượt chạy; dừng trả lời trước khi xoá.".into());
        }
        for sql in [
            "DELETE FROM messages WHERE session_id=?1",
            "DELETE FROM runs WHERE session_id=?1",
            "DELETE FROM sessions WHERE id=?1",
        ] {
            tx.execute(sql, [session]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn import_sessions(&self, sessions: Vec<Session>) -> Result<(), String> {
        if sessions.len() > 1000 {
            return Err("Quá nhiều phiên để chuyển một lần".into());
        }
        let mut db = self.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        for session in sessions {
            if session.id.len() > 128
                || session.id.is_empty()
                || session.title.len() > 1000
                || chrono::DateTime::parse_from_rfc3339(&session.created).is_err()
                || session.messages.len() > 10000
            {
                return Err("Dữ liệu phiên cũ không hợp lệ".into());
            }
            let inserted = tx
                .execute(
                    "INSERT OR IGNORE INTO sessions(id,title,created,updated) VALUES(?1,?2,?3,?3)",
                    params![session.id, session.title, session.created],
                )
                .map_err(|e| e.to_string())?;
            if inserted == 0 {
                continue;
            }
            for (position, message) in session.messages.iter().enumerate() {
                if !["user", "assistant"].contains(&message.role.as_str())
                    || message.text.len() > 2_000_000
                    || message.id.len() > 128
                    || message.id.is_empty()
                {
                    return Err("Tin nhắn cũ không hợp lệ".into());
                }
                let source = if message.role == "assistant" {
                    "ui_preview"
                } else {
                    "user"
                };
                tx.execute("INSERT INTO messages(id,session_id,role,text,source,status,created,position) VALUES(?1,?2,?3,?4,?5,'completed',?6,?7)",params![message.id,session.id,message.role,message.text,source,session.created,position as i64]).map_err(|e|e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn begin(
        &self,
        session: &str,
        run: &str,
        model: &str,
        text: &str,
    ) -> Result<(Message, Message), String> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
                [session],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exists {
            return Err("Không tìm thấy phiên hội thoại".into());
        }
        let position: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(position)+1,0) FROM messages WHERE session_id=?1",
                [session],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let timestamp = now();
        tx.execute(
            "INSERT INTO runs(id,session_id,model,status,started) VALUES(?1,?2,?3,'running',?4)",
            params![run, session, model, timestamp],
        )
        .map_err(|e| e.to_string())?;
        let user = Message {
            id: Uuid::new_v4().to_string(),
            role: "user".into(),
            text: text.into(),
            source: "user".into(),
            run_id: Some(run.into()),
            status: Some("completed".into()),
            sources: None,
            model: None,
            thinking: None,
            metrics: None,
            error: None,
        };
        let assistant = Message {
            id: Uuid::new_v4().to_string(),
            role: "assistant".into(),
            text: String::new(),
            source: "model".into(),
            run_id: Some(run.into()),
            status: Some("running".into()),
            sources: None,
            model: Some(model.into()),
            thinking: None,
            metrics: None,
            error: None,
        };
        for (offset, message) in [&user, &assistant].iter().enumerate() {
            tx.execute("INSERT INTO messages(id,session_id,run_id,role,text,source,status,created,position,model,thinking) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'')",params![message.id,session,run,message.role,message.text,message.source,message.status,timestamp,position+offset as i64,message.model]).map_err(|e|e.to_string())?;
        }
        tx.execute("UPDATE sessions SET title=CASE WHEN ?2=0 THEN ?3 ELSE title END,updated=?4 WHERE id=?1",params![session,position,text.chars().take(48).collect::<String>(),timestamp]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok((user, assistant))
    }
    pub fn checkpoint(&self, id: &str, text: &str, thinking: &str) -> Result<(), String> {
        self.db()?
            .execute(
                "UPDATE messages SET text=?2,thinking=?3 WHERE id=?1 AND status='running'",
                params![id, text, thinking],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    pub fn save_sources(&self, id: &str, sources: &[crate::vault::VaultHit]) -> Result<(), String> {
        let raw = serde_json::to_string(sources).map_err(|e| e.to_string())?;
        self.db()?
            .execute(
                "UPDATE messages SET sources=?2 WHERE id=?1",
                params![id, raw],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    /// Reopens an answer that stopped at the output limit so a new run can continue it.
    pub fn begin_continue(&self, session: &str, run: &str, message_id: &str, model: &str) -> Result<Message, String> {
        {
            let mut db = self.db()?;
            let tx = db.transaction().map_err(|e| e.to_string())?;
            let status: Option<String> = tx
                .query_row(
                    "SELECT status FROM messages WHERE id=?1 AND session_id=?2 AND role='assistant'",
                    params![message_id, session],
                    |r| r.get(0),
                )
                .map_err(|_| "Không tìm thấy câu trả lời cần tiếp tục".to_string())?;
            if status.as_deref() != Some("truncated") {
                return Err("Chỉ tiếp tục được câu trả lời đã dừng vì hết token.".into());
            }
            let newer: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE session_id=?1 AND position>(SELECT position FROM messages WHERE id=?2))",
                    params![session, message_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if newer {
                return Err("Đã có tin nhắn mới hơn; chỉ tiếp tục được câu trả lời mới nhất.".into());
            }
            tx.execute(
                "INSERT INTO runs(id,session_id,model,status,started) VALUES(?1,?2,?3,'running',?4)",
                params![run, session, model, now()],
            )
            .map_err(|_| "Phiên này đang có một lượt chạy".to_string())?;
            tx.execute("UPDATE messages SET status='running',run_id=?2 WHERE id=?1", params![message_id, run])
                .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        self.messages(session)?
            .into_iter()
            .find(|m| m.id == message_id)
            .ok_or_else(|| "Không tìm thấy câu trả lời cần tiếp tục".into())
    }
    pub fn finish(
        &self,
        session: &str,
        message: &Message,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), String> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        let metrics_raw = message
            .metrics
            .as_ref()
            .and_then(|m| serde_json::to_string(m).ok());
        let thinking_raw = message.thinking.clone().unwrap_or_default();
        tx.execute(
            "UPDATE messages SET text=?2,status=?3,thinking=?4,metrics=?5,model=COALESCE(model,?6) WHERE id=?1 AND status='running'",
            params![message.id, message.text, status, thinking_raw, metrics_raw, message.model],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE runs SET status=?2,ended=?3,error=?4 WHERE id=?1 AND status='running'",
            params![message.run_id, status, now(), error],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE sessions SET updated=?2 WHERE id=?1",
            params![session, now()],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod delete_tests {
    use super::*;
    #[test]
    fn deletes_idle_session_and_refuses_running_one() {
        let dir = std::env::temp_dir().join(format!("lim-del-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("t.sqlite3")).unwrap();
        let busy = store.create_session().unwrap().id;
        store.begin(&busy, &Uuid::new_v4().to_string(), "m", "hi").unwrap();
        assert!(store.delete_session(&busy).is_err(), "running session must not be deleted");
        assert_eq!(store.messages(&busy).unwrap().len(), 2);
        let idle = store.create_session().unwrap().id;
        store.delete_session(&idle).unwrap();
        assert!(store.snapshot().unwrap().sessions.iter().all(|s| s.id != idle));
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod session_config_tests {
    use super::*;
    #[test]
    fn sessions_keep_independent_run_settings() {
        let dir = std::env::temp_dir().join(format!("lim-cfg-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("t.sqlite3")).unwrap();
        let a = store.create_session().unwrap().id;
        let mut custom = InferenceSettings { temperature: 1.3, num_predict: 6000, system_prompt: "Bạn là trợ lý viết văn.".into(), ..Default::default() };
        store.set_session_config(&a, &SessionConfig { context_size: 16384, settings: custom.clone() }).unwrap();
        // A session created afterwards starts from defaults, not from the last applied session.
        let b = store.create_session().unwrap().id;
        let (ctx_a, set_a) = store.session_config(&a).unwrap();
        let (ctx_b, set_b) = store.session_config(&b).unwrap();
        assert_eq!((ctx_a, set_a.temperature, set_a.num_predict, set_a.system_prompt.as_str()), (16384, 1.3, 6000, "Bạn là trợ lý viết văn."));
        assert_eq!((ctx_b, set_b.temperature, set_b.num_predict), (4096, 0.7, 2048));
        assert_eq!(set_b.system_prompt, DEFAULT_SYSTEM_PROMPT);
        // An invalid window is rejected and the previous value stays.
        custom.num_predict = 8000;
        assert!(store.set_session_config(&b, &SessionConfig { context_size: 4096, settings: custom }).is_err());
        assert_eq!(store.session_config(&b).unwrap().0, 4096);
        // Older partial configs fill missing fields from the global settings.
        store.db().unwrap().execute("UPDATE sessions SET config='{\"contextSize\":8192}' WHERE id=?1", [&b]).unwrap();
        let (ctx, settings) = store.session_config(&b).unwrap();
        assert_eq!((ctx, settings.temperature), (8192, 0.7));
        // A config saved with separate thinking/answer budgets is folded into one output limit.
        store.db().unwrap().execute("UPDATE sessions SET config='{\"contextSize\":8192,\"numPredict\":1024,\"thinkPredict\":1024}' WHERE id=?1", [&b]).unwrap();
        assert_eq!(store.session_config(&b).unwrap().1.num_predict, 2048);
        assert!(store.snapshot().unwrap().sessions.iter().all(|s| s.config.is_some()));
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
