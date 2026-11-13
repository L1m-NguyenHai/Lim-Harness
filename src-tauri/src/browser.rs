//! In-app browser. Each tab is a native child webview (WebView2 on Windows) inside the main
//! window, placed over the browser panel the frontend reserves. The user browses it like a
//! normal browser; agent mode drives it with the `browser_*` tools (open, search, read the
//! page, click, type, scroll, tabs). Page content is read with injected scripts, so the
//! model sees text plus a numbered list of interactive elements (browser-use style).
//!
//! Locking: `STATE` is never held while waiting on the main thread (creating a webview or
//! evaluating a script), because page-load callbacks run there and lock it too.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url, WebviewBuilder, WebviewUrl};

/// Heavy pages keep loading images long after the text is there; read them as they are.
const LOAD_TIMEOUT: Duration = Duration::from_secs(10);
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_TABS: usize = 8;

static APP: OnceLock<AppHandle> = OnceLock::new();
static STATE: Mutex<Browser> = Mutex::new(Browser { tabs: Vec::new(), active: None, next: 1, bounds: None, visible: false });

#[derive(Clone, Serialize)]
pub struct Tab {
    id: u32,
    title: String,
    url: String,
    loading: bool,
    /// Currently drawn over the panel (not sent to the UI).
    #[serde(skip)]
    shown: bool,
}
struct Browser {
    tabs: Vec<Tab>,
    active: Option<u32>,
    next: u32,
    /// Panel rectangle in logical pixels (x, y, width, height), reported by the frontend.
    bounds: Option<Bounds>,
    visible: bool,
}
#[derive(Clone, Copy, Deserialize)]
pub struct Bounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

pub fn init(app: AppHandle) {
    let _ = APP.set(app);
}
fn app() -> Result<&'static AppHandle, String> {
    APP.get().ok_or_else(|| "Trình duyệt chưa sẵn sàng".to_string())
}
fn label(id: u32) -> String {
    format!("browser-{id}")
}
fn lock() -> std::sync::MutexGuard<'static, Browser> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sends the tab list to the main window. `reveal` asks the UI to open the browser panel.
fn publish(reveal: bool) {
    let payload = {
        let state = lock();
        json!({"tabs": state.tabs, "active": state.active, "reveal": reveal})
    };
    if let Ok(app) = app() {
        let _ = app.emit_to("main", "browser-state", payload);
    }
}

/// Shows the active tab over the panel and hides the others.
fn layout() {
    let Ok(app) = app() else { return };
    let (tabs, active, bounds, visible) = {
        let state = lock();
        (state.tabs.iter().map(|t| t.id).collect::<Vec<_>>(), state.active, state.bounds, state.visible)
    };
    let mut hid_shown = false;
    for id in tabs {
        let Some(webview) = app.get_webview(&label(id)) else { continue };
        let show = matches!(bounds, Some(b) if visible && Some(id) == active && b.width >= 1.0 && b.height >= 1.0);
        if let (true, Some(b)) = (show, bounds) {
            let _ = webview.set_position(LogicalPosition::new(b.x, b.y));
            let _ = webview.set_size(LogicalSize::new(b.width, b.height));
            let _ = webview.show();
        } else {
            let _ = webview.hide();
        }
        let mut state = lock();
        if let Some(tab) = state.tabs.iter_mut().find(|t| t.id == id) {
            hid_shown |= tab.shown && !show;
            tab.shown = show;
        }
    }
    // A hidden page keeps WebView2's keyboard focus, and the app UI then ignores typing.
    if hid_shown {
        focus_main();
    }
}

/// Gives the keyboard back to the app UI (webview "main").
pub fn focus_main() {
    if let Some(main) = APP.get().and_then(|app| app.get_webview("main")) {
        let _ = main.set_focus();
    }
}

