use crate::{
    storage::{estimate_tokens, InferenceSettings, Message, Session, Snapshot, Store},
    vault,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{ipc::Channel, State};
use tokio::sync::{watch, OwnedRwLockReadGuard, RwLock};
use uuid::Uuid;

const OLLAMA: &str = "http://127.0.0.1:11434";
/// Tokens left for summary, notes and conversation after the system prompt, the output
/// reservation and a framing margin.
fn input_budget(context_size: usize, settings: &InferenceSettings) -> usize {
    context_size.saturating_sub(settings.output_reserve() + estimate_tokens(&settings.system_prompt) + 256)
}
/// Auto-compaction starts when summary + conversation pass this share of the input budget.
const COMPACT_AT_PERCENT: usize = 75;
/// After compaction the newest turns kept verbatim take about this share.
const KEEP_RECENT_PERCENT: usize = 35;
fn summary_block(summary: &str) -> String {
    format!("Tóm tắt phần hội thoại trước (đã thu gọn để tiết kiệm context):\n{summary}")
}
/// Messages after the last one covered by the session summary.
fn after_summary(messages: Vec<Message>, until: Option<&str>) -> Vec<Message> {
    match until.and_then(|id| messages.iter().position(|m| m.id == id)) {
        Some(index) => messages.into_iter().skip(index + 1).collect(),
        None => messages,
    }
}
/// System prompt with the session summary appended.
fn system_with_summary(system: &str, summary: Option<&str>) -> String {
    match summary.filter(|s| !s.trim().is_empty()) {
        Some(summary) if system.is_empty() => summary_block(summary),
        Some(summary) => format!("{system}\n\n{}", summary_block(summary)),
        None => system.to_string(),
    }
}
/// `/api/show` for a model, or None when Ollama does not answer.
async fn model_show(runtime: &Runtime, model: &str) -> Option<Value> {
    let response = runtime.client.post(format!("{OLLAMA}/api/show")).json(&json!({"model":model})).send().await.ok()?;
    response.json::<Value>().await.ok()
}
/// The model's trained context length from GGUF metadata (e.g. qwen35.context_length).
fn context_limit(info: &Value) -> Option<usize> {
    info["model_info"].as_object()?.iter().find(|(key, _)| key.ends_with(".context_length")).and_then(|(_, v)| v.as_u64()).map(|v| v as usize)
}
#[tauri::command]
pub async fn model_context(runtime: State<'_, Arc<Runtime>>, model: String) -> Result<Option<usize>, String> {
    Ok(model_show(&runtime, model.trim()).await.as_ref().and_then(context_limit))
}
/// Streamed chunk counts. Ollama streams about one token per chunk, so these are live
/// estimates; the final Ollama eval_count stays the authoritative total.
pub(crate) struct Counts {
    thinking: u64,
    answer: u64,
    budget: u64,
    /// Agent mode collects tool calls instead of rejecting them.
    agent: bool,
    tool_calls: Vec<Value>,
}
impl Counts {
    fn usage(&self) -> Value {
        json!({"type":"usage","usage":{"thinkingTokens":self.thinking,"answerTokens":self.answer,"budget":self.budget}})
    }
}
enum Step {
    Continue,
    Done,
    /// Ollama stopped at num_predict (done_reason "length").
    Length,
}
struct ActiveRun {
    id: String,
    cancel: watch::Sender<bool>,
}
pub struct Runtime {
    pub store: Store,
    active: Mutex<HashMap<String, ActiveRun>>,
    pub(crate) client: reqwest::Client,
    pub(crate) model_gate: Arc<RwLock<()>>,
    /// Agent steps waiting for the user's Allow/Deny, keyed by approval id.
    approvals: Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
    pub(crate) memory_control: Mutex<()>,
    pub(crate) memory_cancel: watch::Sender<u64>,
    pub(crate) memory_running: AtomicBool,
    pub(crate) foreground_waiting: AtomicUsize,
}
impl Runtime {
    pub fn new(store: Store) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            store,
            active: Mutex::new(HashMap::new()),
            client,
            model_gate: Arc::new(RwLock::new(())),
            approvals: Mutex::new(HashMap::new()),
            memory_control: Mutex::new(()),
            memory_cancel: watch::channel(0).0,
            memory_running: AtomicBool::new(false),
            foreground_waiting: AtomicUsize::new(0),
        })
    }
}
fn emit(
    channel: &Channel<Value>,
    run: &str,
    session: &str,
    sequence: &mut u64,
    mut event: Value,
) -> Result<(), String> {
    event["runId"] = json!(run);
    event["sessionId"] = json!(session);
    event["sequence"] = json!(*sequence);
    *sequence += 1;
    channel
        .send(event)
        .map_err(|_| "Kênh giao diện đã đóng".into())
}

#[tauri::command]
pub fn get_snapshot(state: State<'_, Arc<Runtime>>) -> Result<Snapshot, String> {
    state.store.snapshot()
}
#[tauri::command]
pub fn create_session(state: State<'_, Arc<Runtime>>) -> Result<Session, String> {
    state.store.create_session()
}
#[tauri::command]
pub fn set_session_config(
    state: State<'_, Arc<Runtime>>,
    session_id: String,
    config: crate::storage::SessionConfig,
) -> Result<(), String> {
    state.store.set_session_config(&session_id, &config)
}
#[tauri::command]
pub fn delete_session(state: State<'_, Arc<Runtime>>, session_id: String) -> Result<(), String> {
    // Holding the active-run lock keeps a new turn from starting while the rows are removed.
    let active = state.active.lock().map_err(|_| "Runtime không khả dụng")?;
    if active.contains_key(&session_id) {
        return Err("Phiên đang có lượt chạy; dừng trả lời trước khi xoá.".into());
    }
    let _control = state
        .memory_control
        .lock()
        .map_err(|_| "Memory không khả dụng")?;
    state
        .memory_cancel
        .send_modify(|value| *value = value.wrapping_add(1));
    state.store.delete_session(&session_id)
}
#[tauri::command]
pub fn import_sessions(
    state: State<'_, Arc<Runtime>>,
    sessions: Vec<Session>,
) -> Result<(), String> {
    state.store.import_sessions(sessions)
}
#[tauri::command]
pub fn set_model(state: State<'_, Arc<Runtime>>, model: String) -> Result<(), String> {
    if model.len() > 200 || model.contains(['\r', '\n']) {
        return Err("Tên model không hợp lệ".into());
    }
    state.store.set_setting("model", model.trim())
}
#[tauri::command]
pub fn set_inference_settings(
    state: State<'_, Arc<Runtime>>,
    settings: InferenceSettings,
    context_size: usize,
) -> Result<(), String> {
    state.store.set_inference_config(&settings, context_size)
}
#[tauri::command]
pub fn set_context_size(state: State<'_, Arc<Runtime>>, context_size: usize) -> Result<(), String> {
    if !crate::storage::CONTEXT_SIZES.contains(&context_size) {
        return Err("Context phải là 4K, 8K, 16K, 32K, 64K hoặc 128K token".into());
    }
    state
        .store
        .set_setting("context_size", &context_size.to_string())
}
#[tauri::command]
pub fn set_vault(state: State<'_, Arc<Runtime>>, path: String) -> Result<String, String> {
    let path = if path.trim().is_empty() {
        String::new()
    } else {
        vault::validate_vault_path(&path)?
    };
    let _control = state
        .memory_control
        .lock()
        .map_err(|_| "Memory không khả dụng")?;
    crate::memory_runtime::invalidate(&state)?;
    state.store.set_setting("vault", &path)?;
    Ok(path)
}
#[tauri::command]
pub async fn search_memory(
    state: State<'_, Arc<Runtime>>,
    query: String,
) -> Result<vault::VaultSearchResult, String> {
    let path = state.store.setting("vault")?;
    if path.is_empty() {
        return Err("Chưa chọn vault Obsidian".into());
    }
    tauri::async_runtime::spawn_blocking(move || vault::search_vault(&path, &query, 10))
        .await
        .map_err(|e| e.to_string())?
}
/// What Agent mode needs besides the chat model, for the setup buttons in Settings: the
/// decision model (installed in Ollama or not) and the local SearXNG.
#[tauri::command]
pub async fn agent_setup_status(state: State<'_, Arc<Runtime>>) -> Result<Value, String> {
    let decision = state.store.setting("decision_model").ok().filter(|m| !m.trim().is_empty()).unwrap_or_else(|| crate::agent::DECISION_MODEL.to_string());
    let models = list_models(state).await;
    let installed = models.as_ref().ok().map(|names| names.iter().any(|n| n == &decision || n.strip_suffix(":latest") == Some(decision.as_str())));
    Ok(json!({
        "decisionModel": decision,
        "decisionInstalled": installed,
        "ollama": models.is_ok(),
        "searxng": crate::searxng::status(),
    }))
}
#[tauri::command]
pub async fn list_models(state: State<'_, Arc<Runtime>>) -> Result<Vec<String>, String> {
    let response = state
        .client
        .get(format!("{OLLAMA}/api/tags"))
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .map_err(|_| "Không kết nối được Ollama tại 127.0.0.1:11434. Hãy mở Ollama.".to_string())?;
    if !response.status().is_success() {
        return Err(format!("Ollama trả HTTP {}", response.status()));
    }
    let data: Value = response
        .json()
        .await
        .map_err(|_| "Danh sách model từ Ollama không hợp lệ".to_string())?;
    let models = data
        .get("models")
        .and_then(Value::as_array)
        .ok_or("Ollama không trả danh sách models")?;
    Ok(models
        .iter()
        .filter_map(|m| m.get("name").and_then(Value::as_str).map(str::to_string))
        .collect())
}
#[tauri::command]
pub async fn start_turn(
    state: State<'_, Arc<Runtime>>,
    session_id: String,
    text: String,
    model: String,
    on_event: Channel<Value>,
) -> Result<String, String> {
    let model_guard = state.foreground_read().await?;
    let text = text.trim().to_string();
    let model = model.trim().to_string();
    let (context_size, settings) = state.store.session_config(&session_id)?;
    let memory_target = crate::memory_runtime::capture_target(&state)?;
    // Message size in bytes: what the window can hold (~3 bytes/token), capped at 8.000.

    let maximum = input_budget(context_size, &settings)
        .saturating_sub(16)
        .saturating_mul(3)
        .min(8_000);
    if maximum == 0 {
        return Err(format!("Context {context_size} không còn chỗ cho tin nhắn sau khi dành {} token output và system prompt. Giảm output/system prompt hoặc tăng context.",settings.output_reserve()));
    }
    if text.is_empty() || text.len() > maximum {
        return Err(format!("Tin nhắn cần từ 1 đến {maximum} byte UTF-8 với context {context_size}. Rút gọn nội dung, giảm system prompt/output hoặc tăng context trong cài đặt."));
    }
    if model.is_empty() || model.len() > 200 || model.contains(['\r', '\n']) {
        return Err("Chọn model Ollama hợp lệ trước khi gửi".into());
    }
    let run_id = Uuid::new_v4().to_string();
    let (cancel, receiver) = watch::channel(false);
    let (user, assistant) = {
        let mut active = state.active.lock().map_err(|_| "Runtime không khả dụng")?;
        if active.contains_key(&session_id) {
            return Err("Phiên này đang có một lượt chạy".into());
        }
        let messages = state.store.begin_with_memory_target(
            &session_id,
            &run_id,
            &model,
            &text,
            memory_target.as_ref(),
        )?;
        active.insert(
            session_id.clone(),
            ActiveRun {
                id: run_id.clone(),
                cancel,
            },
        );
        messages
    };
    let mut sequence = 0;
    if let Err(error) = emit(
        &on_event,
        &run_id,
        &session_id,
        &mut sequence,
        json!({"type":"started","message":assistant,"userMessage":user}),
    ) {
        let persisted = state
            .store
            .finish(&session_id, &assistant, "failed", Some(&error));
        state
            .active
            .lock()
            .map_err(|_| "Runtime không khả dụng")?
            .remove(&session_id);
        persisted?;
        return Err(error);
    }
    let runtime = Arc::clone(state.inner());
    let returned = run_id.clone();
    tauri::async_runtime::spawn(async move {
        run_turn(
            runtime,
            session_id,
            run_id,
            model,
            Some(text),
            context_size,
            settings,
            assistant,
            on_event,
            sequence,
            receiver,
            model_guard,
            memory_target,
        )
        .await;
    });
    Ok(returned)
}
/// Continues an answer that stopped at the output limit in a new run with a fresh budget.
#[tauri::command]
pub async fn continue_turn(
    state: State<'_, Arc<Runtime>>,
    session_id: String,
    message_id: String,
    model: String,
    on_event: Channel<Value>,
) -> Result<String, String> {
    let model_guard = state.foreground_read().await?;
    let memory_target = state.store.memory_turn_target(&message_id)?;
    let model = model.trim().to_string();
    if model.is_empty() || model.len() > 200 || model.contains(['\r', '\n']) {
        return Err("Chọn model Ollama hợp lệ trước khi tiếp tục".into());
    }
    let (context_size, settings) = state.store.session_config(&session_id)?;
    let run_id = Uuid::new_v4().to_string();
    let (cancel, receiver) = watch::channel(false);
    let assistant = {
        let mut active = state.active.lock().map_err(|_| "Runtime không khả dụng")?;
        if active.contains_key(&session_id) {
            return Err("Phiên này đang có một lượt chạy".into());
        }
        let assistant = state
            .store
            .begin_continue(&session_id, &run_id, &message_id, &model)?;
        active.insert(
            session_id.clone(),
            ActiveRun {
                id: run_id.clone(),
                cancel,
            },
        );
        assistant
    };
    let mut sequence = 0;
    if let Err(error) = emit(
        &on_event,
        &run_id,
        &session_id,
        &mut sequence,
        json!({"type":"started","message":assistant}),
    ) {
        let persisted = state
            .store
            .finish(&session_id, &assistant, "truncated", None);
        state
            .active
            .lock()
            .map_err(|_| "Runtime không khả dụng")?
            .remove(&session_id);
        persisted?;
        return Err(error);
    }
    let runtime = Arc::clone(state.inner());
    let returned = run_id.clone();
    tauri::async_runtime::spawn(async move {
        run_turn(
            runtime,
            session_id,
            run_id,
            model,
            None,
            context_size,
            settings,
            assistant,
            on_event,
            sequence,
            receiver,
            model_guard,
            memory_target,
        )
        .await;
    });
    Ok(returned)
}
#[tauri::command]
pub fn cancel_turn(
    state: State<'_, Arc<Runtime>>,
    session_id: String,
    run_id: String,
) -> Result<bool, String> {
    let active = state.active.lock().map_err(|_| "Runtime không khả dụng")?;
    if let Some(run) = active.get(&session_id) {
        if run.id == run_id {
            return Ok(run.cancel.send(true).is_ok());
        }
    }
    Ok(false)
}

