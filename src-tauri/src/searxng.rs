//! Lifecycle of the local SearXNG installed by scripts/setup-searxng.ps1 in `<repo>/.local`:
//! started when the app starts, stopped when the app exits.
//!
//! On Windows the processes run inside a Job Object with KILL_ON_JOB_CLOSE. The app holds the
//! job handle for its whole life, so when the app exits for any reason (window closed, crash,
//! Ctrl+C on `tauri dev`) Windows closes the handle and ends SearXNG and its child processes.
//! A SearXNG already listening that belongs to this install (left by an older build or a
//! crash) is adopted into the job; one from anywhere else is used but never stopped.
use std::{
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::Duration,
};

pub const ADDRESS: &str = "127.0.0.1:8888";

struct Managed {
    child: Option<Child>,
    #[cfg(windows)]
    job: isize,
}
static STATE: Mutex<Option<Managed>> = Mutex::new(None);

fn local_dir() -> PathBuf {
    // The dev app runs from source: src-tauri/.. is the repository.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(".local")
}
fn venv_python() -> PathBuf {
    local_dir().join(if cfg!(windows) { "searxng-venv/Scripts/python.exe" } else { "searxng-venv/bin/python" })
}
pub fn installed() -> bool {
    venv_python().exists() && local_dir().join("searxng-src/searx/webapp.py").exists()
}
fn listening() -> bool {
    TcpStream::connect_timeout(&ADDRESS.parse().unwrap(), Duration::from_millis(300)).is_ok()
}

