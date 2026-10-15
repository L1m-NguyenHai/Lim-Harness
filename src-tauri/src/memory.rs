//! Lim's Markdown memory store. Current content is edited in Obsidian; metadata
//! carries source evidence and immutable previous versions in the same file.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

const MARKER: &str = ".lim-memory.json";
const MAX_FILE: u64 = 256 * 1024;
const MAX_ENTRIES: usize = 5000;
const MAX_FILES: usize = 1000;
const MAX_BYTES: u64 = 8 * 1024 * 1024;
// ponytail: one process-wide writer; per-vault locks only if concurrent vaults become necessary.
static WRITER: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemorySource {
    pub session_id: String,
    pub message_id: String,
    pub quote: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryHistory {
    pub updated_at: String,
    pub title: String,
    pub category: String,
    pub status: String,
    pub pinned: bool,
    pub content: String,
    pub sources: Vec<MemorySource>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryRecord {
    pub schema_version: u32,
    pub id: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub revision: String,
    pub title: String,
    pub category: String,
    pub status: String,
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub sources: Vec<MemorySource>,
    #[serde(default)]
    pub history: Vec<MemoryHistory>,
    #[serde(default)]
    pub operation_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryList {
    pub records: Vec<MemoryRecord>,
    pub diagnostics: Vec<String>,
    pub truncated: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryWriteResult {
    pub record: MemoryRecord,
    pub diagnostics: Vec<String>,
}

pub fn normalize(value: &str) -> String {
    value
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .map(|c| if c == 'đ' { 'd' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn linked(meta: &Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
fn safe_existing(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Đường dẫn memory nằm ngoài vault.")?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err("Đường dẫn memory không hợp lệ.".into());
        }
        current.push(component);
        let meta =
            fs::symlink_metadata(&current).map_err(|e| format!("Không đọc được memory: {e}"))?;
        if linked(&meta) {
            return Err("Memory không được dùng symlink hoặc junction.".into());
        }
    }
    let canonical = fs::canonicalize(path).map_err(|e| e.to_string())?;
    if !canonical.starts_with(root) {
        return Err("Memory nằm ngoài vault.".into());
    }
    Ok(canonical)
}
fn vault(path: &str) -> Result<PathBuf, String> {
    let root = Path::new(path);
    if !root.is_absolute() {
        return Err("Vault cần đường dẫn tuyệt đối.".into());
    }
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    if !root.is_dir() {
        return Err("Vault phải là thư mục.".into());
    }
    Ok(root)
}
fn read_text(root: &Path, path: &Path) -> Result<String, String> {
    let path = safe_existing(root, path)?;
    let file = File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Memory phải là tệp.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE {
        return Err("Memory vượt giới hạn 256 KiB.".into());
    }
    String::from_utf8(bytes).map_err(|_| "Memory không phải UTF-8.".into())
}
fn store(root: &Path) -> Result<PathBuf, String> {
    let directory = safe_existing(root, &root.join("LimMemory"))?;
    let marker: serde_json::Value =
        serde_json::from_str(&read_text(root, &directory.join(MARKER))?)
            .map_err(|_| "Không nhận diện được định dạng LimMemory.")?;
    if marker != serde_json::json!({"schemaVersion":1,"owner":"Lim Harness"}) {
        return Err("Định dạng LimMemory chưa được hỗ trợ.".into());
    }
    let facts = safe_existing(root, &directory.join("facts"))?;
    if !facts.is_dir() {
        return Err("LimMemory/facts phải là thư mục.".into());
    }
    Ok(directory)
}
pub fn initialize(path: &str) -> Result<MemoryList, String> {
    let _lock = WRITER.lock().map_err(|_| "Memory writer không sẵn sàng.")?;
    let root = vault(path)?;
    let directory = root.join("LimMemory");
    match fs::symlink_metadata(&directory) {
        Ok(_) => {
            store(&root)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&directory).map_err(|e| e.to_string())?;
            fs::create_dir(directory.join("facts")).map_err(|e| e.to_string())?;
            atomic_write(
                &root,
                &directory.join(MARKER),
                "{\"schemaVersion\":1,\"owner\":\"Lim Harness\"}",
                None,
            )?;
        }
        Err(e) => return Err(e.to_string()),
    }
    let mut result = scan(&root)?;
    if let Err(e) = rebuild_index(&root, &result.records) {
        result.diagnostics.push(e);
    }
    Ok(result)
}
fn validate(record: &MemoryRecord) -> Result<(), String> {
    if record.schema_version != 1 || uuid::Uuid::parse_str(&record.id).is_err() {
        return Err("Schema hoặc ID memory không hợp lệ.".into());
    }
    if !["active", "retracted"].contains(&record.status.as_str()) {
        return Err("Trạng thái memory không hợp lệ.".into());
    }
    if record.title.trim().is_empty()
        || record.title.chars().count() > 160
        || record.content.trim().is_empty()
        || record.content.chars().count() > 12000
        || record.category.trim().is_empty()
        || record.category.chars().count() > 80
    {
        return Err("Tiêu đề, loại hoặc nội dung memory không hợp lệ.".into());
    }
    if record
        .content
        .lines()
        .any(|line| line.starts_with("## Nguồn") || line.starts_with("## Lịch sử"))
    {
        return Err("Nội dung không được chứa tiêu đề phần Nguồn/Lịch sử dành riêng.".into());
    }
    for source in &record.sources {
        if source.session_id.is_empty()
            || source.message_id.is_empty()
            || source.quote.trim().is_empty()
            || source.quote.chars().count() > 2000
        {
            return Err("Nguồn memory không hợp lệ.".into());
        }
    }
    for date in [&record.created_at, &record.updated_at] {
        chrono::DateTime::parse_from_rfc3339(date).map_err(|_| "Ngày memory không hợp lệ.")?;
    }
    for history in &record.history {
        chrono::DateTime::parse_from_rfc3339(&history.updated_at)
            .map_err(|_| "Ngày lịch sử memory không hợp lệ.")?;
        if !["active", "retracted"].contains(&history.status.as_str()) {
            return Err("Trạng thái lịch sử memory không hợp lệ.".into());
        }
    }
    Ok(())
}
fn parse(text: &str) -> Result<MemoryRecord, String> {
    let normalized = text.replace("\r\n", "\n");
    let rest = normalized
        .strip_prefix("---\n")
        .ok_or("Memory thiếu frontmatter.")?;
    let (yaml, body) = rest
        .split_once("\n---\n")
        .ok_or("Memory thiếu kết thúc frontmatter.")?;
    let mut record: MemoryRecord =
        serde_yaml::from_str(yaml).map_err(|e| format!("Frontmatter memory không hợp lệ: {e}"))?;
    record.id = uuid::Uuid::parse_str(&record.id)
        .map_err(|_| "ID memory không hợp lệ.")?
        .to_string();
    let body = body
        .strip_prefix("\n## Nội dung\n\n")
        .or_else(|| body.strip_prefix("## Nội dung\n\n"))
        .ok_or("Memory thiếu phần Nội dung.")?;
    let (content, tail) = body
        .split_once("\n\n## Nguồn\n")
        .ok_or("Memory thiếu phần Nguồn.")?;
    if !tail.contains("\n## Lịch sử\n") {
        return Err("Memory thiếu phần Lịch sử.".into());
    }
    record.content = content.trim().into();
    validate(&record)?;
    Ok(record)
}
fn render(record: &MemoryRecord) -> Result<String, String> {
    validate(record)?;
    let mut metadata = serde_json::to_value(record).map_err(|e| e.to_string())?;
    for field in ["path", "revision", "content"] {
        metadata.as_object_mut().unwrap().remove(field);
    }
    let yaml = serde_yaml::to_string(&metadata).map_err(|e| e.to_string())?;
    let sources = record
        .sources
        .iter()
        .map(|s| {
            format!(
                "- Phiên `{}` · tin `{}`: {}",
                s.session_id,
                s.message_id,
                s.quote.replace('\n', " ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let history = record
        .history
        .iter()
        .map(|h| {
            format!(
                "### {} — {} ({})\n\n{}",
                h.updated_at, h.title, h.status, h.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let text = format!(
        "---\n{yaml}---\n\n## Nội dung\n\n{}\n\n## Nguồn\n\n> Nguồn và lịch sử hiển thị được sinh từ frontmatter. Chỉnh memory tại phần Nội dung; chỉnh thuộc tính trong frontmatter.\n\n{}\n\n## Lịch sử\n\n{}\n",
        record.content.trim(),
        sources,
        history
    );
    if text.len() as u64 > MAX_FILE {
        return Err("Memory và lịch sử vượt giới hạn 256 KiB; không ghi.".into());
    }
    Ok(text)
}
fn scan(root: &Path) -> Result<MemoryList, String> {
    let directory = store(root)?;
    let mut result = MemoryList {
        records: Vec::new(),
        diagnostics: Vec::new(),
        truncated: false,
    };
    let mut pending = vec![(directory.join("facts"), 0usize)];
    let (mut entries, mut files, mut bytes) = (0usize, 0usize, 0u64);
    let started = std::time::Instant::now();
    'scan: while let Some((directory, depth)) = pending.pop() {
        let directory = safe_existing(root, &directory)?;
        for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
            entries += 1;
            if entries > MAX_ENTRIES
                || files >= MAX_FILES
                || bytes >= MAX_BYTES
                || started.elapsed() > std::time::Duration::from_secs(2)
            {
                result.truncated = true;
                break 'scan;
            }
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            let relative = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            if linked(&metadata) {
                result
                    .diagnostics
                    .push(format!("{relative}: bỏ qua symlink/junction."));
                continue;
            }
            if metadata.is_dir() {
                if depth < 24 {
                    pending.push((path, depth + 1));
                } else {
                    result.truncated = true;
                }
                continue;
            }
            if !metadata.is_file()
                || path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| !e.eq_ignore_ascii_case("md"))
                    .unwrap_or(true)
            {
                continue;
            }
            files += 1;
            if metadata.len() > MAX_FILE || bytes + metadata.len() > MAX_BYTES {
                result.truncated = true;
                result
                    .diagnostics
                    .push(format!("{relative}: vượt giới hạn đọc."));
                continue;
            }
            let text = read_text(root, &path)?;
            bytes += text.len() as u64;
            match parse(&text) {
                Ok(mut record) => {
                    record.path = relative;
                    record.revision = hash(text.as_bytes());
                    result.records.push(record);
                }
                Err(e) => result.diagnostics.push(format!("{relative}: {e}")),
            }
        }
    }
    let mut counts = HashMap::new();
    for record in &result.records {
        *counts.entry(record.id.clone()).or_insert(0usize) += 1;
    }
    result.records.retain(|r| {
        if counts[&r.id] > 1 {
            result
                .diagnostics
                .push(format!("{}: ID trùng {}; bản ghi bị chặn.", r.path, r.id));
            false
        } else {
            true
        }
    });
    result
        .records
        .sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
    Ok(result)
}
pub fn list(path: &str) -> Result<MemoryList, String> {
    scan(&vault(path)?)
}
pub fn read(path: &str, id: &str) -> Result<MemoryRecord, String> {
    list(path)?
        .records
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| "Memory không tồn tại hoặc có lỗi; tải lại danh sách.".into())
}

fn atomic_write(
    root: &Path,
    path: &Path,
    text: &str,
    revision: Option<&str>,
) -> Result<(), String> {
    let parent = safe_existing(root, path.parent().ok_or("Memory thiếu thư mục cha.")?)?;
    let temporary = parent.join(format!(".lim-{}.tmp", uuid::Uuid::new_v4()));
    let outcome = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        safe_existing(root, &parent)?;
        if let Some(expected) = revision {
            if hash(read_text(root, path)?.as_bytes()) != expected {
                return Err("Memory đã được sửa trong Obsidian; tải lại trước khi ghi.".into());
            }
        } else if fs::symlink_metadata(path).is_ok() {
            return Err("Tệp memory đã tồn tại; không ghi đè.".into());
        }
        replace(&temporary, path, revision.is_some())
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    outcome
}
#[cfg(windows)]
fn replace(from: &Path, to: &Path, overwrite: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    let flags = 8 | if overwrite { 1 } else { 0 }; // WRITE_THROUGH, optional REPLACE_EXISTING.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
#[cfg(not(windows))]
fn replace(from: &Path, to: &Path, overwrite: bool) -> Result<(), String> {
    if overwrite {
        fs::rename(from, to).map_err(|e| e.to_string())?;
    } else {
        fs::hard_link(from, to).map_err(|e| e.to_string())?;
        fs::remove_file(from).map_err(|e| e.to_string())?;
    }
    File::open(to.parent().unwrap())
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
}
fn rebuild_index(root: &Path, records: &[MemoryRecord]) -> Result<(), String> {
    let path = store(root)?.join("index.md");
    let revision = match fs::symlink_metadata(&path) {
        Ok(_) => Some(hash(read_text(root, &path)?.as_bytes())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    let rows = records
        .iter()
        .map(|r| {
            format!(
                "- [[{}|{}]] — {}{}",
                r.path.trim_end_matches(".md"),
                r.title
                    .replace(|c| matches!(c, '|' | '[' | ']' | '\n'), " "),
                r.status,
                if r.pinned { " · ghim" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    atomic_write(
        root,
        &path,
        &format!("# Bộ nhớ Lim\n\nTrang tổng hợp tự sinh. Sửa nội dung tại từng note.\n\n{rows}\n"),
        revision.as_deref(),
    )
}
fn finish(root: &Path, mut record: MemoryRecord, text: String) -> MemoryWriteResult {
    record.revision = hash(text.as_bytes());
    let diagnostics = match scan(root) {
        Ok(mut scanned) => {
            if scanned.truncated {
                scanned
                    .diagnostics
                    .push("Kho memory chưa được đọc đủ; index có thể thiếu bản ghi.".into());
            }
            if let Err(e) = rebuild_index(root, &scanned.records) {
                scanned
                    .diagnostics
                    .push(format!("Memory đã lưu; chưa cập nhật index: {e}"));
            }
            scanned.diagnostics
        }
        Err(e) => vec![format!("Memory đã lưu; chưa cập nhật index: {e}")],
    };
    MemoryWriteResult {
        record,
        diagnostics,
    }
}
fn writable(root: &Path) -> Result<MemoryList, String> {
    let scanned = scan(root)?;
    if scanned.truncated || !scanned.diagnostics.is_empty() {
        return Err("Kho memory có tệp lỗi, ID trùng hoặc chưa đọc đủ; sửa lỗi trước khi ghi để tránh mất dữ liệu.".into());
    }
    Ok(scanned)
}
pub fn create(
    path: &str,
    title: &str,
    category: &str,
    content: &str,
    sources: &[MemorySource],
    operation_id: &str,
) -> Result<MemoryWriteResult, String> {
    let _lock = WRITER.lock().map_err(|_| "Memory writer không sẵn sàng.")?;
    let root = vault(path)?;
    let current = writable(&root)?;
    if operation_id.trim().is_empty() || operation_id.len() > 200 {
        return Err("Operation ID không hợp lệ.".into());
    }
    if let Some(record) = current
        .records
        .iter()
        .find(|r| r.operation_ids.iter().any(|op| op == operation_id))
    {
        return Ok(MemoryWriteResult {
            record: record.clone(),
            diagnostics: vec![],
        });
    }
    // Includes withdrawn records: automatic retries must not recreate forgotten content.
    if current
        .records
        .iter()
        .any(|r| normalize(&r.content) == normalize(content))
    {
        return Err("Nội dung memory đã tồn tại (có thể đã được quên).".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let slug: String = normalize(title)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(48)
        .collect();
    let date = now();
    let record = MemoryRecord {
        schema_version: 1,
        id: id.clone(),
        path: format!("LimMemory/facts/{}-{id}.md", slug.trim_matches('-')),
        revision: String::new(),
        title: title.trim().into(),
        category: category.trim().into(),
        status: "active".into(),
        pinned: false,
        created_at: date.clone(),
        updated_at: date,
        content: content.trim().into(),
        sources: sources.to_vec(),
        history: vec![],
        operation_ids: vec![operation_id.into()],
    };
    let text = render(&record)?;
    atomic_write(&root, &root.join(&record.path), &text, None)?;
    Ok(finish(&root, record, text))
}
#[allow(clippy::too_many_arguments)]
pub fn update(
    path: &str,
    id: &str,
    revision: &str,
    title: &str,
    category: &str,
    content: &str,
    pinned: bool,
    status: &str,
    sources: &[MemorySource],
    operation_id: &str,
) -> Result<MemoryWriteResult, String> {
    let _lock = WRITER.lock().map_err(|_| "Memory writer không sẵn sàng.")?;
    let root = vault(path)?;
    let current = writable(&root)?;
    if operation_id.trim().is_empty() || operation_id.len() > 200 {
        return Err("Operation ID không hợp lệ.".into());
    }
    if let Some(record) = current
        .records
        .iter()
        .find(|r| r.operation_ids.iter().any(|op| op == operation_id))
    {
        if record.id != id {
            return Err("Operation ID đã dùng cho memory khác.".into());
        }
        return Ok(MemoryWriteResult {
            record: record.clone(),
            diagnostics: vec![],
        });
    }
    let mut record = current
        .records
        .into_iter()
        .find(|r| r.id == id)
        .ok_or("Memory không tồn tại hoặc bị chặn.")?;
    if record.revision != revision {
        return Err("Memory đã thay đổi; tải lại trước khi ghi.".into());
    }
    record.history.push(MemoryHistory {
        updated_at: record.updated_at.clone(),
        title: record.title.clone(),
        category: record.category.clone(),
        status: record.status.clone(),
        pinned: record.pinned,
        content: record.content.clone(),
        sources: record.sources.clone(),
    });
    record.title = title.trim().into();
    record.category = category.trim().into();
    record.content = content.trim().into();
    record.pinned = pinned;
    record.status = status.into();
    record.updated_at = now();
    for source in sources {
        if !record.sources.contains(source) {
            record.sources.push(source.clone());
        }
    }
    record.operation_ids.push(operation_id.into());
    let text = render(&record)?;
    atomic_write(&root, &root.join(&record.path), &text, Some(revision))?;
    Ok(finish(&root, record, text))
}
pub fn recall(path: &str, query: &str) -> Result<MemoryList, String> {
    let mut result = list(path)?;
    let normalized = normalize(query);
    let words: Vec<_> = normalized
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2)
        .take(32)
        .collect();
    let query_words = format!(" {} ", words.join(" "));
    let score = |r: &MemoryRecord| {
        let title = normalize(&r.title);
        let content = normalize(&r.content);
        // Evidence helps discover translated facts; context still uses current content only.
        let source_quotes: Vec<_> = r
            .sources
            .iter()
            .map(|source| normalize(&source.quote))
            .collect();
        // Category labels cover short facts without introducing semantic retrieval.
        let category_keywords: &[&str] = match r.category.as_str() {
            "preference" => &["so thich", "thich", "preference"],
            "personal" => &["ca nhan", "toi la", "personal"],
            "work" => &["cong viec", "du an", "quyet dinh", "work"],
            _ => &[],
        };
        let category_match = category_keywords
            .iter()
            .any(|keyword| query_words.contains(&format!(" {keyword} ")));
        usize::from(category_match)
            + words
                .iter()
                .map(|w| {
                    3 * usize::from(title.contains(*w))
                        + usize::from(content.contains(*w))
                        + usize::from(source_quotes.iter().any(|quote| quote.contains(*w)))
                })
                .sum::<usize>()
    };
    result
        .records
        .retain(|r| r.status == "active" && (r.pinned || score(r) > 0));
    result.records.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| score(b).cmp(&score(a)))
            .then(a.id.cmp(&b.id))
    });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("lim-memory-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn lifecycle_external_edit_retry_and_recall() {
        let f = Fixture::new();
        initialize(f.path()).unwrap();
        let source = MemorySource {
            session_id: "session".into(),
            message_id: "user".into(),
            quote: "Tôi thích cà phê đen".into(),
        };
        let r = create(
            f.path(),
            "Sở thích đồ uống",
            "preference",
            "Tôi thích cà phê đen",
            &[source],
            "create-1",
        )
        .unwrap()
        .record;
        assert_eq!(recall(f.path(), "ca phe den").unwrap().records.len(), 1);
        assert_eq!(
            create(f.path(), "x", "x", "x", &[], "create-1")
                .unwrap()
                .record
                .id,
            r.id
        );
        let file = f.0.join(&r.path);
        let edited = fs::read_to_string(&file).unwrap().replace(
            "## Nội dung\n\nTôi thích cà phê đen",
            "## Nội dung\n\nTôi thích trà xanh",
        );
        fs::write(&file, edited).unwrap();
        assert!(update(
            f.path(),
            &r.id,
            &r.revision,
            &r.title,
            &r.category,
            &r.content,
            false,
            "active",
            &[],
            "stale"
        )
        .is_err());
        let r = read(f.path(), &r.id).unwrap();
        assert_eq!(r.content, "Tôi thích trà xanh");
        let moved = f.0.join("LimMemory/facts/renamed.md");
        fs::rename(file, &moved).unwrap();
        let r = read(f.path(), &r.id).unwrap();
        let forgotten = update(
            f.path(),
            &r.id,
            &r.revision,
            &r.title,
            &r.category,
            &r.content,
            false,
            "retracted",
            &[],
            "forget",
        )
        .unwrap()
        .record;
        assert_eq!(forgotten.history.len(), 1);
        assert!(recall(f.path(), "tra xanh").unwrap().records.is_empty());
        assert_eq!(
            update(
                f.path(),
                &r.id,
                &r.revision,
                "ignored",
                "ignored",
                "ignored",
                false,
                "active",
                &[],
                "forget"
            )
            .unwrap()
            .record
            .history
            .len(),
            1
        );
        assert!(create(f.path(), "Tea", "fact", "Tôi thích trà xanh", &[], "new").is_err());
    }
    #[test]
    fn category_cues_recall_short_facts_but_never_withdrawn_facts() {
        let f = Fixture::new();
        initialize(f.path()).unwrap();
        let r = create(f.path(), "trà xanh", "preference", "trà xanh", &[], "tea")
            .unwrap()
            .record;
        assert_eq!(
            recall(f.path(), "Tôi thích uống gì?").unwrap().records[0].id,
            r.id
        );
        assert!(recall(f.path(), "2 + 2 bằng bao nhiêu?")
            .unwrap()
            .records
            .is_empty());
        update(
            f.path(),
            &r.id,
            &r.revision,
            &r.title,
            &r.category,
            &r.content,
            false,
            "retracted",
            &[],
            "forget-tea",
        )
        .unwrap();
        assert!(recall(f.path(), "Tôi thích uống gì?")
            .unwrap()
            .records
            .is_empty());
    }
    #[test]
    fn source_keywords_find_translated_content_without_exposing_evidence_in_context() {
        let f = Fixture::new();
        initialize(f.path()).unwrap();
        let source = MemorySource {
            session_id: "session".into(),
            message_id: "user".into(),
            quote: "Tôi thích uống trà xanh. Evidence-only marker.".into(),
        };
        let r = create(
            f.path(),
            "Beverage",
            "personal",
            "The user enjoys green tea.",
            &[source],
            "translated-tea",
        )
        .unwrap()
        .record;
        let found = recall(f.path(), "Tôi thích uống gì?").unwrap();
        assert_eq!(found.records.len(), 1);
        assert_eq!(found.records[0].content, "The user enjoys green tea.");
        let context =
            crate::vault::memory_context(&[crate::memory_runtime::record_hit(&found.records[0])]);
        assert!(context.contains("The user enjoys green tea."));
        assert!(!context.contains("Evidence-only marker"));
        assert!(!context.contains("Tôi thích uống trà xanh"));
        update(
            f.path(),
            &r.id,
            &r.revision,
            &r.title,
            &r.category,
            &r.content,
            false,
            "retracted",
            &[],
            "forget-translated-tea",
        )
        .unwrap();
        assert!(recall(f.path(), "Tôi thích uống gì?")
            .unwrap()
            .records
            .is_empty());
    }
    #[test]
    fn unknown_duplicate_and_invalid_fail_closed() {
        let f = Fixture::new();
        fs::create_dir(f.0.join("LimMemory")).unwrap();
        assert!(initialize(f.path()).is_err());
        fs::remove_dir(f.0.join("LimMemory")).unwrap();
        initialize(f.path()).unwrap();
        let r = create(f.path(), "Title", "fact", "content", &[], "one")
            .unwrap()
            .record;
        fs::copy(f.0.join(&r.path), f.0.join("LimMemory/facts/copy.md")).unwrap();
        let listed = list(f.path()).unwrap();
        assert!(listed.records.is_empty());
        assert_eq!(listed.diagnostics.len(), 2);
        assert!(create(f.path(), "Other", "fact", "other", &[], "two").is_err());
        fs::remove_file(f.0.join("LimMemory/facts/copy.md")).unwrap();
        fs::write(f.0.join("LimMemory/facts/bad.md"), "---\nbroken").unwrap();
        assert_eq!(list(f.path()).unwrap().diagnostics.len(), 1);
    }
    #[test]
    fn index_failure_preserves_commit_and_history_not_recalled() {
        let f = Fixture::new();
        initialize(f.path()).unwrap();
        fs::remove_file(f.0.join("LimMemory/index.md")).unwrap();
        fs::create_dir(f.0.join("LimMemory/index.md")).unwrap();
        let saved = create(f.path(), "Title", "fact", "oldword", &[], "one").unwrap();
        assert!(!saved.diagnostics.is_empty());
        let r = saved.record;
        update(
            f.path(),
            &r.id,
            &r.revision,
            &r.title,
            &r.category,
            "newword",
            false,
            "active",
            &[],
            "two",
        )
        .unwrap();
        assert!(recall(f.path(), "oldword").unwrap().records.is_empty());
        assert_eq!(recall(f.path(), "newword").unwrap().records.len(), 1);
    }
    #[cfg(windows)]
    #[test]
    fn junction_is_not_followed() {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        let f = Fixture::new();
        let outside = Fixture::new();
        initialize(f.path()).unwrap();
        let junction = f.0.join("LimMemory/facts/link");
        let command = format!(
            "mklink /J \"{}\" \"{}\"",
            junction.to_string_lossy().replace('/', "\\"),
            outside.0.to_string_lossy().replace('/', "\\")
        );
        let output = Command::new("cmd")
            .args(["/D", "/C"])
            .raw_arg(command)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "mklink failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let result = list(f.path()).unwrap();
        assert!(!result.diagnostics.is_empty());
        fs::remove_dir(junction).unwrap();
    }
    #[test]
    fn invalid_sources_reserved_sections_and_oversized_files_do_not_write() {
        let f = Fixture::new();
        initialize(f.path()).unwrap();
        let invalid = MemorySource {
            session_id: "s".into(),
            message_id: "m".into(),
            quote: String::new(),
        };
        assert!(create(
            f.path(),
            "Title",
            "fact",
            "content",
            &[invalid],
            "invalid-source"
        )
        .is_err());
        assert!(create(
            f.path(),
            "Title",
            "fact",
            "## Nguồn\nreserved",
            &[],
            "invalid-section"
        )
        .is_err());
        assert!(list(f.path()).unwrap().records.is_empty());
        fs::write(
            f.0.join("LimMemory/facts/oversized.md"),
            vec![b'x'; MAX_FILE as usize + 1],
        )
        .unwrap();
        let listed = list(f.path()).unwrap();
        assert!(listed.truncated);
        assert_eq!(listed.diagnostics.len(), 1);
        assert!(create(f.path(), "Title", "fact", "content", &[], "blocked-write").is_err());
    }
}
