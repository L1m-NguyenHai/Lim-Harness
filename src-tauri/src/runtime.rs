use crate::{
    storage::{estimate_tokens, InferenceSettings, Message, Session, Snapshot, Store},
    vault,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
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
pub fn start_turn(
    state: State<'_, Arc<Runtime>>,
    session_id: String,
    text: String,
    model: String,
    on_event: Channel<Value>,
) -> Result<String, String> {
    let model_guard = Arc::clone(&state.model_gate)
        .try_read_owned()
        .map_err(|_| {
            "Cookbook đang chuẩn bị model; chờ hoàn tất hoặc hủy thao tác trước khi gửi."
        })?;
    let text = text.trim().to_string();
    let model = model.trim().to_string();
    let (context_size, settings) = state.store.session_config(&session_id)?;
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
        let messages = state.store.begin(&session_id, &run_id, &model, &text)?;
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
        )
        .await;
    });
    Ok(returned)
}
/// Continues an answer that stopped at the output limit in a new run with a fresh budget.
#[tauri::command]
pub fn continue_turn(
    state: State<'_, Arc<Runtime>>,
    session_id: String,
    message_id: String,
    model: String,
    on_event: Channel<Value>,
) -> Result<String, String> {
    let model_guard = Arc::clone(&state.model_gate)
        .try_read_owned()
        .map_err(|_| "Cookbook đang chuẩn bị model; chờ hoàn tất hoặc hủy thao tác trước khi gửi.")?;
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
        let assistant = state.store.begin_continue(&session_id, &run_id, &message_id, &model)?;
        active.insert(session_id.clone(), ActiveRun { id: run_id.clone(), cancel });
        assistant
    };
    let mut sequence = 0;
    if let Err(error) = emit(&on_event, &run_id, &session_id, &mut sequence, json!({"type":"started","message":assistant})) {
        let persisted = state.store.finish(&session_id, &assistant, "truncated", None);
        state.active.lock().map_err(|_| "Runtime không khả dụng")?.remove(&session_id);
        persisted?;
        return Err(error);
    }
    let runtime = Arc::clone(state.inner());
    let returned = run_id.clone();
    tauri::async_runtime::spawn(async move {
        run_turn(runtime, session_id, run_id, model, None, context_size, settings, assistant, on_event, sequence, receiver, model_guard).await;
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

async fn run_turn(
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
    let persisted = runtime
        .store
        .finish(&session, &assistant, status, error.as_deref());
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
    let messages: Vec<Message> = messages.into_iter().filter(|m| m.source != "ui_preview").collect();
    let usable = |m: &Message| m.role == "assistant" && !m.text.trim().is_empty() && matches!(m.status.as_deref(), Some("completed" | "truncated"));
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
    let system = system_with_summary(&settings.system_prompt, summary.as_deref());
    let mut history = Vec::new();
    if !system.is_empty() {
        history.push(json!({"role":"system","content":system}));
    }
    let mut history_budget = input_budget(context_size, settings)
        .saturating_sub(summary.as_deref().map_or(0, |s| estimate_tokens(&summary_block(s))));
    // Notes get at most 4.500 bytes and never the room the new message needs.
    let memory_budget = history_budget.saturating_sub(estimate_tokens(text) + 16).saturating_mul(3).min(4_500);
    let path = runtime.store.setting("vault")?;
    if !path.is_empty() {
        let query = text.to_string();
        let result =
            tauri::async_runtime::spawn_blocking(move || vault::search_vault(&path, &query, 4))
                .await
                .map_err(|_| "Không đọc được bộ nhớ vault")??;
        let mut context_truncated = result.truncated;
        if !result.hits.is_empty() {
            // Keep complete source JSON; skip excess sources instead of slicing its framing.
            let mut hits = result.hits.clone();
            while !hits.is_empty() && vault::memory_context(&hits).len() > memory_budget {
                hits.pop();
                context_truncated = true;
            }
            if !hits.is_empty() {
                let context = vault::memory_context(&hits);
                history_budget = history_budget.saturating_sub(estimate_tokens(&context));
                history.push(json!({"role":"user","content":context}));
                assistant.sources = Some(hits.clone());
                runtime.store.save_sources(&assistant.id, &hits)?;
            }
        }
        emit(
            channel,
            run,
            session,
            sequence,
            json!({"type":"memory_sources","sources":assistant.sources.clone().unwrap_or_default(),"truncated":context_truncated,"skippedFiles":result.skipped_files,"scannedFiles":result.scanned_files}),
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
    let mut options = json!({"num_ctx":context_size,"temperature":settings.temperature,"top_p":settings.top_p,"top_k":settings.top_k,"repeat_penalty":settings.repeat_penalty});
    if let Some(seed) = settings.seed {
        options["seed"] = json!(seed);
    }
    // One output limit shared by thinking and answer; Ollama counts it exactly.
    options["num_predict"] = json!(settings.num_predict);
    let request = json!({"model":model,"messages":history,"stream":true,"options":options});
    let mut counts = Counts { thinking: 0, answer: 0, budget: settings.num_predict as u64 };
    let _ = emit(channel, run, session, sequence, counts.usage());
    let step = stream_chat(runtime, &request, assistant, channel, run, session, sequence, content, thinking, metrics, model, context_size, &mut counts).await?;
    let _ = emit(channel, run, session, sequence, counts.usage());
    outcome(step, content)
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
                format!("Ghi chú suy nghĩ của trợ lý cho câu hỏi cuối:
{draft}")
            };
            system = if system.is_empty() { note } else { format!("{system}

{note}") };
        }
        0
    };
    let mut history = Vec::new();
    if !system.is_empty() {
        history.push(json!({"role":"system","content":system}));
    }
    let mut carried = estimate_tokens(&system) + carried_thinking + estimate_tokens(content) + 256;
    if let Some(hits) = assistant.sources.as_ref().filter(|hits| !hits.is_empty()) {
        let context = vault::memory_context(hits);
        carried += estimate_tokens(&context);
        history.push(json!({"role":"user","content":context}));
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
    let mut options = json!({"num_ctx":context_size,"num_predict":num_predict,"temperature":settings.temperature,"top_p":settings.top_p,"top_k":settings.top_k,"repeat_penalty":settings.repeat_penalty});
    if let Some(seed) = settings.seed {
        options["seed"] = json!(seed);
    }
    let (path, request, mut split) = if resume_thinking {
        let mut prompt = String::new();
        for message in &history {
            prompt.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", message["role"].as_str().unwrap_or("user"), message["content"].as_str().unwrap_or_default()));
        }
        prompt.push_str("<|im_start|>assistant\n<think>\n");
        prompt.push_str(thinking);
        let answering = !content.trim().is_empty();
        if answering {
            prompt.push_str("\n</think>\n\n");
            prompt.push_str(content);
        }
        ("/api/generate", json!({"model":model,"prompt":prompt,"raw":true,"stream":true,"options":options}), Some(ThinkSplit::new(!answering)))
    } else {
        if !content.trim().is_empty() {
            history.push(json!({"role":"assistant","content":content.clone()}));
        }
        ("/api/chat", json!({"model":model,"messages":history,"stream":true,"think":false,"options":options}), None)
    };
    let previous = assistant.metrics.clone();
    // The answer's limit grows by this run's limit: 1.047 → 2.094 → 3.141 …
    let granted_before = previous.as_ref().and_then(|m| m.output_limit.or(m.output_tokens)).unwrap_or(settings.num_predict as u64);
    let mut counts = Counts {
        thinking: previous.as_ref().and_then(|m| m.thinking_tokens).unwrap_or(0),
        answer: previous.as_ref().and_then(|m| m.answer_tokens).unwrap_or(0),
        budget: granted_before + num_predict as u64,
    };
    let _ = emit(channel, run, session, sequence, counts.usage());
    let step = stream_from(runtime, path, &request, split.as_mut(), assistant, channel, run, session, sequence, content, thinking, metrics, model, context_size, &mut counts).await?;
    // Report the whole answer, not only this continuation request.
    if let (Some(m), Some(before)) = (metrics.as_mut(), previous.and_then(|p| p.output_tokens)) {
        m.output_tokens = Some(m.output_tokens.unwrap_or(0) + before);
        if let Ok(raw) = serde_json::to_value(&*m) {
            let _ = emit(channel, run, session, sequence, json!({"type":"metrics","metrics":raw}));
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
        Self { thinking, pending: String::new(), trim_answer: false }
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
        let keep = (1..TAG.len()).rev().find(|k| self.pending.ends_with(&TAG[..*k])).unwrap_or(0);
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
    stream_from(runtime, "/api/chat", request, None, assistant, channel, run, session, sequence, content, thinking, metrics, model, context_size, counts).await
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
                    let line = match split.as_deref_mut() { Some(split) => raw_frame(&line, split)?, None => line };
                    match consume(&line, runtime, assistant, channel, run, session, sequence, content, thinking, metrics, model, context_size, counts)? {
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
                    let pending = match split.as_deref_mut() { Some(split) => raw_frame(&pending, split)?, None => pending };
                    match consume(&pending, runtime, assistant, channel, run, session, sequence, content, thinking, metrics, model, context_size, counts)? {
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
    let mut frame: Value = serde_json::from_slice(line).map_err(|_| "Ollama trả khung JSON không hợp lệ")?;
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
    modelfile.lines().any(|line| line.trim().to_lowercase().starts_with("renderer qwen3"))
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
    if frame["message"]["tool_calls"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
    {
        return Err(
            "Model yêu cầu tool call; executor chưa được bật trong lõi hội thoại hiện tại.".into(),
        );
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
    Ok(if length { Step::Length } else if done { Step::Done } else { Step::Continue })
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
        assert_eq!((format!("{t1}{t2}"), a), ("chưa xong </".to_string(), String::new()));
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
        let runtime = Runtime::new(crate::storage::Store::open(&dir.join("t.sqlite3")).unwrap()).unwrap();
        let session = runtime.store.create_session().unwrap().id;
        let model = "qwen3.5:2b";
        let channel: Channel<Value> = Channel::new(|_| Ok(()));
        let settings = InferenceSettings { num_predict: 48, ..Default::default() };
        // A question the 2B model can finish; long essays make qwen3.5:2b think for 3.500+ tokens.
        let prompt = "Thủ đô của Pháp là gì? Trả lời một câu.";
        // 1) A tiny output limit stops the answer: status must be "truncated", not failed.
        let run = Uuid::new_v4().to_string();
        let (_, mut assistant) = runtime.store.begin(&session, &run, model, prompt).unwrap();
        let (mut sequence, mut content, mut thinking, mut metrics) = (0, String::new(), String::new(), None);
        let first = tauri::async_runtime::block_on(generate(&runtime, &session, &run, model, prompt, 4096, &settings, &mut assistant, &channel, &mut sequence, &mut content, &mut thinking, &mut metrics));
        println!("first: {first:?} | thinking {} chars | answer {:?}", thinking.len(), content.chars().take(120).collect::<String>());
        assert_eq!(first, Ok(true));
        assistant.text = content.clone();
        assistant.thinking = Some(thinking.clone()).filter(|t| !t.is_empty());
        assistant.metrics = metrics.clone();
        runtime.store.finish(&session, &assistant, "truncated", None).unwrap();
        // 2) Continue with a fresh budget until the answer finishes (at most 15 rounds).
        let mut finished = false;
        for round in 1..=15 {
            let run = Uuid::new_v4().to_string();
            let mut assistant = runtime.store.begin_continue(&session, &run, &assistant.id, model).unwrap();
            let before = assistant.text.clone();
            let (mut content, mut thinking, mut metrics) = (assistant.text.clone(), assistant.thinking.clone().unwrap_or_default(), assistant.metrics.clone());
            let more = InferenceSettings { num_predict: 400, ..Default::default() };
            let result = tauri::async_runtime::block_on(continue_generate(&runtime, &session, &run, model, 4096, &more, &mut assistant, &channel, &mut sequence, &mut content, &mut thinking, &mut metrics));
            println!("round {round}: {result:?} | +{} chars | tail {:?}", content.len() - before.len(), content.chars().rev().take(80).collect::<String>().chars().rev().collect::<String>());
            assert!(content.starts_with(&before), "continuation must keep the earlier text");
            let thinking_before = assistant.thinking.clone().unwrap_or_default();
            if round == 1 && before.is_empty() {
                assert!(thinking.len() > thinking_before.len(), "Qwen continuation should resume the reasoning");
                println!("    thinking resumed: +{} chars, seam {:?}", thinking.len() - thinking_before.len(), format!("{}⟦+⟧{}", thinking_before.chars().rev().take(30).collect::<Vec<_>>().into_iter().rev().collect::<String>(), thinking[thinking_before.len()..].chars().take(40).collect::<String>()));
            }
            let seam: String = before.chars().rev().take(40).collect::<Vec<_>>().into_iter().rev().chain(["⟦+⟧".chars().collect::<Vec<_>>(), content[before.len()..].chars().take(50).collect()].concat()).collect();
            println!("    seam: {seam:?}");
            let limit = metrics.as_ref().and_then(|m| m.output_limit);
            println!("    output limit {limit:?} · output tokens {:?}", metrics.as_ref().and_then(|m| m.output_tokens));
            if round == 1 {
                assert_eq!(limit, Some(48 + 400), "each continue adds its run's limit");
            }
            let truncated = result.clone().unwrap();
            assistant.text = content;
            assistant.thinking = Some(thinking).filter(|t| !t.is_empty());
            assistant.metrics = metrics;
            runtime.store.finish(&session, &assistant, if truncated { "truncated" } else { "completed" }, None).unwrap();
            if !truncated { finished = true; break; }
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
        Message { id: Uuid::new_v4().to_string(), role: role.into(), text: text.into(), source: if role == "user" { "user".into() } else { "model".into() }, run_id: None, status: Some(status.into()), sources: None, model: None, thinking: None, metrics: None, error: None }
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
        let roles: Vec<String> = bounded_history(history, 10_000).iter().map(|m| format!("{}:{}", m["role"].as_str().unwrap(), m["content"].as_str().unwrap())).collect();
        assert_eq!(roles, ["user:viết về nước Pháp", "assistant:Nước Pháp là…", "user:chào"]);
        // A cut answer that already has text stays, with its question.
        let partial = vec![msg("user", "kể chuyện", "completed"), msg("assistant", "Ngày xưa", "truncated"), msg("user", "tiếp đi", "completed")];
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