/// Starts (or adopts) the local SearXNG. Safe to call repeatedly; returns what it did.
pub fn start() -> &'static str {
    let Ok(mut state) = STATE.lock() else { return "busy" };
    if state.is_some() {
        return "running";
    }
    if !installed() {
        return "not-installed";
    }
    #[cfg(windows)]
    let Some(job) = job::create() else { return "no-job" };
    if listening() {
        #[cfg(windows)]
        {
            if job::adopt_install(job, &venv_python()) > 0 {
                *state = Some(Managed { child: None, job });
                return "adopted";
            }
            job::close(job);
        }
        return "external";
    }
    let local = local_dir();
    let mut cmd = Command::new(venv_python());
    cmd.args(["-m", "searx.webapp"])
        .current_dir(local.join("searxng-src"))
        .env("SEARXNG_SETTINGS_PATH", local.join("searxng-settings.yml"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let Ok(child) = cmd.spawn() else {
        #[cfg(windows)]
        job::close(job);
        return "spawn-failed";
    };
    #[cfg(windows)]
    {
        // The venv python.exe is a launcher that starts the real interpreter as a child.
        // Children created after the assignment join the job automatically; the short scan
        // catches one created before it. It stops as soon as the job is no longer ours.
        let pid = child.id();
        job::adopt_tree(job, vec![pid]);
        std::thread::spawn(move || {
            for _ in 0..10 {
                std::thread::sleep(Duration::from_millis(300));
                let Ok(state) = STATE.lock() else { return };
                if state.as_ref().map(|m| m.job) != Some(job) {
                    return;
                }
                job::adopt_tree(job, vec![pid]);
            }
        });
    }
    *state = Some(Managed { child: Some(child), #[cfg(windows)] job });
    "started"
}

/// One install at a time (the button can be pressed again while pip is still running).
static INSTALLING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn on_path(program: &str) -> Option<PathBuf> {
    let finder = if cfg!(windows) { "where" } else { "which" };
    let mut cmd = Command::new(finder);
    cmd.arg(program).stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    String::from_utf8_lossy(&out.stdout).lines().map(str::trim).find(|l| !l.is_empty()).map(PathBuf::from)
}

/// A Python the setup script can use: the `py` launcher, or a `python` that is not the
/// Microsoft Store alias (Store Python redirects venvs under AppData, see the script).
fn usable_python() -> bool {
    on_path("py").is_some() || on_path("python").is_some_and(|p| !p.to_string_lossy().contains("WindowsApps"))
}

/// What the Settings window shows for SearXNG.
pub fn status() -> serde_json::Value {
    serde_json::json!({
        "installed": installed(),
        "running": listening(),
        "installing": INSTALLING.load(std::sync::atomic::Ordering::SeqCst),
        "git": on_path("git").is_some(),
        "python": usable_python(),
    })
}

/// Runs scripts/setup-searxng.ps1 (the same script as the manual setup), streaming its output
/// as `{"type":"log","line":…}`, then starts SearXNG with the app's lifecycle (not the script's
/// -Start, whose process would outlive the app). Ends with `done` or `failed`.
#[tauri::command]
pub async fn install_searxng(on_event: tauri::ipc::Channel<serde_json::Value>) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("SearXNG đang được cài".into());
    }
    let result = tauri::async_runtime::spawn_blocking(move || install_blocking(&on_event)).await.map_err(|e| e.to_string()).and_then(|r| r);
    INSTALLING.store(false, Ordering::SeqCst);
    result
}

fn install_blocking(on_event: &tauri::ipc::Channel<serde_json::Value>) -> Result<(), String> {
    use std::io::{BufRead, BufReader};
    let send = |value: serde_json::Value| {
        let _ = on_event.send(value);
    };
    let fail = |message: String| {
        send(serde_json::json!({"type": "failed", "error": message}));
        Err(message)
    };
    if !cfg!(windows) {
        return fail("Bộ cài tự động hiện chỉ hỗ trợ Windows.".into());
    }
    if on_path("git").is_none() {
        return fail("Chưa có Git. Cài Git for Windows (git-scm.com) rồi bấm cài lại.".into());
    }
    if !usable_python() {
        return fail("Chưa có Python 3.10+ từ python.org (bản Microsoft Store không dùng được). Cài Python rồi bấm cài lại.".into());
    }
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("scripts").join("setup-searxng.ps1");
    if !script.exists() {
        return fail(format!("Không tìm thấy {}", script.display()));
    }
    send(serde_json::json!({"type": "log", "line": "Đang tải mã nguồn SearXNG và cài thư viện Python (có thể mất vài phút)…"}));
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => return fail(format!("Không chạy được PowerShell: {error}")),
    };
    // stderr on its own thread so a full pipe cannot block the script.
    let stderr = child.stderr.take().map(|err| {
        std::thread::spawn(move || BufReader::new(err).lines().map_while(Result::ok).collect::<Vec<_>>())
    });
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if !line.trim().is_empty() {
                send(serde_json::json!({"type": "log", "line": line}));
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let errors = stderr.and_then(|t| t.join().ok()).unwrap_or_default();
    if !status.success() || !installed() {
        let tail = errors.iter().rev().filter(|l| !l.trim().is_empty()).take(4).cloned().collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ");
        return fail(if tail.is_empty() { format!("Script cài đặt dừng với mã {}", status.code().unwrap_or(-1)) } else { tail });
    }
    let started = start();
    send(serde_json::json!({"type": "done", "start": started}));
    Ok(())
}

/// Stops the SearXNG this app started or adopted. Called on app exit.
pub fn stop() {
    let Ok(mut state) = STATE.lock() else { return };
    if let Some(mut managed) = state.take() {
        #[cfg(windows)]
        job::terminate(managed.job);
        if let Some(child) = managed.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(windows)]
mod job {
    use std::{ffi::c_void, path::Path};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS},
            JobObjects::{AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE},
            Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE},
        },
    };

    pub fn create() -> Option<isize> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const c_void, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
            if ok == 0 {
                CloseHandle(job);
                return None;
            }
            Some(job as isize)
        }
    }
    pub fn close(job: isize) {
        unsafe { CloseHandle(job as HANDLE) };
    }
    pub fn terminate(job: isize) {
        unsafe {
            TerminateJobObject(job as HANDLE, 0);
            CloseHandle(job as HANDLE);
        }
    }
    fn assign(job: isize, pid: u32) -> bool {
        unsafe {
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if process.is_null() {
                return false;
            }
            let ok = AssignProcessToJobObject(job as HANDLE, process) != 0;
            CloseHandle(process);
            ok
        }
    }
    /// (pid, parent pid) of every process.
    fn processes() -> Vec<(u32, u32)> {
        let mut list = Vec::new();
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return list;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snapshot, &mut entry) != 0;
            while more {
                list.push((entry.th32ProcessID, entry.th32ParentProcessID));
                more = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
        }
        list
    }
    fn image(pid: u32) -> Option<String> {
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut buffer = [0u16; 1024];
            let mut size = buffer.len() as u32;
            let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut size) != 0;
            CloseHandle(process);
            ok.then(|| String::from_utf16_lossy(&buffer[..size as usize]))
        }
    }
    fn normalize(path: &str) -> String {
        path.trim_start_matches(r"\\?\").replace('/', "\\").to_lowercase()
    }
    /// Assigns the given processes and all their descendants to the job.
    pub fn adopt_tree(job: isize, roots: Vec<u32>) -> usize {
        let all = processes();
        let mut members = roots;
        let mut index = 0;
        while index < members.len() {
            let parent = members[index];
            members.extend(all.iter().filter(|(pid, ppid)| *ppid == parent && !members.contains(pid)).map(|(pid, _)| *pid).collect::<Vec<_>>());
            index += 1;
        }
        members.into_iter().filter(|pid| assign(job, *pid)).count()
    }
    /// Adopts running SearXNG processes of this install (by the venv interpreter path).
    pub fn adopt_install(job: isize, python: &Path) -> usize {
        let wanted = python.canonicalize().map(|p| normalize(&p.display().to_string())).unwrap_or_default();
        if wanted.is_empty() {
            return 0;
        }
        let roots: Vec<u32> = processes().into_iter().filter(|(pid, _)| image(*pid).is_some_and(|path| normalize(&path) == wanted)).map(|(pid, _)| pid).collect();
        if roots.is_empty() { 0 } else { adopt_tree(job, roots) }
    }
}
