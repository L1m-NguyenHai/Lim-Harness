//! Explicit first-run setup; Obsidian vault registration remains owned by Obsidian.
use crate::{memory, memory_runtime, runtime::Runtime, vault};
use std::{fs, path::Path, sync::Arc};
use tauri::{Manager, State};

fn initialize_directory(path: &Path) -> Result<String, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err("Vault mặc định không được là junction hoặc symlink.".into());
                }
            }
            if !meta.is_dir() || meta.file_type().is_symlink() {
                return Err("Vault mặc định phải là thư mục thật.".into());
            }
            if !path.join("LimMemory").exists()
                && fs::read_dir(path)
                    .map_err(|e| e.to_string())?
                    .next()
                    .is_some()
            {
                return Err(
                    "Thư mục mặc định đã có dữ liệu. Kết nối thủ công hoặc chọn thư mục khác."
                        .into(),
                );
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|e| format!("Không tạo được vault: {e}"))?;
        }
        Err(e) => return Err(e.to_string()),
    }
    let canonical = vault::validate_vault_path(&path.to_string_lossy())?;
    let result = memory::initialize(&canonical)?;
    if result.truncated || !result.diagnostics.is_empty() {
        return Err(format!(
            "Kho memory cần kiểm tra trước khi kết nối: {}",
            result.diagnostics.join("; ")
        ));
    }
    Ok(canonical)
}

#[tauri::command]
pub fn initialize_memory_vault(
    app: tauri::AppHandle,
    state: State<'_, Arc<Runtime>>,
) -> Result<String, String> {
    let path = app
        .path()
        .document_dir()
        .map_err(|e| e.to_string())?
        .join("Lim Harness Memory");
    let _control = state
        .memory_control
        .lock()
        .map_err(|_| "Memory không khả dụng")?;
    let canonical = initialize_directory(&path)?;
    memory_runtime::invalidate(&state)?;
    memory_runtime::enable_initialized_vault(&state, &canonical)?;
    state.store.set_setting("vault", &canonical)?;
    Ok(canonical)
}

fn obsidian_uri(path: &str) -> String {
    let path = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
    };
    let mut uri = reqwest::Url::parse("obsidian://open").expect("fixed URI");
    uri.query_pairs_mut().append_pair("path", &path);
    uri.to_string().replace('+', "%20")
}

#[tauri::command]
pub fn open_memory_obsidian(
    app: tauri::AppHandle,
    state: State<'_, Arc<Runtime>>,
) -> Result<String, String> {
    let path = state.store.setting("vault")?;
    if path.is_empty() {
        return Err("Khởi tạo hoặc kết nối vault trước.".into());
    }
    let canonical = vault::validate_vault_path(&path)?;
    let index = Path::new(&canonical).join("LimMemory/index.md");
    if !index.is_file() {
        return Err("Bật bộ nhớ cho vault để tạo trang tổng hợp trước.".into());
    }
    // Read only: Obsidian owns registration. Unknown installations use its vault manager.
    let registry = app
        .path()
        .config_dir()
        .ok()
        .map(|path| path.join("obsidian/obsidian.json"))
        .and_then(|path| {
            fs::metadata(&path)
                .ok()
                .filter(|meta| meta.len() <= 1_048_576)
                .and_then(|_| fs::read_to_string(path).ok())
        })
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let registered = registry
        .as_ref()
        .is_some_and(|value| registered_vault(value, Path::new(&canonical)));
    if registered {
        launch_uri(&obsidian_uri(&index.to_string_lossy()))?;
        Ok("Đã gửi yêu cầu mở LimMemory/index.md trong Obsidian.".into())
    } else {
        launch_uri("obsidian://choose-vault")?;
        let display = canonical.strip_prefix(r"\\?\").unwrap_or(&canonical);
        Ok(format!("Đã mở trình quản lý vault. Chọn Open folder as vault → Open, rồi chọn {display}. Sau đó bấm Mở trong Obsidian lần nữa."))
    }
}

fn registered_vault(registry: &serde_json::Value, target: &Path) -> bool {
    registry
        .get("vaults")
        .and_then(|value| value.as_object())
        .is_some_and(|vaults| {
            vaults
                .values()
                .filter_map(|value| value.get("path").and_then(|path| path.as_str()))
                .filter_map(|path| fs::canonicalize(path).ok())
                .any(|root| target.starts_with(root))
        })
}

#[cfg(windows)]
fn launch_uri(uri: &str) -> Result<(), String> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            window: *mut std::ffi::c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }
    let operation: Vec<u16> = OsStr::new("open").encode_wide().chain(Some(0)).collect();
    let file: Vec<u16> = OsStr::new(uri).encode_wide().chain(Some(0)).collect();
    // Fixed scheme, native-generated path; no command shell or user-supplied executable.
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            ptr::null(),
            ptr::null(),
            1,
        )
    };
    if result <= 32 {
        return Err(
            "Không mở được Obsidian. Cài và mở Obsidian một lần để đăng ký liên kết obsidian://."
                .into(),
        );
    }
    Ok(())
}

#[cfg(not(windows))]
fn launch_uri(uri: &str) -> Result<(), String> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let status = std::process::Command::new(program)
        .arg(uri)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Không mở được Obsidian. Kiểm tra ứng dụng và liên kết obsidian://.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initializes_reuses_and_preserves_unrecognized_directory() {
        let root = std::env::temp_dir().join(format!("lim-setup-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let vault = root.join("Memory");
        let first = initialize_directory(&vault).unwrap();
        assert!(vault.join("LimMemory/index.md").is_file());
        assert_eq!(initialize_directory(&vault).unwrap(), first);
        let occupied = root.join("Existing");
        fs::create_dir(&occupied).unwrap();
        fs::write(occupied.join("keep.md"), "keep").unwrap();
        assert!(initialize_directory(&occupied).is_err());
        assert_eq!(
            fs::read_to_string(occupied.join("keep.md")).unwrap(),
            "keep"
        );
        assert!(!occupied.join("LimMemory").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn uri_encodes_path_and_cannot_add_parameters() {
        let path = r"C:\Notes & riêng#1\LimMemory\index.md";
        let uri = reqwest::Url::parse(&obsidian_uri(path)).unwrap();
        assert_eq!(uri.scheme(), "obsidian");
        assert_eq!(
            uri.query_pairs().collect::<Vec<_>>(),
            vec![("path".into(), path.into())]
        );
    }

    #[test]
    fn only_registered_containing_vault_opens_note() {
        let root = std::env::temp_dir().join(format!("lim-registry-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let canonical = fs::canonicalize(&root).unwrap();
        let registry = serde_json::json!({"vaults":{"test":{"path":root}}});
        assert!(registered_vault(&registry, &canonical));
        assert!(registered_vault(&registry, &canonical.join("nested")));
        assert!(!registered_vault(
            &registry,
            &canonical.with_extension("other")
        ));
        assert!(!registered_vault(&serde_json::json!({}), &canonical));
        assert!(!registered_vault(
            &serde_json::json!({"vaults":{"bad":{"path":false}}}),
            &canonical
        ));
        fs::remove_dir(&root).unwrap();
    }
}
