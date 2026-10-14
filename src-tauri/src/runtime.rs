use crate::{
    storage::{InferenceSettings, Message, Session, Snapshot, Store},
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
fn input_budget(context_size: usize, settings: &InferenceSettings) -> usize {
    context_size.saturating_sub(settings.num_predict + settings.system_prompt.len() + 256)
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
    if ![4096, 8192, 16384].contains(&context_size) {
        return Err("Context phải là 4096, 8192 hoặc 16384".into());
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
    let (context_size, settings) = state.store.inference_config()?;
    let maximum = input_budget(context_size, &settings)
        .saturating_sub(32)
        .min(8_000);
    if maximum == 0 {
        return Err(format!("Context {context_size} không còn chỗ cho tin nhắn sau khi dành {} token output và {} byte system prompt. Giảm output/system prompt hoặc tăng context.",settings.num_predict,settings.system_prompt.len()));
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
            text,
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
    text: String,
    context_size: usize,
    settings: InferenceSettings,
    mut assistant: Message,
    channel: Channel<Value>,
    mut sequence: u64,
    mut cancelled: watch::Receiver<bool>,
    _model_guard: OwnedRwLockReadGuard<()>,
) {
    let mut content = String::new();
    let mut thinking = String::new();
    let mut metrics: Option<crate::storage::ChatMetrics> = None;
    let result = if *cancelled.borrow() {
        Err("__cancelled__".into())
    } else {
        tokio::select! {
            _=cancelled.changed()=>Err("__cancelled__".into()),
            result=tokio::time::timeout(Duration::from_secs(300),generate(&runtime,&session,&run,&model,&text,context_size,&settings,&mut assistant,&channel,&mut sequence,&mut content,&mut thinking,&mut metrics))=>result.unwrap_or_else(|_|Err("Lượt chạy vượt quá 5 phút và đã dừng.".into())),
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
        Ok(()) => ("completed", None),
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

fn bounded_history(messages: Vec<Message>, budget: usize) -> Vec<Value> {
    let mut selected = Vec::new();
    let mut remaining = budget;
    for message in messages.into_iter().rev() {
        if message.source == "ui_preview"
            || message.text.is_empty()
            || (message.role == "assistant" && message.status.as_deref() != Some("completed"))
        {
            continue;
        }
        let cost = message.text.len() + 32;
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
) -> Result<(), String> {
    let mut history = Vec::new();
    if !settings.system_prompt.is_empty() {
        history.push(json!({"role":"system","content":settings.system_prompt}));
    }
    let mut history_budget = input_budget(context_size, settings);
    let memory_budget = history_budget.saturating_sub(text.len() + 64).min(4_500);
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
                history_budget = history_budget.saturating_sub(context.len() + 32);
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
    let transcript = bounded_history(runtime.store.messages(session)?, history_budget);
    if !transcript
        .last()
        .is_some_and(|m| m["role"] == "user" && m["content"] == text)
    {
        return Err("Không thể giữ nguyên tin nhắn mới nhất trong context; lượt đã dừng.".into());
    }
    history.extend(transcript);
    let mut options = json!({"num_ctx":context_size,"num_predict":settings.num_predict,"temperature":settings.temperature,"top_p":settings.top_p,"top_k":settings.top_k,"repeat_penalty":settings.repeat_penalty});
    if let Some(seed) = settings.seed {
        options["seed"] = json!(seed);
    }
    let mut request = json!({"model":model,"messages":history,"stream":true,"options":options});
    // Qwen3's thinking can exhaust this small output budget before any answer text.
    if model == "qwen3" || model.starts_with("qwen3:") {
        request["think"] = json!(false);
    }
    let response = runtime
        .client
        .post(format!("{OLLAMA}/api/chat"))
        .json(&request)
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
                    if consume(
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
                        settings.num_predict,
                    )? {
                        return completed(content);
                    }
                }
            }
            Some(Err(_)) => {
                return Err("Kết nối Ollama bị gián đoạn; phần đã nhận được giữ lại.".into())
            }
            None => {
                if !pending.is_empty()
                    && consume(
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
                        settings.num_predict,
                    )?
                {
                    return completed(content);
                }
                return Err("Ollama đóng stream trước tín hiệu hoàn tất.".into());
            }
        }
    }
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
    num_predict: usize,
) -> Result<bool, String> {
    if line.iter().all(u8::is_ascii_whitespace) {
        return Ok(false);
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
        }
    }
    let done = frame.get("done").and_then(Value::as_bool) == Some(true);
    if done && frame.get("done_reason").and_then(Value::as_str) == Some("length") {
        return Err(format!("Model đã đạt giới hạn {num_predict} token đầu ra (thinking + trả lời dùng chung). Phần đã nhận được giữ lại; tăng output trong cấu hình hoặc yêu cầu trả lời ngắn hơn."));
    }
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
    Ok(done)
}
