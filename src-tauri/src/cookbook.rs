use crate::runtime::Runtime;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::{ipc::Channel, State};
use tokio::sync::{watch, OwnedRwLockWriteGuard};
use uuid::Uuid;

const OLLAMA: &str = "http://127.0.0.1:11434";
const CANCELLED: &str = "__cancelled__";
struct Operation {
    id: String,
    cancel: watch::Sender<bool>,
}
pub struct Cookbook {
    active: Mutex<Option<Operation>>,
}
impl Cookbook {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(None),
        }
    }
}
struct OperationGuard {
    cookbook: Arc<Cookbook>,
    id: String,
}
impl Drop for OperationGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.cookbook.active.lock() {
            if active.as_ref().is_some_and(|op| op.id == self.id) {
                *active = None;
            }
        }
    }
}

fn valid_model(model: &str) -> Result<(), String> {
    let valid_segment = |segment: &str| {
        !segment.is_empty()
            && segment.len() <= 80
            && (segment.as_bytes()[0].is_ascii_alphanumeric() || segment.as_bytes()[0] == b'_')
            && segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    let mut parts = model.split(':');
    let name = parts.next().unwrap_or_default();
    let tag = parts.next();
    // Hugging Face GGUF repos are pulled by Ollama as hf.co/<user>/<repo>[:<quant>].
    let hugging_face = name.strip_prefix("hf.co/");
    let segments: Vec<&str> = hugging_face.unwrap_or(name).split('/').collect();
    if model.len() > 200
        || parts.next().is_some()
        || segments.len() > 2
        || (hugging_face.is_some() && segments.len() != 2)
        || !segments.iter().all(|segment| valid_segment(segment))
        || (hugging_face.is_none() && segments.len() == 2 && segments[0].contains('.'))
        || tag.is_some_and(|tag| !valid_segment(tag))
    {
        return Err("Nhập tag Ollama (qwen3:4b, namespace/tên:tag) hoặc repo GGUF Hugging Face dạng hf.co/user/repo:Q4_K_M, tối đa 200 ký tự. Không nhập URL hoặc registry riêng.".into());
    }
    Ok(())
}
fn same_model(entry: &Value, model: &str) -> bool {
    entry["name"]
        .as_str()
        .is_some_and(|name| name.eq_ignore_ascii_case(model))
}
fn send(
    channel: &Channel<Value>,
    id: &str,
    model: &str,
    sequence: &mut u64,
    mut event: Value,
) -> Result<(), String> {
    event["operationId"] = json!(id);
    event["model"] = json!(model);
    event["sequence"] = json!(*sequence);
    *sequence += 1;
    channel
        .send(event)
        .map_err(|_| "Kênh Cookbook đã đóng".into())
}
async fn get_models(client: &reqwest::Client, path: &str) -> Result<Vec<Value>, String> {
    let response = client
        .get(format!("{OLLAMA}{path}"))
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .map_err(|_| "Không kết nối được Ollama tại 127.0.0.1:11434".to_string())?;
    let response = checked(response).await?;
    let value: Value = response
        .json()
        .await
        .map_err(|_| "Ollama trả dữ liệu JSON không hợp lệ")?;
    value["models"]
        .as_array()
        .cloned()
        .ok_or_else(|| "Ollama không trả danh sách models".into())
}
async fn checked(response: reqwest::Response) -> Result<reqwest::Response, String> {
    if response.status().is_success() {
        return Ok(response);
    }
    Err(format!(
        "Ollama trả HTTP {}. Kiểm tra dịch vụ, model và dung lượng trống.",
        response.status()
    ))
}
async fn get_version(client: &reqwest::Client) -> Result<String, String> {
    let response = client
        .get(format!("{OLLAMA}/api/version"))
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .map_err(|_| "Không đọc được phiên bản Ollama".to_string())?;
    let data: Value = checked(response)
        .await?
        .json()
        .await
        .map_err(|_| "Phản hồi phiên bản Ollama không hợp lệ")?;
    data["version"]
        .as_str()
        .filter(|value| !value.is_empty() && value.len() <= 100)
        .map(str::to_string)
        .ok_or_else(|| "Ollama không trả phiên bản hợp lệ".into())
}

#[tauri::command]
pub async fn cookbook_snapshot(runtime: State<'_, Arc<Runtime>>) -> Result<Value, String> {
    let client = runtime.client.clone();
    let hardware = tauri::async_runtime::spawn_blocking(hardware_snapshot);
    let (tags, ps, version, hardware) = tokio::join!(
        get_models(&client, "/api/tags"),
        get_models(&client, "/api/ps"),
        get_version(&client),
        hardware
    );
    let mut errors = Vec::new();
    let available = tags.is_ok();
    let version = version.ok();
    let models=match tags{Ok(models)=>models.into_iter().map(|m|json!({"name":m["name"],"sizeBytes":m["size"],"parameterSize":m["details"]["parameter_size"],"quantizationLevel":m["details"]["quantization_level"],"modifiedAt":m["modified_at"]})).collect::<Vec<_>>(),Err(e)=>{errors.push(e);vec![]}};
    let running=match ps{Ok(models)=>models.into_iter().map(|m|json!({"name":m["name"],"sizeBytes":m["size"],"sizeVramBytes":m["size_vram"],"contextLength":m["context_length"],"expiresAt":m["expires_at"]})).collect::<Vec<_>>(),Err(e)=>{if !errors.contains(&e){errors.push(e)}vec![]}};
    let hardware =
        hardware.unwrap_or_else(|_| unknown_hardware("Không đọc được thông tin phần cứng"));
    let observations=runtime.store.setting("model_observations")?;
    let observations=serde_json::from_str::<Value>(&observations).unwrap_or(Value::Null);
    let measurements=observations.as_object().map(|observations|observations.values().cloned().collect::<Vec<_>>()).unwrap_or_default();
    Ok(
        json!({"ollamaAvailable":available,"version":version,"measurements":measurements,"error":if errors.is_empty(){None}else{Some(errors.join("; "))},"hardware":hardware,"models":models,"running":running}),
    )
}

#[tauri::command]
pub fn start_model_setup(
    runtime: State<'_, Arc<Runtime>>,
    cookbook: State<'_, Arc<Cookbook>>,
    model: String,
    context_size: usize,
    download_only: Option<bool>,
    on_event: Channel<Value>,
) -> Result<String, String> {
    let model = model.trim();
    let model = if model
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("library/"))
    {
        &model[8..]
    } else {
        model
    };
    let model = model.to_string();
    valid_model(&model)?;
    let model = if model.contains(':') {
        model
    } else {
        format!("{model}:latest")
    };
    valid_model(&model)?;
    let download_only = download_only.unwrap_or(false);
    if !crate::storage::CONTEXT_SIZES.contains(&context_size) {
        return Err("Context phải là 4K, 8K, 16K, 32K, 64K hoặc 128K token".into());
    }
    let model_guard = Arc::clone(&runtime.model_gate)
        .try_write_owned()
        .map_err(|_| {
            "Đang có hội thoại hoặc thao tác model. Chờ hoàn tất trước khi chuẩn bị model."
                .to_string()
        })?;
    let id = Uuid::new_v4().to_string();
    let (cancel, receiver) = watch::channel(false);
    {
        let mut active = cookbook
            .active
            .lock()
            .map_err(|_| "Cookbook không khả dụng")?;
        if active.is_some() {
            return Err("Cookbook đang có một thao tác chuẩn bị model".into());
        }
        *active = Some(Operation {
            id: id.clone(),
            cancel,
        });
    }
    let operation_guard = OperationGuard {
        cookbook: Arc::clone(cookbook.inner()),
        id: id.clone(),
    };
    let mut sequence = 0;
    send(
        &on_event,
        &id,
        &model,
        &mut sequence,
        json!({"type":"started","contextSize":context_size,"downloadOnly":download_only,"status":"Đang kiểm tra model trên máy"}),
    )?;
    let runtime = Arc::clone(runtime.inner());
    let returned = id.clone();
    tauri::async_runtime::spawn(async move {
        setup_operation(
            runtime,
            id,
            model,
            context_size,
            download_only,
            on_event,
            sequence,
            receiver,
            operation_guard,
            model_guard,
        )
        .await;
    });
    Ok(returned)
}
#[tauri::command]
pub fn cancel_model_setup(
    cookbook: State<'_, Arc<Cookbook>>,
    operation_id: String,
) -> Result<bool, String> {
    let active = cookbook
        .active
        .lock()
        .map_err(|_| "Cookbook không khả dụng")?;
    Ok(active
        .as_ref()
        .filter(|op| op.id == operation_id)
        .is_some_and(|op| op.cancel.send(true).is_ok()))
}
async fn setup_operation(
    runtime: Arc<Runtime>,
    id: String,
    model: String,
    context_size: usize,
    download_only: bool,
    channel: Channel<Value>,
    mut sequence: u64,
    mut cancelled: watch::Receiver<bool>,
    operation_guard: OperationGuard,
    model_guard: OwnedRwLockWriteGuard<()>,
) {
    let result = if *cancelled.borrow() {
        Err(CANCELLED.into())
    } else {
        tokio::select! {biased;
            _=cancelled.changed()=>Err(CANCELLED.into()),
            result=tokio::time::timeout(Duration::from_secs(7200),prepare_model(&runtime,&id,&model,context_size,download_only,&channel,&mut sequence))=>result.unwrap_or_else(|_|Err("Chuẩn bị model vượt quá 2 giờ; đã ngắt chờ tại Lim. Ollama có thể vẫn xử lý; kiểm tra lại trạng thái.".into())),
        }
    };
    let event = match result {
        Ok(()) if download_only => {
            json!({"type":"downloaded","status":"Model đã được xác nhận trong danh sách đã cài; chưa nạp và chưa đổi cấu hình hội thoại"})
        }
        Ok(()) => {
            json!({"type":"ready","contextSize":context_size,"status":"Model đã có trên máy, đã nạp và cấu hình hội thoại đã lưu"})
        }
        Err(e) if e == CANCELLED => {
            json!({"type":"cancelled","status":"Đã ngắt thao tác tại Lim. Ollama có thể vẫn tải hoặc nạp; dữ liệu tải dở được giữ để tiếp tục."})
        }
        Err(e) => {
            json!({"type":"failed","error":e,"status":"Chưa xác nhận model sẵn sàng. Dữ liệu tải dở được giữ lại."})
        }
    };
    drop(model_guard);
    drop(operation_guard);
    let _ = send(&channel, &id, &model, &mut sequence, event);
}
async fn prepare_model(
    runtime: &Runtime,
    id: &str,
    model: &str,
    context_size: usize,
    download_only: bool,
    channel: &Channel<Value>,
    sequence: &mut u64,
) -> Result<(), String> {
    let installed = get_models(&runtime.client, "/api/tags").await?;
    if !installed.iter().any(|m| same_model(m, model)) {
        pull(&runtime.client, id, model, channel, sequence).await?;
    }
    let installed = get_models(&runtime.client, "/api/tags").await?;
    if !installed.iter().any(|m| same_model(m, model)) {
        return Err("Ollama chưa xác nhận model trong danh sách đã cài".into());
    }
    if download_only {
        return Ok(());
    }
    send(
        channel,
        id,
        model,
        sequence,
        json!({"type":"loading","status":"Đang nạp model vào bộ nhớ","contextSize":context_size}),
    )?;
    let response=runtime.client.post(format!("{OLLAMA}/api/chat")).timeout(Duration::from_secs(180)).json(&json!({"model":model,"messages":[],"stream":false,"keep_alive":"5m","options":{"num_ctx":context_size}})).send().await.map_err(|_|"Không nạp được model trong 3 phút. Ollama có thể vẫn nạp; làm mới để kiểm tra.".to_string())?;
    let value: Value = checked(response)
        .await?
        .json()
        .await
        .map_err(|_| "Phản hồi nạp model không hợp lệ")?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(format!(
            "Ollama: {}",
            error.chars().take(300).collect::<String>()
        ));
    }
    if value["done"].as_bool() != Some(true) {
        return Err("Ollama chưa báo hoàn tất nạp model".into());
    }
    let running = get_models(&runtime.client, "/api/ps").await?;
    let loaded = running
        .iter()
        .find(|m| same_model(m, model))
        .ok_or("Model chưa xuất hiện trong danh sách đang chạy của Ollama")?;
    if loaded["context_length"]
        .as_u64()
        .is_some_and(|actual| actual != context_size as u64)
    {
        return Err("Ollama đang nạp model với context khác yêu cầu. Làm mới và thử lại.".into());
    }
    runtime
        .store
        .set_model_config(loaded["name"].as_str().unwrap_or(model), context_size)
}
async fn pull(
    client: &reqwest::Client,
    id: &str,
    model: &str,
    channel: &Channel<Value>,
    sequence: &mut u64,
) -> Result<(), String> {
    let response = client
        .post(format!("{OLLAMA}/api/pull"))
        .json(&json!({"model":model,"stream":true}))
        .send()
        .await
        .map_err(|_| "Không bắt đầu tải được model từ Ollama")?;
    let mut stream = checked(response).await?.bytes_stream();
    let mut pending = Vec::new();
    loop {
        let next = tokio::time::timeout(Duration::from_secs(120), stream.next())
            .await
            .map_err(|_| {
                "Ollama không gửi tiến độ trong 2 phút; đã ngắt chờ tại Lim. Làm mới để kiểm tra."
                    .to_string()
            })?;
        match next {
            Some(Ok(bytes)) => {
                pending.extend_from_slice(&bytes);
                if pending.len() > 1_000_000 {
                    return Err("Khung tiến độ Ollama quá lớn".into());
                }
                while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=end).collect();
                    if pull_frame(&line, id, model, channel, sequence)? {
                        return Ok(());
                    }
                }
            }
            Some(Err(_)) => {
                return Err(
                    "Kết nối tải model bị gián đoạn. Có thể thử lại để tiếp tục phần đã tải."
                        .into(),
                )
            }
            None => {
                if !pending.is_empty() && pull_frame(&pending, id, model, channel, sequence)? {
                    return Ok(());
                }
                return Err("Stream tải kết thúc trước trạng thái success".into());
            }
        }
    }
}
fn pull_frame(
    line: &[u8],
    id: &str,
    model: &str,
    channel: &Channel<Value>,
    sequence: &mut u64,
) -> Result<bool, String> {
    if line.iter().all(u8::is_ascii_whitespace) {
        return Ok(false);
    }
    let frame: Value =
        serde_json::from_slice(line).map_err(|_| "Tiến độ Ollama không phải JSON hợp lệ")?;
    if let Some(error) = frame.get("error").and_then(Value::as_str) {
        return Err(format!(
            "Ollama: {}",
            error.chars().take(300).collect::<String>()
        ));
    }
    let status = frame
        .get("status")
        .and_then(Value::as_str)
        .ok_or("Tiến độ Ollama thiếu status")?;
    let mut event = json!({"type":"progress","status":status});
    for key in ["digest", "completed", "total"] {
        if let Some(value) = frame.get(key) {
            event[key] = value.clone();
        }
    }
    send(channel, id, model, sequence, event)?;
    Ok(status == "success")
}