/// Accepts `example.com`, `https://…`; only http(s) and about:blank are allowed.
pub fn normalize_url(raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Thiếu địa chỉ trang".into());
    }
    if raw == "about:blank" {
        return Url::parse(raw).map_err(|e| e.to_string());
    }
    let candidate = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let url = Url::parse(&candidate).map_err(|_| format!("Địa chỉ không hợp lệ: {raw}"))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => Err(format!("Không mở được địa chỉ {other}: (chỉ hỗ trợ http/https)")),
    }
}

fn update_tab(id: u32, change: impl FnOnce(&mut Tab)) {
    let changed = {
        let mut state = lock();
        match state.tabs.iter_mut().find(|t| t.id == id) {
            Some(tab) => {
                change(tab);
                true
            }
            None => false,
        }
    };
    if changed {
        publish(false);
    }
}

async fn create_tab(url: Url) -> Result<u32, String> {
    let app = app()?;
    let window = app.get_window("main").ok_or("Không tìm thấy cửa sổ chính")?;
    let (id, bounds) = {
        let mut state = lock();
        if state.tabs.len() >= MAX_TABS {
            return Err(format!("Đã mở tối đa {MAX_TABS} tab; hãy đóng bớt tab"));
        }
        let id = state.next;
        state.next += 1;
        state.tabs.push(Tab { id, title: "Tab mới".into(), url: url.to_string(), loading: true, shown: false });
        state.active = Some(id);
        (id, state.bounds)
    };
    // Not focused: a new tab must not take the keyboard from the chat box (the agent opens
    // tabs while the user may be typing). Clicking into the page focuses it as usual.
    let builder = WebviewBuilder::new(label(id), WebviewUrl::External(url))
        .focused(false)
        .on_page_load(move |_, payload| {
            let url = payload.url().to_string();
            let loading = matches!(payload.event(), PageLoadEvent::Started);
            update_tab(id, |tab| {
                tab.url = url;
                tab.loading = loading;
            });
        })
        .on_document_title_changed(move |_, title| update_tab(id, |tab| tab.title = title))
        .on_new_window(|url, _| {
            // Links with target=_blank / window.open become a new tab in this browser.
            tauri::async_runtime::spawn(open_popup(url));
            NewWindowResponse::Deny
        });
    let (position, size) = match bounds {
        Some(b) => (LogicalPosition::new(b.x, b.y), LogicalSize::new(b.width.max(1.0), b.height.max(1.0))),
        None => (LogicalPosition::new(0.0, 0.0), LogicalSize::new(1.0, 1.0)),
    };
    // add_child blocks on the main thread; run it off the async workers.
    let result = tauri::async_runtime::spawn_blocking(move || window.add_child(builder, position, size).map(|_| ()))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r.map_err(|e| e.to_string()));
    if let Err(error) = result {
        let mut state = lock();
        state.tabs.retain(|t| t.id != id);
        state.active = state.tabs.last().map(|t| t.id);
        return Err(format!("Không tạo được tab: {error}"));
    }
    layout();
    publish(false);
    Ok(id)
}

/// Boxed as `dyn Send` so the popup handler inside `create_tab` does not make its future recursive.
fn open_popup(url: Url) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        if create_tab(url).await.is_ok() {
            publish(true);
        }
    })
}

fn active_id() -> Result<u32, String> {
    lock().active.ok_or_else(|| "Chưa có tab nào đang mở; hãy dùng browser_open trước".to_string())
}

fn is_loading(id: u32) -> bool {
    lock().tabs.iter().find(|t| t.id == id).map(|t| t.loading).unwrap_or(false)
}

/// Waits for the tab to finish loading (best effort; slow pages are read as they are).
async fn wait_loaded(id: u32) {
    let deadline = tokio::time::Instant::now() + LOAD_TIMEOUT;
    while is_loading(id) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    // Let scripts render a little after the load event.
    tokio::time::sleep(Duration::from_millis(400)).await;
}