pub(crate) async fn run_turn(
    runtime: Arc<Runtime>,
    session: String,
    run: String,
    model: String,
    text: Option<String>,
    context_size: usize,
    settings: InferenceSettings,
    mut assistant: Message,
    channel: Channel<Value>,
    mut sequence: u64,
    mut cancelled: watch::Receiver<bool>,
    _model_guard: OwnedRwLockReadGuard<()>,
    memory_target: Option<(String, String)>,
) {
    // A continuation starts from the saved partial answer and reasoning.
    let mut content = assistant.text.clone();
    let mut thinking = assistant.thinking.clone().unwrap_or_default();
    let mut metrics: Option<crate::storage::ChatMetrics> = assistant.metrics.clone();
    let result = if *cancelled.borrow() {
        Err("__cancelled__".into())
    } else {
        tokio::select! {
            _=cancelled.changed()=>Err("__cancelled__".into()),
            result=tokio::time::timeout(Duration::from_secs(300),async {
                match &text {
                    Some(text) => generate(&runtime,&session,&run,&model,text,context_size,&settings,&mut assistant,&channel,&mut sequence,&mut content,&mut thinking,&mut metrics).await,
                    None => continue_generate(&runtime,&session,&run,&model,context_size,&settings,&mut assistant,&channel,&mut sequence,&mut content,&mut thinking,&mut metrics).await,
                }
            })=>result.unwrap_or_else(|_|Err("Lượt chạy vượt quá 5 phút và đã dừng.".into())),
        }
    };
    assistant.text = content;
    assistant.thinking = if thinking.trim().is_empty() {
        None
    } else {
        Some(thinking)
    };
    assistant.metrics = metrics;
    assistant.model = Some(model.clone());
    let (status, error) = match result {
        Ok(false) => ("completed", None),
        Ok(true) => ("truncated", None),
        Err(e) if e == "__cancelled__" => ("cancelled", None),
        Err(e) => ("failed", Some(e)),
    };
    assistant.status = Some(status.into());
    let persisted = crate::memory_runtime::finish_turn(
        &runtime,
        &session,
        &assistant,
        status,
        error.as_deref(),
        &model,
        context_size,
        memory_target,
    );
    if let Ok(mut active) = runtime.active.lock() {
        if active.get(&session).is_some_and(|a| a.id == run) {
            active.remove(&session);
        }
    }
    let event = match persisted {
        Ok(()) => json!({"type":status,"message":assistant,"error":error}),
        Err(e) => {
            json!({"type":"failed","message":assistant,"error":format!("Không lưu được kết quả: {e}"),"storageError":true})
        }
    };
    let _ = emit(&channel, &run, &session, &mut sequence, event);
}

/// Messages the model should see: answered turns plus the latest question. A question whose
/// answer is missing, empty, failed or cancelled is dropped with it, so the model never gets two
/// user turns in a row (which made it reason about "which question to answer").
fn conversation(messages: Vec<Message>) -> Vec<Message> {
    let messages: Vec<Message> = messages
        .into_iter()
        .filter(|m| m.source != "ui_preview")
        .collect();
    let usable = |m: &Message| {
        m.role == "assistant"
            && !m.text.trim().is_empty()
            && matches!(m.status.as_deref(), Some("completed" | "truncated"))
    };
    let last_user = messages.iter().rposition(|m| m.role == "user");
    let mut kept = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let keep = if message.role == "user" {
            Some(index) == last_user || messages.get(index + 1).is_some_and(usable)
        } else {
            usable(message) && index > 0 && messages[index - 1].role == "user"
        };
        if keep && !message.text.is_empty() {
            kept.push(message.clone());
        }
    }
    kept
}
fn bounded_history(messages: Vec<Message>, budget: usize) -> Vec<Value> {
    let mut selected = Vec::new();
    let mut remaining = budget;
    for message in conversation(messages).into_iter().rev() {
        let cost = estimate_tokens(&message.text);
        if cost > remaining {
            break;
        }
        remaining -= cost;
        selected.push(json!({"role":message.role,"content":message.text}));
    }
    selected.reverse();
    while selected.first().is_some_and(|m| m["role"] == "assistant") {
        selected.remove(0);
    }
    selected
}
async fn generate(
    runtime: &Runtime,
    session: &str,
    run: &str,
    model: &str,
    text: &str,
    context_size: usize,
    settings: &InferenceSettings,
    assistant: &mut Message,
    channel: &Channel<Value>,
    sequence: &mut u64,
    content: &mut String,
    thinking: &mut String,
    metrics: &mut Option<crate::storage::ChatMetrics>,
) -> Result<bool, String> {
    let info = model_show(runtime, model).await;
    let context_size = clamp_context(context_size, info.as_ref(), channel, run, session, sequence);
    // Auto-compaction: summarize older turns before they push the conversation out.
    if settings.auto_compact {
        compact_with_events(runtime, session, model, context_size, settings, info.as_ref(), false, channel, run, session, sequence).await;
    }
    let (summary, until) = runtime.store.session_summary(session)?;
    let mut system = system_with_summary(&settings.system_prompt, summary.as_deref());
    if settings.agent_mode {
        system = format!("{system}\n\n{}", crate::agent::instructions()).trim().to_string();
    }
    let mut history = Vec::new();
    if !system.is_empty() {
        history.push(json!({"role":"system","content":system}));
    }
    let mut history_budget = input_budget(context_size, settings)
        .saturating_sub(summary.as_deref().map_or(0, |s| estimate_tokens(&summary_block(s))))
        .saturating_sub(if settings.agent_mode { estimate_tokens(&crate::agent::instructions()) + 600 } else { 0 });
    let mut retrieved_context = None;
    // Notes get at most 4.500 bytes and never the room the new message needs.
    let memory_budget = history_budget.saturating_sub(estimate_tokens(text) + 16).saturating_mul(3).min(4_500);

    let path = runtime.store.setting("vault")?;
    if !path.is_empty() {
        let config = crate::memory_runtime::settings(runtime, &path)?;
        let note_path = path.clone();
        let query = text.to_string();
        let result = tauri::async_runtime::spawn_blocking(move || {
            vault::search_vault(&note_path, &query, 4)
        })
        .await
        .map_err(|_| "Không đọc được bộ nhớ vault")??;
        let mut context_truncated = result.truncated;
        let mut hits = Vec::new();
        let mut memory_diagnostics = Vec::new();
        if config.enabled && config.use_memory {
            match crate::memory::recall(&path, text) {
                Ok(memories) => {
                    context_truncated |= memories.truncated || !memories.diagnostics.is_empty();
                    memory_diagnostics = memories.diagnostics;
                    let mut pinned_cost = 0usize;
                    for record in memories.records.iter().filter(|r| r.pinned) {
                        let hit = crate::memory_runtime::record_hit(record);
                        let mut next = hits.clone();
                        next.push(hit);
                        let cost = vault::memory_context(&next)
                            .len()
                            .saturating_sub(vault::memory_context(&hits).len());
                        if pinned_cost + cost <= memory_budget / 4 {
                            pinned_cost += cost;
                            hits = next;
                        } else {
                            context_truncated = true;
                        }
                    }
                    for record in memories.records.iter().filter(|r| !r.pinned).take(8) {
                        let mut next = hits.clone();
                        next.push(crate::memory_runtime::record_hit(record));
                        if vault::memory_context(&next).len() <= memory_budget {
                            hits = next;
                        } else {
                            context_truncated = true;
                        }
                    }
                }
                Err(error) => {
                    context_truncated = true;
                    memory_diagnostics.push(error);
                }
            }
        }
        for hit in &result.hits {
            let mut next = hits.clone();
            next.push(hit.clone());
            if vault::memory_context(&next).len() <= memory_budget {
                hits = next;
            } else {
                context_truncated = true;
            }
        }
        if !hits.is_empty() {
            // Keep full source records with the question in one user turn. Some
            // chat templates do not preserve two consecutive user messages.
            let context = vault::memory_context(&hits);
            history_budget = history_budget.saturating_sub(estimate_tokens(&context));
            retrieved_context = Some(context);
            assistant.sources = Some(hits.clone());
            runtime.store.save_sources(&assistant.id, &hits)?;

        }
        emit(
            channel,
            run,
            session,
            sequence,
            json!({"type":"memory_sources","sources":assistant.sources.clone().unwrap_or_default(),"truncated":context_truncated,"skippedFiles":result.skipped_files,"scannedFiles":result.scanned_files,"diagnostics":memory_diagnostics}),
        )?;
    }
    let transcript = bounded_history(after_summary(runtime.store.messages(session)?, until.as_deref()), history_budget);
    if !transcript
        .last()
        .is_some_and(|m| m["role"] == "user" && m["content"] == text)
    {
        return Err("Không thể giữ nguyên tin nhắn mới nhất trong context; lượt đã dừng.".into());
    }
    history.extend(transcript);
    if let Some(context) = retrieved_context {
        if let Some(last) = history.last_mut() {
            last["content"] = json!(format!("{context}\n\nCurrent user message:\n{text}"));
        }
    }
    let mut options = json!({"num_ctx":context_size,"temperature":settings.temperature,"top_p":settings.top_p,"top_k":settings.top_k,"repeat_penalty":settings.repeat_penalty});
    if let Some(seed) = settings.seed {
        options["seed"] = json!(seed);
    }
    // One output limit shared by thinking and answer; Ollama counts it exactly.
    options["num_predict"] = json!(settings.num_predict);
    let request = json!({"model":model,"messages":history,"stream":true,"options":options});
    let tools_capable = info.as_ref().is_some_and(|i| i["capabilities"].as_array().is_some_and(|c| c.iter().any(|v| v == "tools")));
    if settings.agent_mode {
        if !tools_capable {
            return Err(format!("Model {model} không hỗ trợ gọi công cụ nên không chạy được chế độ Agent. Chọn model có khả năng tools (ví dụ qwen3, qwen3.5, llama3.1)."));
        }
        return agent_loop(runtime, session, run, model, history, options, crate::agent::tool_specs(), context_size, settings, info.as_ref(), assistant, channel, sequence, content, thinking, metrics).await;
    }
    // Normal chat on a tools-capable model goes through the same loop with only the local
    // clock tool: that alone keeps Qwen-style reasoning short on simple prompts.
    if tools_capable {
        return agent_loop(runtime, session, run, model, history, options, crate::agent::chat_tool_specs(), context_size, settings, info.as_ref(), assistant, channel, sequence, content, thinking, metrics).await;
    }
    let mut counts = Counts { thinking: 0, answer: 0, budget: settings.num_predict as u64, agent: false, tool_calls: Vec::new() };
    let _ = emit(channel, run, session, sequence, counts.usage());
    let step = stream_chat(
        runtime,
        &request,
        assistant,
        channel,
        run,
        session,
        sequence,
        content,
        thinking,
        metrics,
        model,
        context_size,
        &mut counts,
    )
    .await?;
    let _ = emit(channel, run, session, sequence, counts.usage());
    outcome(step, content)
}
/// Answer from the user's Allow/Deny on an agent step.
#[tauri::command]
pub fn resolve_approval(state: State<'_, Arc<Runtime>>, approval_id: String, allow: bool) -> Result<bool, String> {
    let sender = state.approvals.lock().map_err(|_| "Runtime không khả dụng")?.remove(&approval_id);
    Ok(sender.is_some_and(|sender| sender.send(allow).is_ok()))
}
/// Agent mode: Ollama tool calling in a loop. Each round streams the model; tool calls are
/// executed (risky ones after the user's approval) and their results appended as `tool`
/// messages, until the model answers without tools or MAX_STEPS is reached.
#[allow(clippy::too_many_arguments)]
/// Debug smoke test of whole agent turns in the running app (real browser, real model):
/// `LIM_AGENT_SMOKE=<output file>`, `LIM_AGENT_SMOKE_PROMPTS="first|second|…"` (one session, so later
/// prompts see the earlier turns), `LIM_AGENT_MODEL` (default qwen3.5:2b), `LIM_DECISION_MODEL`
/// (stored as the decision_model setting of the test data). Approvals are denied.
#[cfg(debug_assertions)]
pub fn agent_smoke(runtime: Arc<Runtime>, output: std::path::PathBuf) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(6)).await;
        let prompts = std::env::var("LIM_AGENT_SMOKE_PROMPTS").unwrap_or_default();
        let model = std::env::var("LIM_AGENT_MODEL").unwrap_or_else(|_| "qwen3.5:2b".into());
        let settings = InferenceSettings { agent_mode: true, num_predict: 2048, ..Default::default() };
        let mut log = format!("model {model}\n");
        if let Ok(decider) = std::env::var("LIM_DECISION_MODEL") {
            let _ = runtime.store.set_setting("decision_model", &decider);
            log.push_str(&format!("decision model {decider}
"));
        }
        let Ok(session) = runtime.store.create_session() else { return };
        for prompt in prompts.split('|').map(str::trim).filter(|p| !p.is_empty()) {
            let run = Uuid::new_v4().to_string();
            let Ok((_, mut assistant)) = runtime.store.begin_with_memory_target(&session.id, &run, &model, prompt, None) else { break };
            let channel: Channel<Value> = Channel::new(|_| Ok(()));
            let (mut sequence, mut content, mut thinking, mut metrics) = (0, String::new(), String::new(), None);
            let deny = async {
                loop {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    let pending: Vec<String> = runtime.approvals.lock().map(|a| a.keys().cloned().collect()).unwrap_or_default();
                    for id in pending {
                        if let Some(sender) = runtime.approvals.lock().ok().and_then(|mut a| a.remove(&id)) {
                            let _ = sender.send(false);
                        }
                    }
                }
            };
            let started = std::time::Instant::now();
            let result = tokio::select! {
                result = generate(&runtime, &session.id, &run, &model, prompt, 8192, &settings, &mut assistant, &channel, &mut sequence, &mut content, &mut thinking, &mut metrics) => result,
                _ = deny => unreachable!(),
            };
            assistant.text = content.clone();
            let status = if result.is_ok() { "completed" } else { "failed" };
            let _ = runtime.store.finish(&session.id, &assistant, status, result.as_ref().err().map(String::as_str));
            log.push_str(&format!("\n===== {prompt} ({} s, {status})\n", started.elapsed().as_secs()));
            for step in assistant.steps.iter().flatten() {
                let text = step["output"].as_str().unwrap_or_default();
                let output = if step["tool"] == "recent_context" { text.lines().filter(|l| l.contains("Hiểu yêu cầu")).collect::<Vec<_>>().join(" ") } else { text.lines().take(2).collect::<Vec<_>>().join(" | ") };
                log.push_str(&format!("  - [{}] {} -> {}\n", step["status"].as_str().unwrap_or_default(), step["title"].as_str().unwrap_or_default(), output.chars().take(200).collect::<String>()));
            }
            log.push_str(&format!("  answer: {}\n", content.replace('\n', " ").chars().take(300).collect::<String>()));
            let _ = std::fs::write(&output, &log);
        }
        log.push_str("\n===== done\n");
        let _ = std::fs::write(&output, &log);
    });
}