#[tauri::command]
pub async fn unload_model(
    runtime: State<'_, Arc<Runtime>>,
    cookbook: State<'_, Arc<Cookbook>>,
    model: String,
) -> Result<Value, String> {
    valid_model(&model)?;
    let _model_guard = Arc::clone(&runtime.model_gate)
        .try_write_owned()
        .map_err(|_| "Đang có hội thoại hoặc thao tác model; chưa thể giải phóng model.")?;
    if cookbook
        .active
        .lock()
        .map_err(|_| "Cookbook không khả dụng")?
        .is_some()
    {
        return Err("Cookbook đang chuẩn bị model".into());
    }
    let running = get_models(&runtime.client, "/api/ps").await?;
    if !running
        .iter()
        .any(|m| m["name"].as_str() == Some(model.as_str()))
    {
        return Ok(
            json!({"unloaded":true,"model":model,"status":"Model không ở trong bộ nhớ; tệp đã cài vẫn được giữ"}),
        );
    }
    let response = runtime
        .client
        .post(format!("{OLLAMA}/api/chat"))
        .timeout(Duration::from_secs(60))
        .json(&json!({"model":model,"messages":[],"stream":false,"keep_alive":0}))
        .send()
        .await
        .map_err(|_| "Không giải phóng được model. Làm mới để kiểm tra trạng thái thực tế.")?;
    let value: Value = checked(response)
        .await?
        .json()
        .await
        .map_err(|_| "Ollama trả phản hồi giải phóng không hợp lệ")?;
    if value["done"].as_bool() != Some(true) {
        return Err("Ollama chưa xác nhận giải phóng model".into());
    }
    let running = get_models(&runtime.client, "/api/ps").await?;
    if running
        .iter()
        .any(|m| m["name"].as_str() == Some(model.as_str()))
    {
        return Err(
            "Model vẫn còn trong bộ nhớ Ollama. Có thể ứng dụng khác đang dùng model.".into(),
        );
    }
    Ok(
        json!({"unloaded":true,"model":model,"status":"Đã giải phóng model khỏi bộ nhớ; tệp đã cài vẫn được giữ"}),
    )
}