async fn navigate(id: u32, url: Url) -> Result<(), String> {
    let webview = app()?.get_webview(&label(id)).ok_or("Tab đã bị đóng")?;
    update_tab(id, |tab| tab.loading = true);
    webview.navigate(url).map_err(|e| e.to_string())?;
    wait_loaded(id).await;
    Ok(())
}

/// Opens `url` in the active tab, or a new tab when asked or when no tab exists.
pub async fn open(raw: &str, new_tab: bool) -> Result<u32, String> {
    let url = normalize_url(raw)?;
    let current = lock().active;
    let id = match current {
        Some(id) if !new_tab => {
            navigate(id, url).await?;
            id
        }
        _ => {
            let id = create_tab(url).await?;
            wait_loaded(id).await;
            id
        }
    };
    Ok(id)
}

/// Runs a script in the tab and returns its JSON result.
async fn eval(id: u32, script: String) -> Result<Value, String> {
    let webview = app()?.get_webview(&label(id)).ok_or("Tab đã bị đóng")?;
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = Mutex::new(Some(tx));
    webview
        .eval_with_callback(script, move |result| {
            if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
                let _ = tx.send(result);
            }
        })
        .map_err(|e| e.to_string())?;
    let raw = tokio::time::timeout(SCRIPT_TIMEOUT, rx)
        .await
        .map_err(|_| "Trang không phản hồi (quá 10 giây)".to_string())?
        .map_err(|_| "Trang đã đóng trong lúc đọc".to_string())?;
    Ok(serde_json::from_str(&raw).unwrap_or(Value::String(raw)))
}

/// Collects visible text and numbers the interactive elements with `data-lim-id`.
const READ_SCRIPT: &str = r#"(() => {
  const visible = (el) => { const r = el.getBoundingClientRect(); const s = getComputedStyle(el); return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none'; };
  const clean = (t) => (t || '').replace(/\s+/g, ' ').trim();
  document.querySelectorAll('[data-lim-id]').forEach((el) => el.removeAttribute('data-lim-id'));
  const items = []; let n = 0;
  const selector = 'a[href], button, input:not([type=hidden]), textarea, select, [role=button], [role=link], [role=tab], [role=menuitem], [role=checkbox], [contenteditable=true]';
  // Main content first (search results, article links), page chrome after it.
  const main = document.querySelector('main, [role=main], #search, #content, article');
  const all = [...document.querySelectorAll(selector)];
  const ordered = main ? [...all.filter((el) => main.contains(el)), ...all.filter((el) => !main.contains(el))] : all;
  for (const el of ordered) {
    if (items.length >= 60) break;
    if (!visible(el) || el.disabled) continue;
    const tag = el.tagName.toLowerCase();
    const field = tag === 'input' || tag === 'textarea' || tag === 'select';
    const label = clean(field ? (el.getAttribute('aria-label') || el.placeholder || el.name || el.title) : (el.innerText || el.getAttribute('aria-label') || el.title || (el.querySelector('img') || {}).alt)).slice(0, 80);
    if (!label && !field) continue;
    n += 1; el.setAttribute('data-lim-id', String(n));
    items.push({ i: n, tag, type: field ? (el.type || tag) : (el.getAttribute('role') || ''), label, value: field ? clean(el.value).slice(0, 60) : '', href: tag === 'a' ? el.href : '' });
  }
  const text = clean((document.body ? document.body.innerText : '').replace(/\n\s*\n+/g, '\n§')).replace(/ ?§ ?/g, '\n');
  return { url: location.href, title: document.title, text, scroll: Math.round(scrollY), height: document.documentElement.scrollHeight, viewport: innerHeight, items };
})()"#;