/// The app runs a tool itself (shown as a step) when the model could not, e.g. opening the page
/// an open request asked for.
#[allow(clippy::too_many_arguments)]
async fn app_runs(client: &reqwest::Client, cap: usize, tool: &str, args: Value, channel: &Channel<Value>, run: &str, session: &str, sequence: &mut u64) -> Value {
    let mut step = json!({"id":Uuid::new_v4().to_string(),"tool":tool,"title":crate::agent::title(tool, &args),"args":args,"status":"running","output":""});
    let _ = emit(channel, run, session, sequence, json!({"type":"agent_step","step":step}));
    let (status, result) = match crate::agent::run(client, tool, &step["args"].clone(), cap).await {
        Ok(text) => ("done", text),
        Err(error) => ("error", format!("Lỗi: {error}")),
    };
    step["status"] = json!(status);
    step["output"] = json!(crate::agent::truncate(&result, 4_000));
    let _ = emit(channel, run, session, sequence, json!({"type":"agent_step","step":step}));
    step
}

/// The last `count` user/assistant pairs before the current message, as plain text.
fn recent_turns(messages: &[Value], count: usize) -> Vec<(String, String)> {
    let last_user = messages.iter().rposition(|m| m["role"] == "user").unwrap_or(messages.len());
    let mut pairs = Vec::new();
    let mut pending: Option<String> = None;
    for m in &messages[..last_user] {
        let text = m["content"].as_str().unwrap_or_default();
        match m["role"].as_str() {
            Some("user") => pending = Some(text.rsplit("Current user message:\n").next().unwrap_or(text).trim().to_string()),
            Some("assistant") if !text.trim().is_empty() => {
                if let Some(user) = pending.take() {
                    pairs.push((user, text.trim().to_string()));
                }
            }
            _ => {}
        }
    }
    let skip = pairs.len().saturating_sub(count);
    pairs.into_iter().skip(skip).collect()
}

/// The decision for this turn (see agent.rs, "Decision"): the decision model, on the CPU so it
/// stays loaded beside the chat model; if it is not installed or fails, the chat model itself
/// with the same prompt. None when neither answers.
async fn decide(runtime: &Runtime, model: &str, thinks: bool, context_size: usize, question: &str, history: &[(String, String)]) -> Option<crate::agent::Intent> {
    let shown = history.iter().map(|(u, a)| format!("{u}\n{a}")).collect::<Vec<_>>().join("\n");
    let chosen = runtime.store.setting("decision_model").ok().filter(|m| !m.trim().is_empty()).unwrap_or_else(|| crate::agent::DECISION_MODEL.to_string());
    let decider_thinks = model_show(runtime, &chosen).await.is_some_and(|i| i["capabilities"].as_array().is_some_and(|c| c.iter().any(|v| v == "thinking")));
    let mut candidates = vec![(chosen, decider_thinks, json!({"temperature": 0, "num_ctx": 2048, "num_predict": 300, "num_gpu": 0}))];
    // Same context size as the turn: a different num_ctx would make Ollama reload the chat model.
    candidates.push((model.to_string(), thinks, json!({"temperature": 0, "num_ctx": context_size, "num_predict": 300})));
    for (decider, decider_thinks, options) in candidates {
        // A follow-up is first rewritten to stand alone; the decision never sees the earlier turns.
        let rewritten = if history.is_empty() {
            None
        } else {
            let reply = structured(runtime, &decider, decider_thinks, options.clone(), crate::agent::rewrite_system(), crate::agent::rewrite_schema(), &crate::agent::decision_user(question, history)).await;
            reply.and_then(|r| r["message"].as_str().and_then(|m| crate::agent::standalone(question, m, &shown)))
        };
        let asked = crate::agent::decision_user(rewritten.as_deref().unwrap_or(question), &[]);
        let reply = structured(runtime, &decider, decider_thinks, options, crate::agent::decision_system(), crate::agent::decision_schema(), &asked).await;
        if let Some(mut intent) = reply.and_then(|m| crate::agent::parse_decision(&m.to_string())) {
            intent.refers_to_previous = rewritten.is_some();
            intent.earlier = shown;
            return Some(intent);
        }
    }
    None
}

/// One decision call whose answer must match `schema` (Ollama `format`); the parsed JSON, or None.
async fn structured(runtime: &Runtime, model: &str, thinks: bool, options: Value, system: &str, schema: Value, user: &str) -> Option<Value> {
    let mut body = json!({
        "model": model, "stream": false, "format": schema,
        "messages": [{"role":"system","content":system},{"role":"user","content":user}],
        "options": options
    });
    if thinks {
        body["think"] = json!(false);
    }
    let reply: Value = runtime.client.post(format!("{OLLAMA}/api/chat")).timeout(Duration::from_secs(90)).json(&body).send().await.ok()?.error_for_status().ok()?.json().await.ok()?;
    serde_json::from_str(reply["message"]["content"].as_str()?).ok()
}