fn unknown_hardware(note: &str) -> Value {
    json!({"cpu":null,"ramTotalBytes":null,"ramAvailableBytes":null,"gpuName":null,"gpuTotalBytes":null,"gpuFreeBytes":null,"diskAvailableBytes":null,"modelDirectory":null,"modelDirectorySource":null,"notes":[note]})
}
fn command_output(program: &str, args: &[&str], timeout: Duration) -> Result<Vec<u8>, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| format!("Không chạy được {program}"))?;
    let started = Instant::now();
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            let output = child.wait_with_output().map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err(format!("{program} không trả dữ liệu phần cứng"));
            }
            if output.stdout.len() > 64_000 {
                return Err("Phản hồi phần cứng quá lớn".into());
            }
            return Ok(output.stdout);
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{program} vượt thời gian đọc phần cứng"));
        }
        thread::sleep(Duration::from_millis(50));
    }
}
fn hardware_snapshot() -> Value {
    #[cfg(not(windows))]
    {
        return unknown_hardware(
            "Tự đọc phần cứng hiện hỗ trợ Windows; các giá trị khác chưa xác định",
        );
    }
    #[cfg(windows)]
    {
        // Fixed read-only script. No user input or model names are inserted into shell code.
        let script = r#"$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false); $os=Get-CimInstance Win32_OperatingSystem; $cpu=(Get-CimInstance Win32_Processor | Select-Object -ExpandProperty Name) -join ', '; $modelDir=$env:OLLAMA_MODELS; $source='configured'; if([string]::IsNullOrWhiteSpace($modelDir)){$modelDir=Join-Path $env:USERPROFILE '.ollama\models';$source='default-estimate'}; $disk=$null; try{$full=[System.IO.Path]::GetFullPath($modelDir);$drive=[System.IO.DriveInfo]::new([System.IO.Path]::GetPathRoot($full));$disk=$drive.AvailableFreeSpace}catch{}; $gpu=$null;try{$gpu=(Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name) -join ', '}catch{}; [ordered]@{cpu=$cpu;ramTotalBytes=[uint64]$os.TotalVisibleMemorySize*1024;ramAvailableBytes=[uint64]$os.FreePhysicalMemory*1024;gpuName=$gpu;diskAvailableBytes=$disk;modelDirectory=$modelDir;modelDirectorySource=$source}|ConvertTo-Json -Compress"#;
        let output = command_output(
            "powershell.exe",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ],
            Duration::from_secs(10),
        );
        let mut hardware = match output {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(value) if value.is_object() => value,
                _ => unknown_hardware("Windows không trả JSON phần cứng hợp lệ"),
            },
            Err(error) => unknown_hardware(&error),
        };
        hardware["gpuTotalBytes"] = Value::Null;
        hardware["gpuFreeBytes"] = Value::Null;
        let mut notes: Vec<String> = hardware["notes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect();
        if hardware["modelDirectorySource"] == "default-estimate" {
            notes.push("Đường dẫn model và dung lượng đĩa theo mặc định của người dùng hiện tại; Ollama chạy bằng tài khoản/cấu hình khác có thể dùng thư mục khác.".into());
        }
        let gpu = command_output(
            "nvidia-smi",
            &[
                "--query-gpu=name,memory.total,memory.free",
                "--format=csv,noheader,nounits",
            ],
            Duration::from_secs(5),
        );
        if let Ok(bytes) = gpu {
            if let Some(line) = String::from_utf8_lossy(&bytes).lines().next() {
                let parts: Vec<&str> = line.split(',').map(str::trim).collect();
                if parts.len() == 3 {
                    hardware["gpuName"] = json!(parts[0]);
                    hardware["gpuTotalBytes"] = parts[1]
                        .parse::<u64>()
                        .ok()
                        .and_then(|n| n.checked_mul(1024 * 1024))
                        .map_or(Value::Null, |n| json!(n));
                    hardware["gpuFreeBytes"] = parts[2]
                        .parse::<u64>()
                        .ok()
                        .and_then(|n| n.checked_mul(1024 * 1024))
                        .map_or(Value::Null, |n| json!(n));
                }
            }
        }
        if hardware["gpuTotalBytes"].is_null() || hardware["gpuFreeBytes"].is_null() {
            notes.push("Chưa đọc được tổng VRAM/VRAM trống từ NVIDIA; không dùng AdapterRAM của Windows để suy đoán VRAM.".into());
        }
        hardware["notes"] = json!(notes);
        hardware
    }
}

#[cfg(test)]
mod model_name_tests {
    use super::valid_model;
    #[test]
    fn accepts_ollama_and_hugging_face_names() {
        for ok in ["qwen3:4b", "library/qwen3", "user/model:tag", "hf.co/unsloth/Qwen3-4B-GGUF:Q4_K_M", "hf.co/bartowski/Llama-3.2-1B-Instruct-GGUF"] {
            assert!(valid_model(ok).is_ok(), "{ok}");
        }
        for bad in ["hf.co/onlyrepo", "hf.co/a/b/c", "example.com/user/model", "https://huggingface.co/a/b", "a:b:c", "../x"] {
            assert!(valid_model(bad).is_err(), "{bad}");
        }
    }
}