fn describe(page: &Value, cap: usize) -> String {
    let mut items = String::new();
    for item in page["items"].as_array().into_iter().flatten() {
        let tag = item["tag"].as_str().unwrap_or_default();
        let kind = match tag {
            "a" => "link".to_string(),
            "input" | "textarea" | "select" => format!("ô nhập({})", item["type"].as_str().unwrap_or(tag)),
            _ => "nút".to_string(),
        };
        let mut line = format!("[{}] {kind} \"{}\"", item["i"], item["label"].as_str().unwrap_or_default());
        if let Some(value) = item["value"].as_str().filter(|v| !v.is_empty()) {
            line.push_str(&format!(" = \"{value}\""));
        }
        if let Some(href) = item["href"].as_str().filter(|h| !h.is_empty() && !h.starts_with("javascript:")) {
            line.push_str(&format!(" -> {}", short_href(href)));
        }
        items.push_str(&line);
        items.push('\n');
    }
    let header = format!(
        "Trang: {}\nURL: {}\nVị trí cuộn: {} / {} px\n",
        page["title"].as_str().unwrap_or_default(),
        page["url"].as_str().unwrap_or_default(),
        page["scroll"],
        page["height"]
    );
    // Page text gets most of the budget (it answers questions); the element list keeps
    // whole lines from the top of the page and says how many were left out.
    let room = cap.saturating_sub(header.len() + 160) * 3 / 5;
    let text = crate::agent::truncate(page["text"].as_str().unwrap_or_default(), room);
    let items = whole_lines(&items, cap.saturating_sub(header.len() + text.len() + 160));
    format!("{header}--- Nội dung ---\n{text}\n--- Phần tử tương tác (dùng số [i] cho browser_click / browser_type) ---\n{items}")
}

/// Long tracking URLs (search engines) waste the model's context: keep at most 100 chars.
fn short_href(href: &str) -> String {
    let href = href.replace(['\n', ' '], "");
    if href.chars().count() <= 100 { href } else { format!("{}…", href.chars().take(100).collect::<String>()) }
}

fn whole_lines(lines: &str, budget: usize) -> String {
    let total = lines.lines().count();
    let mut out = String::new();
    let mut kept = 0;
    for line in lines.lines() {
        if out.len() + line.len() + 1 > budget {
            break;
        }
        out.push_str(line);
        out.push('\n');
        kept += 1;
    }
    if kept < total {
        out.push_str(&format!("…(còn {} phần tử nữa; cuộn trang hoặc mở trang con để xem tiếp)\n", total - kept));
    }
    out
}

async fn read(id: u32, cap: usize) -> Result<String, String> {
    let page = eval(id, READ_SCRIPT.to_string()).await?;
    if !page.is_object() {
        return Err(format!("Không đọc được trang: {page}"));
    }
    Ok(describe(&page, cap))
}

async fn element_action(id: u32, index: i64, body: &str) -> Result<(), String> {
    let script = format!(
        "(() => {{ const el = document.querySelector('[data-lim-id=\"{index}\"]'); if (!el) return 'missing'; el.scrollIntoView({{block:'center'}}); {body} return 'ok'; }})()"
    );
    match eval(id, script).await? {
        Value::String(s) if s == "ok" => Ok(()),
        _ => Err(format!("Không thấy phần tử [{index}] trên trang hiện tại; hãy gọi browser_read để lấy lại danh sách")),
    }
}

/// Clicks the link found by `finder` (a JS expression) in the same tab and waits for the page.
async fn follow_link(id: u32, finder: &str) -> Result<(), String> {
    let script = format!(
        "(() => {{ const el = {finder}; const a = el && el.closest('a[href]'); if (!a || !/^https?:/.test(a.href)) return ''; a.removeAttribute('target'); a.scrollIntoView({{block:'center'}}); a.click(); return 'ok'; }})()"
    );
    match eval(id, script).await? {
        Value::String(s) if s == "ok" => {
            settle(id).await;
            Ok(())
        }
        _ => Err("no link".into()),
    }
}

/// Search-engine redirect links ("google.com/goto?url=…", "google.com/url?q=…").
pub fn is_redirect_link(raw: &str) -> bool {
    Url::parse(raw).is_ok_and(|u| {
        u.host_str().is_some_and(|h| h.contains("google.")) && matches!(u.path(), "/goto" | "/url")
    })
}