async fn agent_loop(
    runtime: &Runtime,
    session: &str,
    run: &str,
    model: &str,
    mut messages: Vec<Value>,
    options: Value,
    tools: Value,
    context_size: usize,
    settings: &InferenceSettings,
    info: Option<&Value>,
    assistant: &mut Message,
    channel: &Channel<Value>,
    sequence: &mut u64,
    content: &mut String,
    thinking: &mut String,
    metrics: &mut Option<crate::storage::ChatMetrics>,
) -> Result<bool, String> {
    let client = crate::agent::client()?;
    let budget_bytes = input_budget(context_size, settings) * 3;
    // Each tool result is capped; old results shrink when they crowd the window.
    let cap = (budget_bytes / 3).clamp(1_500, 6_000);
    let mut steps: Vec<Value> = assistant.steps.clone().unwrap_or_default();
    let mut counts = Counts { thinking: 0, answer: 0, budget: settings.num_predict as u64, agent: true, tool_calls: Vec::new() };
    let mut total_output = 0u64;
    // After a denial the next round gets no tools, so the model answers instead of retrying.
    let mut force_answer = false;
    let thinks = info.is_some_and(|i| i["capabilities"].as_array().is_some_and(|c| c.iter().any(|v| v == "thinking")));
    let _ = emit(channel, run, session, sequence, counts.usage());
    // Memory v1 prepends recalled notes to the last user turn; only the question counts.
    let question = {
        let last_user = messages.iter().rev().find(|m| m["role"] == "user").and_then(|m| m["content"].as_str()).unwrap_or_default();
        last_user.rsplit("Current user message:\n").next().unwrap_or(last_user).trim().to_string()
    };
    // Every decision about the request comes from the decision model (see agent.rs, "Decision").
    let history = recent_turns(&messages, 2);
    let intent = if settings.agent_mode && steps.is_empty() {
        decide(runtime, model, thinks, context_size, &question, &history).await
    } else {
        None
    }
    .unwrap_or_else(crate::agent::Intent::none);
    // A new request is handled on its own, so the last song/page does not leak into it.
    if settings.agent_mode && steps.is_empty() {
        messages = crate::agent::focus_messages(messages, intent.refers_to_previous);
    }
    // Agent mode starts every new turn by checking the clock, so questions about "current"
    // facts are searched and dated against today rather than the model's training cutoff.
    // A continued answer already has it.
    if settings.agent_mode && steps.is_empty() {
        let args = json!({});
        let now = crate::agent::run(&client, "current_time", &args, cap).await?;
        let step = json!({"id":Uuid::new_v4().to_string(),"tool":"current_time","title":crate::agent::title("current_time", &args),"args":args,"status":"done","output":now});
        let _ = emit(channel, run, session, sequence, json!({"type":"agent_step","step":step}));
        steps.push(step);
        messages.push(json!({"role":"assistant","content":"","tool_calls":[{"function":{"name":"current_time","arguments":{}}}]}));
        messages.push(json!({"role":"tool","tool_name":"current_time","content":now}));
        // Follow-ups like "mở bài khác" / "mở đi" need what earlier turns found: history only
        // keeps answer text, so the open tabs and the real links tools returned are added here.
        let mut recent = crate::agent::recent_context(&runtime.store.messages(session)?, &crate::browser::tab_lines(), &question, intent.refers_to_previous);
        recent.push('\n');
        recent.push_str(&intent.note(&question));
        if !recent.is_empty() {
            let args = json!({});
            let step = json!({"id":Uuid::new_v4().to_string(),"tool":"recent_context","title":crate::agent::title("recent_context", &args),"args":args,"status":"done","output":recent});
            let _ = emit(channel, run, session, sequence, json!({"type":"agent_step","step":step}));
            steps.push(step);
            messages.push(json!({"role":"assistant","content":"","tool_calls":[{"function":{"name":"recent_context","arguments":{}}}]}));
            messages.push(json!({"role":"tool","tool_name":"recent_context","content":recent}));
        }
        // Questions about the present are searched right away (dated); a weak model (no
        // reasoning) gets every look-up searched for it.
        let pre_search = intent.pre_search(&question, thinks);
        if let Some(query) = pre_search {
            let args = json!({"query": query});
            let mut step = json!({"id":Uuid::new_v4().to_string(),"tool":"web_search","title":crate::agent::title("web_search", &args),"args":args,"status":"running","output":""});
            let _ = emit(channel, run, session, sequence, json!({"type":"agent_step","step":step}));
            let (status, result) = match crate::agent::run(&client, "web_search", &args, cap).await {
                Ok(text) => ("done", text),
                Err(error) => ("error", format!("Lỗi: {error}")),
            };
            step["status"] = json!(status);
            step["output"] = json!(crate::agent::truncate(&result, 4_000));
            let _ = emit(channel, run, session, sequence, json!({"type":"agent_step","step":step}));
            steps.push(step);
            messages.push(json!({"role":"assistant","content":"","tool_calls":[{"function":{"name":"web_search","arguments":args}}]}));
            messages.push(json!({"role":"tool","tool_name":"web_search","content":result}));
        }
    }
    // Everything the user wrote and every tool result of this turn: URLs the model opens must
    // come from here (kept apart from `messages`, whose old results get shrunk).
    let mut seen: String = messages.iter().filter(|m| m["role"] == "user" || m["role"] == "tool").filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n");
    let mut nudged = false;
    // Made-up URLs refused this turn; after two, the app opens the best real link itself.
    let mut refused = 0;
    let site = intent.site_label(&question);
    // A question is answered from web_search (SearXNG) and fetch_url; the browser is for opening.
    let tools = crate::agent::tools_for(&intent, &tools);
    for round in 0..=crate::agent::MAX_STEPS {
        let last_round = round == crate::agent::MAX_STEPS || force_answer;
        let mut request = json!({"model":model,"messages":messages,"stream":true,"options":options});
        // The final round has no tools so the model must answer with what it has.
        if !last_round {
            request["tools"] = tools.clone();
        }
        let before = content.len();
        let step = stream_chat(runtime, &request, assistant, channel, run, session, sequence, content, thinking, metrics, model, context_size, &mut counts).await?;
        total_output += metrics.as_ref().and_then(|m| m.output_tokens).unwrap_or(0);
        let mut calls = std::mem::take(&mut counts.tool_calls);
        // A weak model may write the call as JSON text: run it as a call and hide that text.
        if calls.is_empty() && !last_round {
            let parsed = crate::agent::calls_from_text(&content[before..]);
            if !parsed.is_empty() {
                calls = parsed;
                content.truncate(before);
                let _ = emit(channel, run, session, sequence, json!({"type":"text_reset","text":content.clone()}));
            }
        }
        let mut step = step;
        // The model stopped without acting: it wrote nothing ("đổi sang bài khác" → only
        // thoughts), or answered an open request with a bare link. Drop the draft and remind it
        // once, with tools still available. Measured: qwen3.5:2b often thinks "I'll search…"
        // and then ends the turn without the call.
        let opened = steps.iter().any(|s| matches!(s["tool"].as_str(), Some("browser_open" | "browser_search")) && s["status"] == "done");
        // Nothing, or a dangling start like "Câu trả lời:" (llama3.2:1b), counts as silent.
        let silent = { let draft = content[before..].trim(); draft.is_empty() || (draft.ends_with(':') && draft.chars().count() < 40) };
        let open_request = intent.opens() && !opened;
        if calls.is_empty() && !last_round && !nudged && matches!(step, Step::Done) && settings.agent_mode && (silent || open_request) {
            nudged = true;
            let draft = content[before..].to_string();
            content.truncate(before);
            let _ = emit(channel, run, session, sequence, json!({"type":"text_reset","text":content.clone()}));
            if !silent {
                messages.push(json!({"role":"assistant","content":draft}));
            }
            let reminder = if open_request { crate::agent::OPEN_REMINDER } else { crate::agent::ACT_REMINDER };
            messages.push(json!({"role":"user","content":reminder}));
            continue;
        }
        // Small models sometimes write the final answer inside an unclosed reasoning block, so
        // Ollama returns it as thinking with empty content. Ask once more for the answer only.
        if (calls.is_empty() || last_round) && matches!(step, Step::Done) && content[before..].trim().is_empty() {
            let mut retry = json!({"model":model,"messages":messages,"stream":true,"options":options});
            if thinks {
                retry["think"] = json!(false);
            }
            step = stream_chat(runtime, &retry, assistant, channel, run, session, sequence, content, thinking, metrics, model, context_size, &mut counts).await?;
            total_output += metrics.as_ref().and_then(|m| m.output_tokens).unwrap_or(0);
            counts.tool_calls.clear();
        }
        if calls.is_empty() || last_round || matches!(step, Step::Length) {
            // Asked to open something and the turn ends without that page open: the app searches
            // (when nothing was) and opens the best result itself, for any model. llama3.2:1b
            // opened the song in 1 of 3 tries on its own; qwen3.5:2b once said "Đã mở bài hát
            // Hello" with only a Google results page open.
            if settings.agent_mode && intent.opens() && crate::agent::opened_page(&steps, &site).is_none() {
                if crate::agent::first_result_url(&steps, &site).is_none() {
                    let args = json!({"query": intent.query(&question)});
                    steps.push(app_runs(&client, cap, "web_search", args, channel, run, session, sequence).await);
                }
                if let Some(url) = crate::agent::first_result_url(&steps, &site) {
                    let step_entry = app_runs(&client, cap, "browser_open", json!({"url": url}), channel, run, session, sequence).await;
                    let done = step_entry["status"] == "done";
                    steps.push(step_entry);
                    if done {
                        // What the model wrote did not open the page; say what was opened instead.
                        content.clear();
                        let _ = emit(channel, run, session, sequence, json!({"type":"text_reset","text":""}));
                    }
                }
            }
            // An answer that is only a broken call written as text is no answer.
            if crate::agent::looks_like_call_text(&content[before..]) {
                content.truncate(before);
                let _ = emit(channel, run, session, sequence, json!({"type":"text_reset","text":content.clone()}));
            }
            // A weak model's own words after opening a page were often wrong ("Tôi xin lỗi vì không
            // thể thực hiện" with the song playing) or a pasted page read-out: for an open request,
            // or any pasted read-out, state what is open instead.
            let page_open = steps.iter().any(|s| matches!(s["tool"].as_str(), Some("browser_open" | "browser_search" | "browser_click" | "browser_back")) && s["status"] == "done");
            if settings.agent_mode && !thinks && page_open && (intent.opens() || crate::agent::echoes_tool_output(content)) {
                content.clear();
                let _ = emit(channel, run, session, sequence, json!({"type":"text_reset","text":""}));
            }
            // The work is done (e.g. the song is open) but the small model wrote nothing, even on
            // the retry: say what was done instead of failing the whole turn.
            if matches!(step, Step::Done) && content.trim().is_empty() && !steps.is_empty() {
                let note = crate::agent::fallback_answer(&steps);
                content.push_str(&note);
                let _ = emit(channel, run, session, sequence, json!({"type":"text_delta","delta":note}));
            }
            // Links no tool returned are flagged, so a made-up address does not look verified.
            let unverified = crate::agent::unverified_links(&content[before..], &seen);
            if !unverified.is_empty() {
                let note = format!("\n\n> ⚠️ Link chưa kiểm chứng (không có trong kết quả tra cứu, có thể sai): {}", unverified.join(", "));
                content.push_str(&note);
                let _ = emit(channel, run, session, sequence, json!({"type":"text_delta","delta":note}));
            }
            if let Some(m) = metrics.as_mut() {
                m.output_tokens = Some(total_output);
            }
            assistant.steps = (!steps.is_empty()).then(|| steps.clone());
            let _ = emit(channel, run, session, sequence, counts.usage());
            return outcome(step, content);
        }
        messages.push(json!({"role":"assistant","content":content[before..].to_string(),"tool_calls":calls}));
        for call in calls {
            let tool = call["function"]["name"].as_str().unwrap_or_default().to_string();
            // Weak models send the parameter schema instead of values; fix the call first.
            let (tool, args, _repaired) = crate::agent::repair_call(&tool, crate::agent::arguments(&call), &intent.query(&question));
            let id = Uuid::new_v4().to_string();
            let approval = crate::agent::needs_approval(&tool);
            let mut entry = json!({"id":id,"tool":tool,"title":crate::agent::title(&tool, &args),"args":args,"status":if approval { "waiting" } else { "running" },"output":""});
            steps.push(entry.clone());
            let index = steps.len() - 1;
            // Once the user denied something this round, further risky calls are denied too.
            let allowed = if approval && force_answer {
                false
            } else if approval {
                let (sender, receiver) = tokio::sync::oneshot::channel();
                runtime.approvals.lock().map_err(|_| "Runtime không khả dụng")?.insert(id.clone(), sender);
                entry["approvalId"] = json!(id);
                steps[index] = entry.clone();
                let _ = emit(channel, run, session, sequence, json!({"type":"agent_step","step":entry}));
                let answer = tokio::time::timeout(Duration::from_secs(600), receiver).await;
                if let Ok(mut pending) = runtime.approvals.lock() {
                    pending.remove(&id);
                }
                matches!(answer, Ok(Ok(true)))
            } else {
                let _ = emit(channel, run, session, sequence, json!({"type":"agent_step","step":entry}));
                true
            };
            let (status, result) = if !allowed {
                force_answer = true;
                ("denied", "Người dùng đã từ chối thao tác này. Không thử lại; hãy trả lời với thông tin hiện có hoặc hỏi người dùng.".to_string())
            } else {
                entry["status"] = json!("running");
                let _ = emit(channel, run, session, sequence, json!({"type":"agent_step","step":entry}));
                let ran = match crate::agent::ungrounded(&tool, &args, &seen) {
                    Some(refusal) => {
                        refused += 1;
                        Err(refusal)
                    }
                    None => crate::agent::run(&client, &tool, &args, cap).await,
                };
                match ran {
                    Ok(text) => ("done", text),
                    Err(error) => ("error", format!("Lỗi: {error}")),
                }
            };
            entry["status"] = json!(status);
            entry["output"] = json!(crate::agent::truncate(&result, 4_000));
            entry.as_object_mut().map(|o| o.remove("approvalId"));
            steps[index] = entry.clone();
            let _ = emit(channel, run, session, sequence, json!({"type":"agent_step","step":entry}));
            // Only real results count: an error (e.g. the refusal of a made-up URL) quotes that
            // URL, and must not make it "seen" for the model's next try.
            if status == "done" {
                seen.push('\n');
                seen.push_str(&result);
            }
            messages.push(json!({"role":"tool","tool_name":tool,"content":result}));
        }
        // An open request is done once the page is open: qwen3.5:2b kept opening more tabs and wrote
        // an essay after the song was playing. One that keeps inventing URLs (5 refused in a row
        // for vnexpress) gets the best real link opened by the app, for any model.
        if settings.agent_mode && intent.opens() {
            if crate::agent::opened_page(&steps, &site).is_none() && refused >= 2 {
                if let Some(url) = crate::agent::first_result_url(&steps, &site) {
                    steps.push(app_runs(&client, cap, "browser_open", json!({"url": url}), channel, run, session, sequence).await);
                }
            }
            if let Some((title, url)) = crate::agent::opened_page(&steps, &site) {
                content.clear();
                let _ = emit(channel, run, session, sequence, json!({"type":"text_reset","text":""}));
                let note = format!("Đã mở **{}** trong trình duyệt bên cạnh: {url}", if title.is_empty() { &url } else { &title });
                content.push_str(&note);
                let _ = emit(channel, run, session, sequence, json!({"type":"text_delta","delta":note}));
                if let Some(m) = metrics.as_mut() {
                    m.output_tokens = Some(total_output);
                }
                assistant.steps = Some(steps.clone());
                let _ = emit(channel, run, session, sequence, counts.usage());
                return Ok(false);
            }
        }
        // Keep tool results within ~60% of the input budget: shrink the oldest first.
        let mut tool_bytes: usize = messages.iter().filter(|m| m["role"] == "tool").map(|m| m["content"].as_str().map_or(0, str::len)).sum();
        for message in messages.iter_mut().filter(|m| m["role"] == "tool") {
            if tool_bytes * 10 <= budget_bytes * 6 {
                break;
            }
            let size = message["content"].as_str().map_or(0, str::len);
            message["content"] = json!("[Kết quả cũ đã được rút gọn để tiết kiệm context]");
            tool_bytes = tool_bytes.saturating_sub(size);
        }
        if !content.is_empty() && !content.ends_with('\n') {
            content.push_str("\n\n");
        }
    }
    unreachable!("the last round always returns")
}
/// Ok(true): stopped at the output limit (the answer can be continued); Ok(false): finished.
fn outcome(step: Step, content: &str) -> Result<bool, String> {
    match step {
        Step::Length => Ok(true),
        _ => completed(content).map(|()| false),
    }
}
/// Continues an answer that stopped at the output limit, with a fresh output budget.
/// The reasoning so far is passed as a note; an answer already started is prefilled so
/// Ollama writes on from its last word.
#[allow(clippy::too_many_arguments)]
async fn continue_generate(
    runtime: &Runtime,
    session: &str,
    run: &str,
    model: &str,
    context_size: usize,
    settings: &InferenceSettings,
    assistant: &mut Message,
    channel: &Channel<Value>,
    sequence: &mut u64,
    content: &mut String,
    thinking: &mut String,
    metrics: &mut Option<crate::storage::ChatMetrics>,
) -> Result<bool, String> {
    let info = model_show(runtime, model).await;
    let context_size = clamp_context(context_size, info.as_ref(), channel, run, session, sequence);
    let (summary, until) = runtime.store.session_summary(session)?;
    crate::memory_runtime::check_continuation_sources(runtime, assistant)?;
    let all = after_summary(runtime.store.messages(session)?, until.as_deref());
    let index = all.iter().position(|m| m.id == assistant.id).ok_or("Không tìm thấy câu trả lời cần tiếp tục")?;

    let prior: Vec<Message> = all.into_iter().take(index).collect();
    // Token budget: what is already written plus the conversation, the rest is new output.
    let capacity = context_size;
    // Qwen-style models (ChatML with <think>) can resume the reasoning itself: the prompt is
    // built raw and ends inside the open <think> block (or after it, inside the answer).
    let resume_thinking = info.as_ref().is_some_and(chatml_thinking);
    let mut system = system_with_summary(&settings.system_prompt, summary.as_deref());
    let carried_thinking = if resume_thinking {
        estimate_tokens(thinking)
    } else {
        // Other models: thinking off, the newest part of the draft passed as a note.
        let mut draft = thinking.trim();
        let limit = capacity; // bytes: about a third of the window in tokens
        if draft.len() > limit {
            let mut cut = draft.len() - limit;
            while !draft.is_char_boundary(cut) {
                cut += 1;
            }
            draft = &draft[cut..];
        }
        if !draft.is_empty() {
            let note = if content.trim().is_empty() {
                format!("Ghi chú suy nghĩ nháp của trợ lý cho câu hỏi cuối (đã dừng vì hết token). Dùng nếu hữu ích, rồi trả lời trực tiếp:
{draft}")
            } else {
                format!(
                    "Ghi chú suy nghĩ của trợ lý cho câu hỏi cuối:
{draft}"
                )
            };
            system = if system.is_empty() {
                note
            } else {
                format!(
                    "{system}

{note}"
                )
            };
        }
        0
    };
    let mut history = Vec::new();
    if !system.is_empty() {
        history.push(json!({"role":"system","content":system}));
    }
    let mut carried = estimate_tokens(&system) + carried_thinking + estimate_tokens(content) + 256;
    let mut retrieved_context = None;
    if let Some(hits) = assistant.sources.as_ref().filter(|hits| !hits.is_empty()) {
        let context = vault::memory_context(hits);
        carried += estimate_tokens(&context);
        retrieved_context = Some(context);

    }
    // Keep room for a useful amount of output, then give the continuation whatever is left.
    let wanted_output = settings.num_predict.min(512);
    let transcript = bounded_history(prior, capacity.saturating_sub(carried + wanted_output));
    if !transcript.last().is_some_and(|m| m["role"] == "user") {
        return Err("Không đủ context để tiếp tục câu trả lời này. Tăng Context window (ví dụ 8K) rồi bấm Tiếp tục.".into());
    }
    let transcript_tokens: usize = transcript.iter().map(|m| estimate_tokens(m["content"].as_str().unwrap_or_default())).sum();
    let room = capacity.saturating_sub(carried + transcript_tokens);

    let num_predict = settings.num_predict.min(room);
    if num_predict < 64 {
        return Err(format!("Phần đã viết gần chiếm hết context {context_size} token nên không còn chỗ để tiếp tục. Tăng Context window rồi bấm Tiếp tục."));
    }
    history.extend(transcript);
    if let Some(context) = retrieved_context {
        if let Some(last) = history.last_mut() {
            let question = last["content"].as_str().unwrap_or_default();
            last["content"] = json!(format!("{context}\n\nCurrent user message:\n{question}"));
        }
    }
    let mut options = json!({"num_ctx":context_size,"num_predict":num_predict,"temperature":settings.temperature,"top_p":settings.top_p,"top_k":settings.top_k,"repeat_penalty":settings.repeat_penalty});
    if let Some(seed) = settings.seed {
        options["seed"] = json!(seed);
    }
    let (path, request, mut split) = if resume_thinking {
        let mut prompt = String::new();
        for message in &history {
            prompt.push_str(&format!(
                "<|im_start|>{}\n{}<|im_end|>\n",
                message["role"].as_str().unwrap_or("user"),
                message["content"].as_str().unwrap_or_default()
            ));
        }
        prompt.push_str("<|im_start|>assistant\n<think>\n");
        prompt.push_str(thinking);
        let answering = !content.trim().is_empty();
        if answering {
            prompt.push_str("\n</think>\n\n");
            prompt.push_str(content);
        }
        (
            "/api/generate",
            json!({"model":model,"prompt":prompt,"raw":true,"stream":true,"options":options}),
            Some(ThinkSplit::new(!answering)),
        )
    } else {
        if !content.trim().is_empty() {
            history.push(json!({"role":"assistant","content":content.clone()}));
        }
        (
            "/api/chat",
            json!({"model":model,"messages":history,"stream":true,"think":false,"options":options}),
            None,
        )
    };
    let previous = assistant.metrics.clone();
    // The answer's limit grows by this run's limit: 1.047 → 2.094 → 3.141 …
    let granted_before = previous
        .as_ref()
        .and_then(|m| m.output_limit.or(m.output_tokens))
        .unwrap_or(settings.num_predict as u64);
    let mut counts = Counts {
        thinking: previous
            .as_ref()
            .and_then(|m| m.thinking_tokens)
            .unwrap_or(0),
        answer: previous.as_ref().and_then(|m| m.answer_tokens).unwrap_or(0),
        budget: granted_before + num_predict as u64,
        agent: false,
        tool_calls: Vec::new(),
    };
    let _ = emit(channel, run, session, sequence, counts.usage());
    let step = stream_from(
        runtime,
        path,
        &request,
        split.as_mut(),
        assistant,
        channel,
        run,
        session,
        sequence,
        content,
        thinking,
        metrics,
        model,
        context_size,
        &mut counts,
    )
    .await?;
    // Report the whole answer, not only this continuation request.
    if let (Some(m), Some(before)) = (metrics.as_mut(), previous.and_then(|p| p.output_tokens)) {
        m.output_tokens = Some(m.output_tokens.unwrap_or(0) + before);
        if let Ok(raw) = serde_json::to_value(&*m) {
            let _ = emit(
                channel,
                run,
                session,
                sequence,
                json!({"type":"metrics","metrics":raw}),
            );
        }
    }
    let _ = emit(channel, run, session, sequence, counts.usage());
    outcome(step, content)
}
/// Splits raw ChatML output into thinking and answer at `</think>`, even when the tag is
/// cut across stream chunks.
pub(crate) struct ThinkSplit {
    thinking: bool,
    pending: String,
    trim_answer: bool,
}
impl ThinkSplit {
    fn new(thinking: bool) -> Self {
        Self {
            thinking,
            pending: String::new(),
            trim_answer: false,
        }
    }
    fn answer(&mut self, text: &str) -> String {
        let mut text = text.replace("<|im_end|>", "");
        if self.trim_answer {
            text = text.trim_start().to_string();
            self.trim_answer = text.is_empty();
        }
        text
    }
    /// Returns (thinking delta, answer delta).
    fn push(&mut self, chunk: &str, done: bool) -> (String, String) {
        if !self.thinking {
            return (String::new(), self.answer(chunk));
        }
        self.pending.push_str(chunk);
        const TAG: &str = "</think>";
        if let Some(index) = self.pending.find(TAG) {
            let thought = self.pending[..index].to_string();
            let rest = self.pending[index + TAG.len()..].to_string();
            self.pending.clear();
            self.thinking = false;
            self.trim_answer = true;
            return (thought, self.answer(&rest));
        }
        if done {
            return (std::mem::take(&mut self.pending), String::new());
        }
        // Hold back a tail that could be the start of the tag.
        let keep = (1..TAG.len())
            .rev()
            .find(|k| self.pending.ends_with(&TAG[..*k]))
            .unwrap_or(0);
        let ready = self.pending.len() - keep;
        let thought = self.pending[..ready].to_string();
        self.pending.drain(..ready);
        (thought, String::new())
    }
}
#[allow(clippy::too_many_arguments)]
async fn stream_chat(
    runtime: &Runtime,
    request: &Value,
    assistant: &mut Message,
    channel: &Channel<Value>,
    run: &str,
    session: &str,
    sequence: &mut u64,
    content: &mut String,
    thinking: &mut String,
    metrics: &mut Option<crate::storage::ChatMetrics>,
    model: &str,
    context_size: usize,
    counts: &mut Counts,
) -> Result<Step, String> {
    stream_from(
        runtime,
        "/api/chat",
        request,
        None,
        assistant,
        channel,
        run,
        session,
        sequence,
        content,
        thinking,
        metrics,
        model,
        context_size,
        counts,
    )
    .await
}
/// Streams from Ollama. With a splitter, raw `/api/generate` frames are turned into chat-style
/// frames (message.thinking / message.content) so `consume` handles both endpoints.
#[allow(clippy::too_many_arguments)]
async fn stream_from(
    runtime: &Runtime,
    path: &str,
    request: &Value,
    mut split: Option<&mut ThinkSplit>,
    assistant: &mut Message,
    channel: &Channel<Value>,
    run: &str,
    session: &str,
    sequence: &mut u64,
    content: &mut String,
    thinking: &mut String,
    metrics: &mut Option<crate::storage::ChatMetrics>,
    model: &str,
    context_size: usize,
    counts: &mut Counts,
) -> Result<Step, String> {
    let response = runtime
        .client
        .post(format!("{OLLAMA}{path}"))
        .json(request)
        .send()
        .await
        .map_err(|_| {
            "Không kết nối được Ollama. Kiểm tra Ollama đang mở và model đã tải.".to_string()
        })?;
    if !response.status().is_success() {
        return Err(format!(
            "Ollama trả HTTP {}. Kiểm tra model đã tải và đang khả dụng.",
            response.status()
        ));
    }
    // Dropping the response on an early return closes the connection, which stops Ollama.
    let mut stream = response.bytes_stream();
    let mut pending = Vec::new();
    loop {
        let next = tokio::time::timeout(Duration::from_secs(120), stream.next())
            .await
            .map_err(|_| "Ollama không gửi dữ liệu trong 2 phút; lượt đã dừng.")?;
        match next {
            Some(Ok(bytes)) => {
                pending.extend_from_slice(&bytes);
                if pending.len() > 1_000_000 {
                    return Err("Khung stream từ Ollama quá lớn".into());
                }
                while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=end).collect();
                    let line = match split.as_deref_mut() {
                        Some(split) => raw_frame(&line, split)?,
                        None => line,
                    };
                    match consume(
                        &line,
                        runtime,
                        assistant,
                        channel,
                        run,
                        session,
                        sequence,
                        content,
                        thinking,
                        metrics,
                        model,
                        context_size,
                        counts,
                    )? {
                        Step::Continue => {}
                        step => return Ok(step),
                    }
                }
            }
            Some(Err(_)) => {
                return Err("Kết nối Ollama bị gián đoạn; phần đã nhận được giữ lại.".into())
            }
            None => {
                if !pending.is_empty() {
                    let pending = match split.as_deref_mut() {
                        Some(split) => raw_frame(&pending, split)?,
                        None => pending,
                    };
                    match consume(
                        &pending,
                        runtime,
                        assistant,
                        channel,
                        run,
                        session,
                        sequence,
                        content,
                        thinking,
                        metrics,
                        model,
                        context_size,
                        counts,
                    )? {
                        Step::Continue => {}
                        step => return Ok(step),
                    }
                }
                return Err("Ollama đóng stream trước tín hiệu hoàn tất.".into());
            }
        }
    }
}
fn raw_frame(line: &[u8], split: &mut ThinkSplit) -> Result<Vec<u8>, String> {
    if line.iter().all(u8::is_ascii_whitespace) {
        return Ok(line.to_vec());
    }
    let mut frame: Value =
        serde_json::from_slice(line).map_err(|_| "Ollama trả khung JSON không hợp lệ")?;
    let done = frame.get("done").and_then(Value::as_bool) == Some(true);
    let (thought, answer) = split.push(frame["response"].as_str().unwrap_or_default(), done);
    frame["message"] = json!({"role":"assistant","thinking":thought,"content":answer});
    serde_json::to_vec(&frame).map_err(|e| e.to_string())
}
/// True for models whose chat format is ChatML with a <think> block (Qwen3 family, including
/// Qwen GGUFs from Hugging Face), which lets a continuation resume the reasoning itself.
fn chatml_thinking(info: &Value) -> bool {

    let modelfile = info["modelfile"].as_str().unwrap_or_default();
    let template = info["template"].as_str().unwrap_or_default();
    modelfile
        .lines()
        .any(|line| line.trim().to_lowercase().starts_with("renderer qwen3"))
        || (template.contains("<|im_start|>") && template.contains("<think>"))
}
/// Uses at most the model's trained context; tells the UI when the session asks for more.
fn clamp_context(context_size: usize, info: Option<&Value>, channel: &Channel<Value>, run: &str, session: &str, sequence: &mut u64) -> usize {
    match info.and_then(context_limit) {
        Some(limit) if limit < context_size => {
            let _ = emit(channel, run, session, sequence, json!({"type":"notice","notice":format!("Model chỉ hỗ trợ context {limit} token; lượt này dùng {limit}.")}));
            limit
        }
        _ => context_size,
    }
}
#[allow(clippy::too_many_arguments)]
async fn compact_with_events(runtime: &Runtime, session_id: &str, model: &str, context_size: usize, settings: &InferenceSettings, info: Option<&Value>, force: bool, channel: &Channel<Value>, run: &str, session: &str, sequence: &mut u64) {
    if !needs_compaction(runtime, session_id, context_size, settings, force).unwrap_or(false) {
        return;
    }
    let _ = emit(channel, run, session, sequence, json!({"type":"compacting"}));
    let event = match compact(runtime, session_id, model, context_size, settings, info, force).await {
        Ok(Some(count)) => json!({"type":"compacted","count":count}),
        Ok(None) => json!({"type":"compacted","count":0}),
        // Compaction is best effort: the turn continues with the rolling window.
        Err(error) => json!({"type":"notice","notice":format!("Không thu gọn được hội thoại ({error}); dùng các tin gần nhất vừa context.")}),
    };
    let _ = emit(channel, run, session, sequence, event);
}
/// Conversation after the summary, in the form the model sees it.
fn compactable(runtime: &Runtime, session: &str) -> Result<(Option<String>, Vec<Message>), String> {
    let (summary, until) = runtime.store.session_summary(session)?;
    Ok((summary, conversation(after_summary(runtime.store.messages(session)?, until.as_deref()))))
}
fn needs_compaction(runtime: &Runtime, session: &str, context_size: usize, settings: &InferenceSettings, force: bool) -> Result<bool, String> {
    let (summary, messages) = compactable(runtime, session)?;
    if messages.len() < 4 {
        return Ok(false);
    }
    let used = summary.as_deref().map_or(0, |s| estimate_tokens(&summary_block(s))) + messages.iter().map(|m| estimate_tokens(&m.text)).sum::<usize>();
    Ok(force || used * 100 > input_budget(context_size, settings) * COMPACT_AT_PERCENT)
}
/// Summarizes the oldest turns with the session's own model and stores the rolling summary.
/// The newest turns (about KEEP_RECENT_PERCENT of the input budget, at least the last turn)
/// stay verbatim. Returns how many messages were folded into the summary.
async fn compact(runtime: &Runtime, session: &str, model: &str, context_size: usize, settings: &InferenceSettings, info: Option<&Value>, force: bool) -> Result<Option<usize>, String> {
    let (summary, messages) = compactable(runtime, session)?;
    let budget = input_budget(context_size, settings);
    // Cut so the kept tail starts at a user message and holds at least one full turn.
    let mut cut = messages.len();
    let mut kept = 0;
    while cut > 0 {
        let cost = estimate_tokens(&messages[cut - 1].text);
        if messages.len() - cut >= 2 && (kept + cost) * 100 > budget * KEEP_RECENT_PERCENT {
            break;
        }
        kept += cost;
        cut -= 1;
    }
    let last_turn = messages.iter().rposition(|m| m.role == "user").unwrap_or(0);
    cut = cut.min(last_turn);
    while cut > 0 && messages[cut].role != "user" {
        cut -= 1;
    }
    // The summarizer must fit too: drop whole turns from the end of the chunk until it does.
    let transcript = |end: usize| messages[..end].iter().map(|m| format!("{}: {}", if m.role == "user" { "Người dùng" } else { "Lim" }, m.text.trim())).collect::<Vec<_>>().join("\n\n");
    let room = context_size.saturating_sub(512 + 400 + summary.as_deref().map_or(0, estimate_tokens));
    while cut >= 2 && estimate_tokens(&transcript(cut)) > room {
        cut -= 1;
        while cut > 0 && messages[cut].role != "user" {
            cut -= 1;
        }
    }
    if cut < 2 {
        return if force { Err("Chưa có đủ lượt cũ để thu gọn".into()) } else { Ok(None) };
    }
    let previous = summary.as_deref().filter(|s| !s.trim().is_empty()).map(|s| format!("Tóm tắt trước đó:\n{s}\n\n")).unwrap_or_default();
    let mut request = json!({
        "model": model,
        "stream": false,
        "messages": [
            {"role":"system","content":"Bạn tóm tắt hội thoại cho trợ lý Lim để dùng làm ngữ cảnh. Viết bằng tiếng Việt, tối đa khoảng 200 từ, dạng gạch đầu dòng ngắn. Giữ: mục tiêu và yêu cầu của người dùng, thông tin và quyết định quan trọng, tên riêng, số liệu, việc còn dang dở. Gộp với tóm tắt trước đó nếu có. Không thêm thông tin mới, không chào hỏi."},
            {"role":"user","content":format!("{previous}Phần hội thoại cần gộp vào tóm tắt:\n{}", transcript(cut))}
        ],
        "options": {"num_ctx": context_size, "num_predict": 512, "temperature": 0.3}
    });
    // Thinking adds nothing to a summary; only models that support it accept the flag.
    if info.is_some_and(|info| info["capabilities"].as_array().is_some_and(|c| c.iter().any(|v| v == "thinking"))) {
        request["think"] = json!(false);
    }
    let response = runtime.client.post(format!("{OLLAMA}/api/chat")).json(&request).timeout(Duration::from_secs(180)).send().await.map_err(|_| "không kết nối được Ollama")?;
    if !response.status().is_success() {
        return Err(format!("Ollama trả HTTP {}", response.status()));
    }
    let body: Value = response.json().await.map_err(|_| "phản hồi không hợp lệ")?;
    let text = body["message"]["content"].as_str().unwrap_or_default().trim().to_string();
    if text.is_empty() {
        return Err("model không trả bản tóm tắt".into());
    }
    runtime.store.set_session_summary(session, &text, &messages[cut - 1].id)?;
    Ok(Some(cut))
}
/// Manual "Thu gọn ngay": summarize older turns of an idle session now.
#[tauri::command]
pub async fn compact_session(state: State<'_, Arc<Runtime>>, session_id: String, model: String) -> Result<usize, String> {
    let _guard = Arc::clone(&state.model_gate).try_read_owned().map_err(|_| "Cookbook đang chuẩn bị model; thử lại sau.")?;
    if state.active.lock().map_err(|_| "Runtime không khả dụng")?.contains_key(&session_id) {
        return Err("Phiên đang chạy; đợi xong rồi thu gọn.".into());
    }
    let model = model.trim().to_string();
    if model.is_empty() {
        return Err("Chọn model Ollama để thu gọn.".into());
    }
    let (context_size, settings) = state.store.session_config(&session_id)?;
    let info = model_show(&state, &model).await;
    let context_size = info.as_ref().and_then(context_limit).map_or(context_size, |limit| context_size.min(limit));
    compact(&state, &session_id, &model, context_size, &settings, info.as_ref(), true).await.map(|count| count.unwrap_or(0))
}
fn completed(content: &str) -> Result<(), String> {
    if content.trim().is_empty() {
        Err("Model không trả nội dung văn bản.".into())
    } else {
        Ok(())
    }
}
fn consume(
    line: &[u8],
    runtime: &Runtime,
    assistant: &Message,
    channel: &Channel<Value>,
    run: &str,
    session: &str,
    sequence: &mut u64,
    content: &mut String,
    thinking: &mut String,
    metrics: &mut Option<crate::storage::ChatMetrics>,
    model: &str,
    context_size: usize,
    counts: &mut Counts,
) -> Result<Step, String> {
    if line.iter().all(u8::is_ascii_whitespace) {
        return Ok(Step::Continue);
    }
    let frame: Value =
        serde_json::from_slice(line).map_err(|_| "Ollama trả khung JSON không hợp lệ")?;
    if let Some(error) = frame.get("error").and_then(Value::as_str) {
        return Err(format!(
            "Ollama: {}",
            error.chars().take(300).collect::<String>()
        ));
    }
    if let Some(calls) = frame["message"]["tool_calls"].as_array().filter(|a| !a.is_empty()) {
        if !counts.agent {
            return Err("Model yêu cầu dùng công cụ; bật chế độ Agent để cho phép.".into());
        }
        counts.tool_calls.extend(calls.iter().cloned());
    }
    // Reasoning streams: Ollama thinking models expose `message.thinking`
    // (some builds use `message.reasoning` or top-level `thinking`).
    let mut thought = frame["message"]["thinking"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if thought.is_empty() {
        thought = frame["message"]["reasoning"]
            .as_str()
            .unwrap_or("")
            .to_string();
    }
    if thought.is_empty() {
        thought = frame
            .get("thinking")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
    }
    if !thought.is_empty() {
        if thinking.len() + thought.len() > 200_000 {
            return Err("Phần reasoning vượt giới hạn lưu 200 KB.".into());
        }
        thinking.push_str(&thought);
        let _ = runtime.store.checkpoint(&assistant.id, content, thinking);
        emit(
            channel,
            run,
            session,
            sequence,
            json!({"type":"thinking_delta","delta":thought}),
        )?;
        counts.thinking += 1;
        if counts.thinking.is_multiple_of(16) {
            let _ = emit(channel, run, session, sequence, counts.usage());
        }
    }
    if let Some(delta) = frame["message"]["content"].as_str() {
        if !delta.is_empty() {
            if content.len() + delta.len() > 200_000 {
                return Err("Phản hồi vượt giới hạn lưu 200 KB.".into());
            }
            content.push_str(delta);
            let _ = runtime.store.checkpoint(&assistant.id, content, thinking);
            emit(
                channel,
                run,
                session,
                sequence,
                json!({"type":"text_delta","delta":delta}),
            )?;
            counts.answer += 1;
            if counts.answer.is_multiple_of(16) {
                let _ = emit(channel, run, session, sequence, counts.usage());
            }
        }
    }
    let done = frame.get("done").and_then(Value::as_bool) == Some(true);
    let length = done && frame.get("done_reason").and_then(Value::as_str) == Some("length");
    if done {
        let eval_count = frame["eval_count"].as_u64();
        let eval_duration = frame["eval_duration"].as_u64();
        let prompt_count = frame["prompt_eval_count"].as_u64();
        let total_duration = frame["total_duration"].as_u64();
        let speed = match (eval_count, eval_duration) {
            (Some(tokens), Some(duration)) if tokens > 0 && duration > 0 => {
                Some(tokens as f64 * 1e9 / duration as f64)
            }
            _ => None,
        };
        if eval_count.is_some() || speed.is_some() {
            *metrics = Some(crate::storage::ChatMetrics {
                thinking_tokens: Some(counts.thinking),
                answer_tokens: Some(counts.answer),
                output_limit: Some(counts.budget),
                output_tokens: eval_count,
                prompt_tokens: prompt_count,
                eval_duration_ms: eval_duration.map(|d| d / 1_000_000),
                total_duration_ms: total_duration.map(|d| d / 1_000_000),
                tokens_per_second: speed,
            });
            if let (Some(tokens), Some(duration)) = (eval_count, eval_duration) {
                if tokens > 0 && duration > 0 {
                    let mut observations: Value = runtime
                        .store
                        .setting("model_observations")
                        .ok()
                        .and_then(|raw| serde_json::from_str(&raw).ok())
                        .filter(Value::is_object)
                        .unwrap_or_else(|| json!({}));
                    observations[model] = json!({ "model": model, "contextSize": context_size, "tokensPerSecond": tokens as f64 * 1e9 / duration as f64, "outputTokens": tokens, "measuredAt": crate::storage::now() });
                    // Optional observation must not turn a successfully saved answer into a failure.
                    let _ = runtime
                        .store
                        .set_setting("model_observations", &observations.to_string());
                }
            }
            if let Some(m) = metrics.as_ref() {
                if let Ok(raw) = serde_json::to_value(m) {
                    let _ = emit(
                        channel,
                        run,
                        session,
                        sequence,
                        json!({"type":"metrics","metrics":raw}),
                    );
                }
            }
        }
    }
    Ok(if length {
        Step::Length
    } else if done {
        Step::Done
    } else {
        Step::Continue
    })
}

#[cfg(test)]
mod split_tests {
    use super::ThinkSplit;
    #[test]
    fn splits_at_close_tag_across_chunks() {
        let mut split = ThinkSplit::new(true);
        let mut thought = String::new();
        let mut answer = String::new();
        for chunk in [" 3. Mía", "...</th", "ink>\n\nĐáp", " án<|im_end|>"] {
            let (t, a) = split.push(chunk, false);
            thought.push_str(&t);
            answer.push_str(&a);
        }
        assert_eq!(thought, " 3. Mía...");
        assert_eq!(answer, "Đáp án");
        // Still thinking at the end of the stream: everything held back is thinking.
        let mut open = ThinkSplit::new(true);
        let (t1, _) = open.push("chưa xong </", false);
        let (t2, a) = open.push("", true);
        assert_eq!(
            (format!("{t1}{t2}"), a),
            ("chưa xong </".to_string(), String::new())
        );
        // Answer already started: output is answer only.
        let (t, a) = ThinkSplit::new(false).push(" tiếp", false);
        assert_eq!((t, a), (String::new(), " tiếp".to_string()));
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    #[test]
    #[ignore]
    fn truncate_then_continue() {
        let dir = std::env::temp_dir().join(format!("lim-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let runtime =
            Runtime::new(crate::storage::Store::open(&dir.join("t.sqlite3")).unwrap()).unwrap();
        let session = runtime.store.create_session().unwrap().id;
        let model = "qwen3.5:2b";
        let channel: Channel<Value> = Channel::new(|_| Ok(()));
        let settings = InferenceSettings {
            num_predict: 48,
            ..Default::default()
        };
        // A question the 2B model can finish; long essays make qwen3.5:2b think for 3.500+ tokens.
        let prompt = "Thủ đô của Pháp là gì? Trả lời một câu.";
        // 1) A tiny output limit stops the answer: status must be "truncated", not failed.
        let run = Uuid::new_v4().to_string();
        let (_, mut assistant) = runtime.store.begin(&session, &run, model, prompt).unwrap();
        let (mut sequence, mut content, mut thinking, mut metrics) =
            (0, String::new(), String::new(), None);
        let first = tauri::async_runtime::block_on(generate(
            &runtime,
            &session,
            &run,
            model,
            prompt,
            4096,
            &settings,
            &mut assistant,
            &channel,
            &mut sequence,
            &mut content,
            &mut thinking,
            &mut metrics,
        ));
        println!(
            "first: {first:?} | thinking {} chars | answer {:?}",
            thinking.len(),
            content.chars().take(120).collect::<String>()
        );
        assert_eq!(first, Ok(true));
        assistant.text = content.clone();
        assistant.thinking = Some(thinking.clone()).filter(|t| !t.is_empty());
        assistant.metrics = metrics.clone();
        runtime
            .store
            .finish(&session, &assistant, "truncated", None)
            .unwrap();
        // 2) Continue with a fresh budget until the answer finishes (at most 15 rounds).
        let mut finished = false;
        for round in 1..=15 {
            let run = Uuid::new_v4().to_string();
            let mut assistant = runtime
                .store
                .begin_continue(&session, &run, &assistant.id, model)
                .unwrap();
            let before = assistant.text.clone();
            let (mut content, mut thinking, mut metrics) = (
                assistant.text.clone(),
                assistant.thinking.clone().unwrap_or_default(),
                assistant.metrics.clone(),
            );
            let more = InferenceSettings {
                num_predict: 400,
                ..Default::default()
            };
            let result = tauri::async_runtime::block_on(continue_generate(
                &runtime,
                &session,
                &run,
                model,
                4096,
                &more,
                &mut assistant,
                &channel,
                &mut sequence,
                &mut content,
                &mut thinking,
                &mut metrics,
            ));
            println!(
                "round {round}: {result:?} | +{} chars | tail {:?}",
                content.len() - before.len(),
                content
                    .chars()
                    .rev()
                    .take(80)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            );
            assert!(
                content.starts_with(&before),
                "continuation must keep the earlier text"
            );
            let thinking_before = assistant.thinking.clone().unwrap_or_default();
            if round == 1 && before.is_empty() {
                assert!(
                    thinking.len() > thinking_before.len(),
                    "Qwen continuation should resume the reasoning"
                );
                println!(
                    "    thinking resumed: +{} chars, seam {:?}",
                    thinking.len() - thinking_before.len(),
                    format!(
                        "{}⟦+⟧{}",
                        thinking_before
                            .chars()
                            .rev()
                            .take(30)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect::<String>(),
                        thinking[thinking_before.len()..]
                            .chars()
                            .take(40)
                            .collect::<String>()
                    )
                );
            }
            let seam: String = before
                .chars()
                .rev()
                .take(40)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .chain(
                    [
                        "⟦+⟧".chars().collect::<Vec<_>>(),
                        content[before.len()..].chars().take(50).collect(),
                    ]
                    .concat(),
                )
                .collect();
            println!("    seam: {seam:?}");
            let limit = metrics.as_ref().and_then(|m| m.output_limit);
            println!(
                "    output limit {limit:?} · output tokens {:?}",
                metrics.as_ref().and_then(|m| m.output_tokens)
            );
            if round == 1 {
                assert_eq!(limit, Some(48 + 400), "each continue adds its run's limit");
            }
            let truncated = result.clone().unwrap();
            assistant.text = content;
            assistant.thinking = Some(thinking).filter(|t| !t.is_empty());
            assistant.metrics = metrics;
            runtime
                .store
                .finish(
                    &session,
                    &assistant,
                    if truncated { "truncated" } else { "completed" },
                    None,
                )
                .unwrap();
            if !truncated {
                finished = true;
                break;
            }
        }
        assert!(finished, "answer should finish after continuing");
        drop(runtime);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
#[cfg(test)]
mod history_tests {
    use super::*;
    fn msg(role: &str, text: &str, status: &str) -> Message {
        Message {
            id: Uuid::new_v4().to_string(),
            role: role.into(),
            text: text.into(),
            source: if role == "user" {
                "user".into()
            } else {
                "model".into()
            },
            run_id: None,
            status: Some(status.into()),
            sources: None,
            model: None,
            thinking: None,
            metrics: None,
            error: None,
            steps: None,
        }
    }
    #[test]
    fn drops_unanswered_questions() {
        // Pháp answered, con bò stopped while thinking (empty), then "chào" with its running reply.
        let history = vec![
            msg("user", "viết về nước Pháp", "completed"),
            msg("assistant", "Nước Pháp là…", "completed"),
            msg("user", "viết về con bò", "completed"),
            msg("assistant", "", "truncated"),
            msg("user", "chào", "completed"),
            msg("assistant", "", "running"),
        ];
        let roles: Vec<String> = bounded_history(history, 10_000)
            .iter()
            .map(|m| {
                format!(
                    "{}:{}",
                    m["role"].as_str().unwrap(),
                    m["content"].as_str().unwrap()
                )
            })
            .collect();
        assert_eq!(
            roles,
            [
                "user:viết về nước Pháp",
                "assistant:Nước Pháp là…",
                "user:chào"
            ]
        );
        // A cut answer that already has text stays, with its question.
        let partial = vec![
            msg("user", "kể chuyện", "completed"),
            msg("assistant", "Ngày xưa", "truncated"),
            msg("user", "tiếp đi", "completed"),
        ];
        assert_eq!(bounded_history(partial, 10_000).len(), 3);
    }
}
#[cfg(test)]
mod compaction_tests {
    use super::*;
    fn store_with_turns(turns: &[(&str, &str)]) -> (Runtime, String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("lim-compact-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let runtime = Runtime::new(crate::storage::Store::open(&dir.join("t.sqlite3")).unwrap()).unwrap();
        let session = runtime.store.create_session().unwrap().id;
        for (question, answer) in turns {
            let run = Uuid::new_v4().to_string();
            let (_, mut assistant) = runtime.store.begin(&session, &run, "qwen3.5:2b", question).unwrap();
            assistant.text = answer.to_string();
            runtime.store.finish(&session, &assistant, "completed", None).unwrap();
        }
        (runtime, session, dir)
    }
    #[test]
    fn compaction_threshold() {
        let settings = InferenceSettings::default();
        let (short, session, dir) = store_with_turns(&[("hi", "Chào bạn"), ("khỏe không", "Khỏe")]);
        assert!(!needs_compaction(&short, &session, 4096, &settings, false).unwrap());
        drop(short);
        let _ = std::fs::remove_dir_all(dir);
        // 4K window, 2.048 output: input budget ~1.700 tokens; 6 turns of ~600 bytes pass 75%.
        let long = "Nội dung khá dài về kế hoạch dự án Lim Harness. ".repeat(12);
        let turns: Vec<(&str, &str)> = (0..6).map(|_| (long.as_str(), long.as_str())).collect();
        let (runtime, session, dir) = store_with_turns(&turns);
        assert!(needs_compaction(&runtime, &session, 4096, &settings, false).unwrap());
        assert!(!needs_compaction(&runtime, &session, 131072, &settings, false).unwrap());
        drop(runtime);
        let _ = std::fs::remove_dir_all(dir);
    }
    /// Live: `cargo test --lib compacts_with_local_model -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn compacts_with_local_model() {
        let facts = [
            ("Tên dự án của tôi là Lim Harness, viết bằng Tauri.", "Đã ghi nhận: Lim Harness dùng Tauri."),
            ("Hạn nộp đồ án là ngày 20 tháng 12.", "Đã ghi nhận hạn nộp 20/12."),
            ("Tôi muốn bộ nhớ dùng Obsidian vault.", "Được, bộ nhớ sẽ đọc từ Obsidian vault."),
            ("Model chính là qwen3.5:2b qua Ollama.", "Đã ghi nhận model qwen3.5:2b."),
            ("Giao diện tối giản, màu xanh lá.", "Đã ghi nhận giao diện tối giản xanh lá."),
            ("Giờ hãy nhắc lại hạn nộp giúp tôi.", "Hạn nộp là 20/12."),
        ];
        let padded: Vec<(String, String)> = facts.iter().map(|(q, a)| (format!("{q} {}", "Ghi chú thêm. ".repeat(25)), a.to_string())).collect();
        let turns: Vec<(&str, &str)> = padded.iter().map(|(q, a)| (q.as_str(), a.as_str())).collect();
        let (runtime, session, dir) = store_with_turns(&turns);
        let settings = InferenceSettings::default();
        let info = tauri::async_runtime::block_on(model_show(&runtime, "qwen3.5:2b"));
        let folded = tauri::async_runtime::block_on(compact(&runtime, &session, "qwen3.5:2b", 4096, &settings, info.as_ref(), false)).unwrap();
        let (summary, until) = runtime.store.session_summary(&session).unwrap();
        let kept = after_summary(runtime.store.messages(&session).unwrap(), until.as_deref());
        println!("folded {folded:?} messages, kept {} verbatim\nsummary:\n{}", kept.len(), summary.clone().unwrap_or_default());
        assert!(folded.unwrap_or(0) >= 2 && summary.is_some());
        assert!(kept.len() >= 2 && kept[0].role == "user", "newest turns stay verbatim, starting at a question");
        drop(runtime);
        let _ = std::fs::remove_dir_all(dir);
    }
}
#[cfg(test)]
mod agent_live_tests {
    use super::*;
    /// Runs one agent turn; `approve` auto-answers approval requests like the UI buttons would.
    fn agent_turn(prompt: &str, approve: bool) -> (Result<bool, String>, String, Vec<Value>) {
        let dir = std::env::temp_dir().join(format!("lim-agent-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let runtime = Runtime::new(crate::storage::Store::open(&dir.join("t.sqlite3")).unwrap()).unwrap();
        let session = runtime.store.create_session().unwrap().id;
        let run = Uuid::new_v4().to_string();
        // LIM_AGENT_MODEL picks another installed model (e.g. llama3.2:1b) for the same checks.
        let model_name = std::env::var("LIM_AGENT_MODEL").unwrap_or_else(|_| "qwen3.5:2b".to_string());
        let model = model_name.as_str();
        let (_, mut assistant) = runtime.store.begin(&session, &run, model, prompt).unwrap();
        let channel: Channel<Value> = Channel::new(|_| Ok(()));
        let settings = InferenceSettings { agent_mode: true, num_predict: 1536, ..Default::default() };
        let (mut sequence, mut content, mut thinking, mut metrics) = (0, String::new(), String::new(), None);
        let result = tauri::async_runtime::block_on(async {
            let answer = async {
                loop {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    let pending: Vec<String> = runtime.approvals.lock().unwrap().keys().cloned().collect();
                    for id in pending {
                        if let Some(sender) = runtime.approvals.lock().unwrap().remove(&id) {
                            println!("  approval requested -> {}", if approve { "allow" } else { "deny" });
                            let _ = sender.send(approve);
                        }
                    }
                }
            };
            tokio::select! {
                result = generate(&runtime, &session, &run, model, prompt, 8192, &settings, &mut assistant, &channel, &mut sequence, &mut content, &mut thinking, &mut metrics) => result,
                _ = answer => unreachable!(),
            }
        });
        let steps = assistant.steps.clone().unwrap_or_default();
        println!("  thinking {} chars, tail: {:?}", thinking.len(), thinking.chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect::<String>());
        println!("  metrics {:?}", metrics.as_ref().map(|m: &crate::storage::ChatMetrics| (m.output_tokens, m.thinking_tokens, m.answer_tokens)));
        drop(runtime);
        let _ = std::fs::remove_dir_all(&dir);
        (result, content, steps)
    }
    #[test]
    #[ignore = "Requires local Ollama; set LIM_AGENT_MODEL to try another model"]
    fn agent_opens_song() {
        let (result, answer, steps) = agent_turn("hãy mở bài gội đầu của thắng ngọt trên youtube", false);
        show(&steps);
        println!("result {result:?}
answer: {answer}");
    }
    /// Replays requests from the user's session (2026-11-13) and prints what would be opened.
    #[test]
    #[ignore = "Requires local Ollama; set LIM_AGENT_MODEL"]
    fn replay_session_requests() {
        for prompt in ["mở bài gội đầu của thắng ngọt", "truy cập trang chủ của bayern muchen", "đổi sang bải 2002 của anna", "mở bài period của iri trên youtube", "truy cập trang chủ MU"] {
            let (result, answer, steps) = agent_turn(prompt, false);
            let opens: Vec<String> = steps.iter().filter(|s| s["tool"] == "browser_open").map(|s| s["args"]["url"].as_str().unwrap_or("(index)").to_string()).collect();
            let search = steps.iter().find(|s| s["tool"] == "web_search").map(|s| s["args"]["query"].clone());
            println!("{prompt}
   search {search:?}
   open {opens:?}
   answer {:?} {result:?}", answer.chars().take(120).collect::<String>());
        }
    }
    #[test]
    fn recent_turns_for_the_decision() {
        let messages = vec![
            json!({"role":"system","content":"s"}),
            json!({"role":"user","content":"ai là tổng thống mỹ"}),
            json!({"role":"assistant","content":"Donald Trump."}),
            json!({"role":"user","content":"notes\n\nCurrent user message:\nông ấy bao nhiêu tuổi"}),
        ];
        assert_eq!(recent_turns(&messages, 2), vec![("ai là tổng thống mỹ".to_string(), "Donald Trump.".to_string())]);
    }

    /// Live: the decision through the app's own code (`decide`), on requests for many sites and
    /// on follow-ups. LIM_DECISION_MODEL picks the decision model (default granite4:3b);
    /// LIM_AGENT_MODEL is the chat model used when the decision model is missing.
    #[test]
    #[ignore = "Requires local Ollama with the decision model"]
    fn decision_live() {
        let model_name = std::env::var("LIM_AGENT_MODEL").unwrap_or_else(|_| "qwen3.5:2b".to_string());
        let dir = std::env::temp_dir().join(format!("lim-decision-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let runtime = Runtime::new(crate::storage::Store::open(&dir.join("t.sqlite3")).unwrap()).unwrap();
        if let Ok(decider) = std::env::var("LIM_DECISION_MODEL") {
            runtime.store.set_setting("decision_model", &decider).unwrap();
        }
        let thinks = model_name.starts_with("qwen");
        let cases = [
            ("vào shopee tìm tai nghe bluetooth", "open_page", "shopee"),
            ("đọc tin mới nhất trên vnexpress", "open_page", "vnexpress"),
            ("show me the github page of ollama", "open_page", "github"),
            ("mở bài Hello của Adele", "open_page", "youtube"),
            ("thủ đô của Úc là gì", "find_info", ""),
            ("liệt kê các file trong thư mục Downloads", "use_computer", ""),
            ("chào bạn", "chat", ""),
            ("mở bài period của iri trên youtube'", "open_page", "youtube"),
            ("cho mình xem giá iphone 16 trên thegioididong", "open_page", "thegioididong"),
            ("xem bản đồ đường đến hồ gươm", "open_page", "google"),
        ];
        let mut right = 0;
        for (question, action, site) in cases {
            let started = std::time::Instant::now();
            let intent = tauri::async_runtime::block_on(decide(&runtime, &model_name, thinks, 8192, question, &[])).expect("no decision");
            let ok = intent.action == action && (site.is_empty() || intent.query(question).to_lowercase().contains(site));
            right += ok as usize;
            println!("{question} -> {} {} | query {:?} | {} ms {}", intent.action, intent.kind, intent.query(question), started.elapsed().as_millis(), if ok { "" } else { "<--" });
        }
        assert!(right + 1 >= cases.len(), "{right}/{}", cases.len());

        // Follow-ups: the right name must reach the search; new requests must not carry old names.
        let turns = |user: &str, answer: &str| vec![(user.to_string(), answer.to_string())];
        let follow = [
            (turns("ai là tổng thống mỹ", "Tổng thống Mỹ hiện tại là Donald Trump."), "vợ của ông ấy đã mất vào năm nào và ở đâu", Some("trump"), None),
            (turns("ai là tổng thống mỹ hiện tại", "Tổng thống Mỹ hiện tại là Donald Trump."), "bao nhiêu tuổi rồi", Some("trump"), None),
            (turns("mở bài hello của adele", "Đã mở Adele - Hello."), "mở bài khác của cô ấy", Some("adele"), None),
            (turns("tìm giá iphone 16", "iPhone 16 giá khoảng 22 triệu."), "còn bản pro max", Some("iphone"), None),
            (turns("mở bài let me love you", "Đã tìm thấy DJ Snake - Let Me Love You trên YouTube."), "mở đi", Some("let me love you"), None),
            (turns("mở bài call me maybe", "Đã mở Carly Rae Jepsen - Call Me Maybe."), "thôi đổi sang bài 2002 đi", Some("2002"), Some("call me")),
            (turns("hãy giúp tôi mở bài gội đầu của thắng", "Đã mở THẮNG - GỘI ĐẦU ft. Hà Lê trên YouTube."), "vào đọc báo mới hôm nay trên vnexpress", Some("vnexpress"), Some("gội")),
            (turns("thủ đô của Úc là gì", "Canberra."), "giá vàng hôm nay bao nhiêu", Some("vàng"), Some("canberra")),
            (vec![("mở bài hello của adele".to_string(), "Đã mở Adele - Hello (Official Music Video).".to_string()), ("mở bài khác của cô ấy".to_string(), "Đã mở Adele Music Playlist.".to_string())], "thôi đổi sang bài 2002 đi", Some("2002"), Some("adele")),
        ];
        let mut follow_right = 0;
        for (history, question, must, must_not) in &follow {
            let intent = tauri::async_runtime::block_on(decide(&runtime, &model_name, thinks, 8192, question, history)).expect("no decision");
            let text = format!("{} {}", intent.subject, intent.query(question)).to_lowercase();
            let ok = must.is_none_or(|m| text.contains(m)) && must_not.is_none_or(|m| !text.contains(m));
            follow_right += ok as usize;
            println!("{question} -> {} refers {} | subject {:?} | query {:?} {}", intent.action, intent.refers_to_previous, intent.subject, intent.query(question), if ok { "" } else { "<--" });
        }
        assert!(follow_right + 1 >= follow.len(), "{follow_right}/{}", follow.len());

        // Questions after earlier turns stay questions: answered from search, not by opening pages.
        let opened = turns("mở bài hello của adele", "Đã mở Adele - Hello (Official Music Video).");
        let president = turns("ai là tổng thống mỹ", "Tổng thống Mỹ hiện tại là Donald Trump.");
        let asks = [
            (&president, "vợ của ông ấy đã mất vào năm nào và ở đâu", "find_info"),
            (&opened, "giá vàng hôm nay bao nhiêu", "find_info"),
            (&opened, "thủ đô của Úc là gì", "find_info"),
            (&president, "mở trang wikipedia của ông ấy", "open_page"),
        ];
        let mut ask_right = 0;
        for (history, question, action) in asks {
            let intent = tauri::async_runtime::block_on(decide(&runtime, &model_name, thinks, 8192, question, history)).expect("no decision");
            let ok = intent.action == action;
            ask_right += ok as usize;
            println!("{question} -> {} | query {:?} {}", intent.action, intent.query(question), if ok { "" } else { "<--" });
        }
        assert_eq!(ask_right, asks.len());
    }

    fn show(steps: &[Value]) {
        for step in steps {
            println!("  - [{}] {} -> {}", step["status"].as_str().unwrap_or(""), step["title"].as_str().unwrap_or(""), step["output"].as_str().unwrap_or("").chars().take(120).collect::<String>().replace('\n', " | "));
        }
    }
    /// `cargo test --lib agent_web -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn agent_web() {
        let (result, answer, steps) = agent_turn("Hãy tìm trên web: thủ đô của nước Úc là thành phố nào? Trả lời một câu và ghi nguồn.", false);
        show(&steps);
        println!("result {result:?}\nanswer: {answer}");
        assert!(steps.iter().any(|s| s["tool"] == "web_search" && s["status"] == "done"));
        assert!(result.is_ok() && !answer.trim().is_empty());
    }
    /// `cargo test --lib agent_current_facts -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn agent_current_facts() {
        let (result, answer, steps) = agent_turn("check giúp tôi ai là tổng thống mỹ hiện tại", false);
        show(&steps);
        println!("result {result:?}
answer: {answer}");
        assert_eq!(steps.first().map(|s| s["tool"].clone()), Some(json!("current_time")), "agent turns start by checking the clock");
        assert!(steps.iter().any(|s| s["tool"] == "web_search"), "a 'current' question must be searched");
        assert!(result.is_ok());
    }
    /// `cargo test --lib agent_terminal -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn agent_terminal() {
        let (result, answer, steps) = agent_turn("Dùng lệnh terminal để cho tôi biết phiên bản PowerShell trên máy (lệnh $PSVersionTable.PSVersion).", true);
        show(&steps);
        println!("result {result:?}\nanswer: {answer}");
        assert!(steps.iter().any(|s| s["tool"] == "run_command" && s["status"] == "done"));
        let (_, _, denied) = agent_turn("Chạy lệnh terminal: Get-Date", false);
        show(&denied);
        assert!(denied.iter().any(|s| s["tool"] == "run_command" && s["status"] == "denied"));
    }
}
#[cfg(test)]
mod chat_live_tests {
    use super::*;
    /// `cargo test --lib chat_greeting_is_short -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn chat_greeting_is_short() {
        let mut outputs = Vec::new();
        for _ in 0..3 {
            let dir = std::env::temp_dir().join(format!("lim-chat-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let runtime = Runtime::new(crate::storage::Store::open(&dir.join("t.sqlite3")).unwrap()).unwrap();
            let session = runtime.store.create_session().unwrap().id;
            let run = Uuid::new_v4().to_string();
            let (_, mut assistant) = runtime.store.begin(&session, &run, "qwen3.5:2b", "chào cậu").unwrap();
            let channel: Channel<Value> = Channel::new(|_| Ok(()));
            let settings = InferenceSettings::default();
            let (mut sequence, mut content, mut thinking, mut metrics) = (0, String::new(), String::new(), None);
            let result = tauri::async_runtime::block_on(generate(&runtime, &session, &run, "qwen3.5:2b", "chào cậu", 4096, &settings, &mut assistant, &channel, &mut sequence, &mut content, &mut thinking, &mut metrics));
            let out = metrics.as_ref().and_then(|m: &crate::storage::ChatMetrics| m.output_tokens).unwrap_or(0);
            println!("result {result:?} · output {out} · steps {} · answer {:?}", assistant.steps.as_ref().map_or(0, Vec::len), content.chars().take(60).collect::<String>());
            assert_eq!(result, Ok(false));
            outputs.push(out);
            drop(runtime);
            let _ = std::fs::remove_dir_all(&dir);
        }
        outputs.sort();
        assert!(outputs[1] < 600, "median output should stay small, got {outputs:?}");
    }
}