/// The real address inside a redirect link when it carries one in plain text (`/url?q=https://…`).
fn redirect_target(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    url.query_pairs()
        .find(|(key, value)| matches!(key.as_ref(), "q" | "url") && value.starts_with("http"))
        .map(|(_, value)| value.into_owned())
}

/// After an action that may navigate, wait for a load that started and settle.
async fn settle(id: u32) {
    tokio::time::sleep(Duration::from_millis(600)).await;
    wait_loaded(id).await;
}

fn search_url(query: &str) -> Result<Url, String> {
    let mut url = Url::parse("https://www.google.com/search").map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("q", query).append_pair("hl", "vi");
    Ok(url)
}

/// Agent tool entry point. Every action returns the resulting page so the model can continue.
pub async fn run_tool(tool: &str, args: &Value, cap: usize) -> Result<String, String> {
    let text = |key: &str| args[key].as_str().unwrap_or_default().trim().to_string();
    let index = || args["index"].as_i64().or_else(|| args["index"].as_str().and_then(|s| s.trim().trim_matches(['[', ']']).parse().ok()));
    let id = match tool {
        "browser_open" => {
            let new_tab = args["new_tab"].as_bool().unwrap_or(false);
            let url = text("url");
            // Links on a results page are often redirects that only work when clicked from that
            // page (Google's /goto?url=… answers 400 when opened directly), so a link of the
            // open page is followed by clicking it, like a person would.
            let id = match index() {
                Some(index) if url.is_empty() => {
                    let id = active_id()?;
                    follow_link(id, &format!("document.querySelector('[data-lim-id=\"{index}\"]')")).await.map_err(|_| {
                        format!("Phần tử [{index}] không phải link mở được; hãy gọi browser_read để xem lại danh sách, hoặc dùng browser_click")
                    })?;
                    id
                }
                _ => {
                    let current = lock().active;
                    let clicked = match current {
                        Some(id) if !new_tab || is_redirect_link(&url) => {
                            let wanted = serde_json::to_string(&url).map_err(|e| e.to_string())?;
                            follow_link(id, &format!("[...document.querySelectorAll('a[href]')].find((a) => a.href === {wanted})")).await.ok().map(|()| id)
                        }
                        _ => None,
                    };
                    match clicked {
                        Some(id) => id,
                        None if is_redirect_link(&url) => match redirect_target(&url) {
                            Some(target) => open(&target, new_tab).await?,
                            None => return Err("Đây là link chuyển hướng của trang kết quả tìm kiếm, không mở trực tiếp được. Hãy dùng browser_open với index [i] của kết quả trên trang đang mở.".into()),
                        },
                        None => open(&url, new_tab).await?,
                    }
                }
            };
            publish(true);
            id
        }
        "browser_search" => {
            let query = text("query");
            if query.is_empty() {
                return Err("Thiếu từ khóa tìm kiếm".into());
            }
            let id = open(search_url(&query)?.as_str(), false).await?;
            publish(true);
            id
        }
        "browser_read" => active_id()?,
        "browser_click" => {
            let id = active_id()?;
            let index = index().ok_or("Thiếu số thứ tự phần tử (index)")?;
            element_action(id, index, "el.click();").await?;
            settle(id).await;
            active_id().unwrap_or(id)
        }
        "browser_type" => {
            let id = active_id()?;
            let index = index().ok_or("Thiếu số thứ tự phần tử (index)")?;
            let value = serde_json::to_string(args["text"].as_str().unwrap_or_default()).map_err(|e| e.to_string())?;
            let submit = args["submit"].as_bool().unwrap_or(false);
            let body = format!(
                "el.focus(); if (el.isContentEditable) {{ el.textContent = {value}; }} else {{ const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : el.tagName === 'SELECT' ? HTMLSelectElement.prototype : HTMLInputElement.prototype; Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, {value}); }} \
                 el.dispatchEvent(new Event('input', {{bubbles:true}})); el.dispatchEvent(new Event('change', {{bubbles:true}})); \
                 if ({submit}) {{ const opts = {{key:'Enter', code:'Enter', keyCode:13, which:13, bubbles:true}}; el.dispatchEvent(new KeyboardEvent('keydown', opts)); el.dispatchEvent(new KeyboardEvent('keyup', opts)); if (el.form) {{ el.form.requestSubmit ? el.form.requestSubmit() : el.form.submit(); }} }}"
            );
            element_action(id, index, &body).await?;
            if submit {
                settle(id).await;
            }
            id
        }
        "browser_scroll" => {
            let id = active_id()?;
            let direction = if text("direction") == "up" { -1 } else { 1 };
            eval(id, format!("window.scrollBy(0, {direction} * innerHeight * 0.8); 'ok'")).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            id
        }
        "browser_back" => {
            let id = active_id()?;
            update_tab(id, |tab| tab.loading = true);
            eval(id, "history.back(); 'ok'".to_string()).await?;
            settle(id).await;
            // history.back() on the first page never loads; don't wait forever on the flag.
            update_tab(id, |tab| tab.loading = false);
            id
        }
        "browser_tabs" => return tabs_tool(&text("action"), args["id"].as_u64()),
        other => return Err(format!("Công cụ {other} không tồn tại")),
    };
    read(id, cap).await
}

fn tabs_tool(action: &str, id: Option<u64>) -> Result<String, String> {
    match action {
        "switch" | "close" => {
            let id = id.ok_or("Thiếu id tab")? as u32;
            if action == "switch" { select(id)? } else { close(id)? }
            publish(true);
        }
        _ => {}
    }
    let state = lock();
    if state.tabs.is_empty() {
        return Ok("Chưa có tab nào.".into());
    }
    Ok(state
        .tabs
        .iter()
        .map(|t| format!("{}tab {}: {} — {}", if Some(t.id) == state.active { "* " } else { "  " }, t.id, t.title, t.url))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Open tabs for the agent's context, one per line; "" when the browser is unused.
pub fn tab_lines() -> String {
    let state = lock();
    state
        .tabs
        .iter()
        .map(|t| format!("{}tab {}: {} — {}", if Some(t.id) == state.active { "* " } else { "  " }, t.id, t.title, t.url))
        .collect::<Vec<_>>()
        .join("\n")
}

fn select(id: u32) -> Result<(), String> {
    {
        let mut state = lock();
        if !state.tabs.iter().any(|t| t.id == id) {
            return Err(format!("Không có tab {id}"));
        }
        state.active = Some(id);
    }
    layout();
    Ok(())
}

fn close(id: u32) -> Result<(), String> {
    {
        let mut state = lock();
        let Some(position) = state.tabs.iter().position(|t| t.id == id) else {
            return Err(format!("Không có tab {id}"));
        };
        state.tabs.remove(position);
        if state.active == Some(id) {
            state.active = state.tabs.get(position.min(state.tabs.len().saturating_sub(1))).map(|t| t.id);
        }
    }
    if let Some(webview) = app()?.get_webview(&label(id)) {
        let _ = webview.close();
    }
    layout();
    focus_main();
    Ok(())
}

/// Debug smoke test (`LIM_BROWSER_SMOKE=<output file>`): drives the agent tools against real
/// pages once the window is up and writes what the model would see to the file.
#[cfg(debug_assertions)]
pub fn smoke(output: std::path::PathBuf) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(4)).await;
        let mut log = format!("before any tab, main has keyboard focus: {}\n", main_focused().await);
        let steps: [(&str, Value); 7] = [
            ("browser_open", json!({"url": "example.com"})),
            ("browser_open", json!({"index": 1})),
            ("browser_search", json!({"query": "thời tiết Đà Nẵng hôm nay"})),
            ("browser_open", json!({"url": "https://vi.wikipedia.org/wiki/Đà_Nẵng", "new_tab": true})),
            ("browser_scroll", json!({"direction": "down"})),
            ("browser_tabs", json!({"action": "switch", "id": 1})),
            ("browser_click", json!({"index": 1})),
        ];
        for (tool, args) in steps {
            let started = std::time::Instant::now();
            let result = run_tool(tool, &args, 3_000).await;
            let result = match result { Ok(t) => t, Err(e) => format!("ERROR: {e}") };
            log.push_str(&format!("===== {tool} {args} ({} ms)\n{result}\n\n", started.elapsed().as_millis()));
            log.push_str(&format!("main has keyboard focus: {}\n\n", main_focused().await));
            let _ = std::fs::write(&output, &log);
        }
        // Google results are redirect links (/goto?url=…) that fail when opened directly:
        // open the first result by index and by its URL; both must land off google.com.
        let _ = run_tool("browser_search", &json!({"query": "Manchester United trang chủ"}), 3_000).await;
        if let Ok(id) = active_id() {
            let first = "(() => { const a = document.querySelector('a h3'); const link = a && a.closest('a'); return link ? [link.getAttribute('data-lim-id') || '', link.href] : ['', '']; })()";
            let found = eval(id, first.to_string()).await.unwrap_or(Value::Null);
            let index = found[0].as_str().and_then(|i| i.parse::<i64>().ok());
            let href = found[1].as_str().unwrap_or_default().to_string();
            log.push_str(&format!("first Google result: index {index:?}, href {}\n", href.chars().take(80).collect::<String>()));
            let by_index = run_tool("browser_open", &json!({"index": index}), 600).await;
            log.push_str(&format!("open by index -> {}\n", by_index.unwrap_or_else(|e| format!("ERROR: {e}")).lines().take(2).collect::<Vec<_>>().join(" | ")));
            let _ = run_tool("browser_back", &json!({}), 600).await;
            let _ = run_tool("browser_read", &json!({}), 600).await;
            let by_url = run_tool("browser_open", &json!({"url": href}), 600).await;
            log.push_str(&format!("open by url -> {}\n\n", by_url.unwrap_or_else(|e| format!("ERROR: {e}")).lines().take(2).collect::<Vec<_>>().join(" | ")));
            let _ = std::fs::write(&output, &log);
        }
        // The user clicks into the page (it takes the keyboard), then clicks back into the app.
        if let Some(page) = lock().active.and_then(|id| APP.get().and_then(|app| app.get_webview(&label(id)))) {
            let _ = page.set_focus();
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        log.push_str(&format!("after focusing the page, main has keyboard focus: {}\n", main_focused().await));
        browser_focus_main().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        log.push_str(&format!("after a click in the app, main has keyboard focus: {}\n", main_focused().await));
        log.push_str("===== done\n");
        let _ = std::fs::write(&output, &log);
    });
}

/// Whether the app UI (webview "main") holds keyboard focus, for the smoke test.
#[cfg(debug_assertions)]
async fn main_focused() -> String {
    let Some(main) = APP.get().and_then(|app| app.get_webview("main")) else { return "no main".into() };
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = Mutex::new(Some(tx));
    let _ = main.eval_with_callback("document.hasFocus()", move |r| {
        if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
            let _ = tx.send(r);
        }
    });
    tokio::time::timeout(SCRIPT_TIMEOUT, rx).await.ok().and_then(|r| r.ok()).unwrap_or_else(|| "timeout".into())
}

// ----- Commands for the browser panel -----

#[tauri::command]
pub async fn browser_state() -> Value {
    let state = lock();
    json!({"tabs": state.tabs, "active": state.active, "reveal": false})
}

#[tauri::command]
pub async fn browser_set_bounds(bounds: Bounds, visible: bool) {
    {
        let mut state = lock();
        state.bounds = Some(bounds);
        state.visible = visible;
    }
    layout();
}

/// The UI calls this on any click in the app: the click itself does not move WebView2's
/// keyboard focus away from a browser page.
#[tauri::command]
pub async fn browser_focus_main() {
    if !lock().tabs.is_empty() {
        focus_main();
    }
}

#[tauri::command]
pub async fn browser_open(url: String, new_tab: bool) -> Result<(), String> {
    let url = normalize_url(&url).or_else(|_| search_url(&url))?;
    let current = lock().active;
    match current {
        Some(id) if !new_tab => {
            let webview = app()?.get_webview(&label(id)).ok_or("Tab đã bị đóng")?;
            update_tab(id, |tab| tab.loading = true);
            webview.navigate(url).map_err(|e| e.to_string())?;
        }
        _ => {
            create_tab(url).await?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_nav(action: String) -> Result<(), String> {
    let id = active_id()?;
    let webview = app()?.get_webview(&label(id)).ok_or("Tab đã bị đóng")?;
    match action.as_str() {
        "back" => webview.eval("history.back()"),
        "forward" => webview.eval("history.forward()"),
        _ => webview.reload(),
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_select(id: u32) -> Result<(), String> {
    select(id)?;
    publish(false);
    Ok(())
}

#[tauri::command]
pub async fn browser_close(id: u32) -> Result<(), String> {
    close(id)?;
    publish(false);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_normalized_and_restricted() {
        assert_eq!(normalize_url("example.com").unwrap().as_str(), "https://example.com/");
        assert_eq!(normalize_url(" http://a.vn/x ").unwrap().as_str(), "http://a.vn/x");
        assert!(normalize_url("file:///C:/Windows/win.ini").is_err());
        assert!(normalize_url("javascript:alert(1)").is_err());
        assert!(normalize_url("").is_err());
        assert!(is_redirect_link("https://www.google.com/goto?url=CAESVQHEIkXXmMgm"));
        assert!(is_redirect_link("https://www.google.com/url?q=https://www.manutd.com/&sa=U"));
        assert!(!is_redirect_link("https://www.manutd.com/"));
        assert_eq!(redirect_target("https://www.google.com/url?q=https://www.manutd.com/&sa=U").as_deref(), Some("https://www.manutd.com/"));
        assert_eq!(redirect_target("https://www.google.com/goto?url=CAESVQHEIkXXmMgm"), None);
        assert!(search_url("giá vàng hôm nay").unwrap().as_str().starts_with("https://www.google.com/search?q=gi%C3%A1"));
    }

    #[test]
    fn page_description_keeps_elements_within_budget() {
        let items: Vec<Value> = (1..=40).map(|i| json!({"i": i, "tag": "a", "label": format!("Liên kết {i}"), "href": format!("https://example.com/{i}")})).collect();
        let page = json!({"url": "https://example.com", "title": "Ví dụ", "text": "nội dung ".repeat(2_000), "scroll": 0, "height": 3000, "items": items});
        let text = describe(&page, 4_000);
        assert!(text.len() < 4_200, "{}", text.len());
        assert!(text.matches("nội dung").count() > 150, "page text should get most of the budget");
        assert!(text.contains("…(còn "), "{text}");
        assert!(text.contains("Trang: Ví dụ"));
        assert!(text.contains("[1] link \"Liên kết 1\" -> https://example.com/1"));
        let long = format!("https://www.google.com/search?q={}", "x".repeat(300));
        assert_eq!(short_href(&long).chars().count(), 101);
        let field = json!({"url": "u", "title": "t", "text": "", "scroll": 0, "height": 0, "items": [{"i": 1, "tag": "input", "type": "search", "label": "Tìm", "value": "abc", "href": ""}]});
        assert!(describe(&field, 2_000).contains("[1] ô nhập(search) \"Tìm\" = \"abc\""));
    }
}
