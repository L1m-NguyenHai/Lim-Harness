import { useEffect, useMemo, useRef, useState } from "react";
import Logo from "./Logo";
import Cookbook from "./Cookbook";
import FloatingWindow from "./FloatingWindow";
import type { WindowMode } from "./FloatingWindow";
import { core, native, defaultInference } from "./core";
import type { InferenceSettings, RunUsage } from "./core";
import { InferencePanel } from "./InferencePanel";
import { ContextMeter } from "./ContextMeter";
import { ThinkingSteps } from "./ThinkingSteps";
import { thinkingSteps } from "./thinking-steps";
import { contextUsage } from "./context-usage";
import type { VaultHit } from "./core";
import {
  ArrowUp,
  AudioLines,
  BookOpen,
  Check,
  ChevronDown,
  Cpu,
  FileText,
  Layers3,
  MessageSquare,
  Moon,
  PanelLeftClose,
  PanelLeftOpen,
  Paperclip,
  Plus,
  Search,
  Settings2,
  ShieldCheck,
  Smile,
  Sun,
  X,
  Copy,
  Square,
  Trash2,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { moduleCatalog, newSession, readState, STORAGE_KEY } from "./state";
import type { Note, Page, SessionConfig, State } from "./state";

const navigation: { id: Page; label: string; icon: LucideIcon }[] = [
  { id: "chat", label: "Hội thoại", icon: MessageSquare },
  { id: "memory", label: "Bộ nhớ", icon: BookOpen },
  { id: "cookbook", label: "Cookbook", icon: Cpu },
  { id: "modules", label: "Module", icon: Layers3 },
];
const moduleIcons: Record<string, LucideIcon> = {
  text: FileText,
  sentiment: Smile,
  privacy: ShieldCheck,
  audio: AudioLines,
};
type ToolPage = Exclude<Page, "chat">;
const toolTitles: Record<ToolPage, string> = { cookbook: "Cookbook", memory: "Bộ nhớ", modules: "Module", settings: "Cài đặt" };
const formatDate = (value: string) =>
  new Intl.DateTimeFormat("vi-VN", {
    day: "2-digit",
    month: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    timeZone: "Asia/Bangkok",
  }).format(new Date(value));

// Tách <think>...</think> mà một số model nhúng trong content thành reasoning riêng.
function splitThink(text: string): { thinking: string; answer: string } {
  const match = text.match(/<think>([\s\S]*?)<\/think>/i);
  if (!match) return { thinking: "", answer: text };
  return {
    thinking: match[1].trim(),
    answer: (text.slice(0, match.index) + text.slice((match.index ?? 0) + match[0].length)).trim(),
  };
}
// Live per-phase token estimate while a run streams (Ollama sends ~1 token per chunk).
function liveUsage(meta: { startedAt: number; chars: number; usage?: RunUsage }): string {
  const u = meta.usage;
  const seconds = (Date.now() - meta.startedAt) / 1000;
  const parts: string[] = [];
  if (u && u.thinkingBudget > 0) parts.push(`Suy nghĩ ~${u.thinkingTokens}/${u.thinkingBudget}${u.thinkingCapped ? " (đã dừng)" : ""}`);
  if (u) parts.push(`Trả lời ~${u.answerTokens}/${u.answerBudget} token`);
  const total = u ? u.thinkingTokens + u.answerTokens : meta.chars / 4;
  if (seconds > 0.5 && total > 0) parts.push(`${(total / seconds).toFixed(1)} token/s`);
  return parts.length ? `${parts.join(" · ")} · ước tính…` : "Đang chạy…";
}
function formatMetrics(m?: import("./state").ChatMetrics): string {
  if (!m) return "";
  const parts: string[] = [];
  if (m.tokensPerSecond != null && Number.isFinite(m.tokensPerSecond))
    parts.push(`${m.tokensPerSecond.toFixed(1)} token/s`);
  if (m.thinkingTokens) parts.push(`suy nghĩ ~${m.thinkingTokens}${m.thinkingCapped ? " (chạm giới hạn)" : ""}`);
  if (m.answerTokens != null && m.thinkingTokens) parts.push(`trả lời ~${m.answerTokens}`);
  if (m.outputTokens != null) parts.push(`${m.outputTokens} token`);
  const ms = m.totalDurationMs ?? m.evalDurationMs;
  if (ms != null) parts.push(ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`);
  return parts.join(" · ");
}

export default function App() {
  const [loaded] = useState(() => {
    const loaded = readState();
    try { if (!localStorage.getItem("lim-minimal-style-v1")) loaded.state.preferences.theme = "light"; } catch { /* Keep app usable without browser storage. */ }
    return loaded;
  });
  const [state, setState] = useState<State>(loaded.state);
  const [page, setFocusedPage] = useState<Page>("chat");
  const [windows, setWindows] = useState<Record<ToolPage, WindowMode>>({ cookbook: "closed", memory: "closed", modules: "closed", settings: "closed" });
  const [windowOrder, setWindowOrder] = useState<ToolPage[]>(["memory", "modules", "settings", "cookbook"]);
  const [collapsed, setCollapsed] = useState(false);
  const [draft, setDraft] = useState("");
  const [notice, setNotice] = useState("");
  const [storageError, setStorageError] = useState(loaded.error);
  const [query, setQuery] = useState("");
  const [editor, setEditor] = useState<Note | null>(null);
  const [noteTitle, setNoteTitle] = useState("");
  const [noteContent, setNoteContent] = useState("");
  const [attachment, setAttachment] = useState<{
    name: string;
    text: string;
  } | null>(null);
  const [ready, setReady] = useState(!native);
  const [models, setModels] = useState<string[]>([]);
  const [modelError, setModelError] = useState("");
  const [runtimeError, setRuntimeError] = useState("");
  const [running, setRunning] = useState<Record<string, string>>({});
  const [pendingDelete, setPendingDelete] = useState("");
  const [streamMeta, setStreamMeta] = useState<Record<string, { startedAt: number; chars: number; usage?: RunUsage }>>({});
  const runLock = useRef(new Set<string>());
  const [vaultPath, setVaultPath] = useState("");
  const [vaultDraft, setVaultDraft] = useState("");
  const [memoryQuery, setMemoryQuery] = useState("");
  const [memoryHits, setMemoryHits] = useState<VaultHit[]>([]);
  const [memoryStatus, setMemoryStatus] = useState("");
  const [memoryBusy, setMemoryBusy] = useState(false);
  const [contextSize, setContextSize] = useState(() => {
    if (native) return 4096;
    try { const size = JSON.parse(localStorage.getItem("lim-inference-preview") || "{}").contextSize; return [4096,8192,16384].includes(size) ? size : 4096; } catch { return 4096; }
  });
  const [inferenceOpen, setInferenceOpen] = useState(() => { try { return localStorage.getItem("lim-inference-open") !== "false"; } catch { return true; } });
  const [inferenceDirty, setInferenceDirty] = useState(false);
  const [inferenceSettings, setInferenceSettings] = useState<InferenceSettings>(() => {
    if (native) return defaultInference;
    try { const { contextSize: _context, ...saved } = JSON.parse(localStorage.getItem("lim-inference-preview") || "{}"); return { ...defaultInference, ...saved }; }
    catch { return defaultInference; }
  });
  const searchDialog = useRef<HTMLDialogElement>(null);
  const noteDialog = useRef<HTMLDialogElement>(null);
  const composer = useRef<HTMLTextAreaElement>(null);
  const messageEnd = useRef<HTMLDivElement>(null);
  const session = state.sessions.find((s) => s.id === state.active)!;
  const selectedModules = state.preferences.modules.length;
  // Run settings of the open session; everything in the panel follows the selected session.
  // Merged over full values so configs saved by older versions (only some fields) stay complete.
  const effective: SessionConfig = { ...inferenceSettings, contextSize, ...session.config };
  const effectiveKey = JSON.stringify(effective);
  // Keyed by value so a refreshed snapshot with identical settings does not reset the panel draft.
  const panelSettings = useMemo<InferenceSettings>(() => { const { contextSize: _context, ...settings } = JSON.parse(effectiveKey) as SessionConfig; return settings; }, [effectiveKey]);
  function applySessionConfig(id: string, config: SessionConfig) {
    setState((c) => ({ ...c, sessions: c.sessions.map((s) => s.id === id ? { ...s, config } : s) }));
  }
  const activeSession = useRef(state.active);
  activeSession.current = state.active;
  const busy = !!running[session.id];
  function focusWindow(id: ToolPage) {
    setFocusedPage(id);
    setWindowOrder((current) => current.at(-1) === id ? current : [...current.filter((item) => item !== id), id]);
  }
  function setPage(id: Page) {
    if (id === "chat") {
      setFocusedPage("chat");
      setWindows((current) => Object.fromEntries(Object.entries(current).map(([key, mode]) => [key, mode === "open" ? "minimized" : mode])) as Record<ToolPage, WindowMode>);
      return;
    }
    setWindows((current) => ({ ...current, [id]: "open" }));
    focusWindow(id);
  }
  function windowProps(id: ToolPage) {
    return { id, title: toolTitles[id], mode: windows[id], zIndex: 100 + windowOrder.indexOf(id),
      onFocus: () => focusWindow(id),
      onClose: () => { setWindows((current) => ({ ...current, [id]: "closed" })); setFocusedPage("chat"); },
      onMinimize: () => { setWindows((current) => ({ ...current, [id]: "minimized" })); setFocusedPage("chat"); },
    };
  }

  useEffect(() => {
    if (!native) return;
    let alive = true;
    async function loadCore() {
      try {
        let snapshot = await core.snapshot();
        if (!snapshot.sessions.length) {
          await core.importSessions(loaded.state.sessions);
          snapshot = await core.snapshot();
        }
        if (!snapshot.sessions.length) snapshot.sessions = [await core.createSession()];
        if (!alive) return;
        setState((c) => ({ ...c, sessions: snapshot.sessions,
          active: snapshot.sessions.some((s) => s.id === c.active) ? c.active : snapshot.sessions[0].id,
          preferences: { ...c.preferences, provider: "ollama", model: snapshot.model },
        }));
        setVaultPath(snapshot.vaultPath);
        setVaultDraft(snapshot.vaultPath);
        setContextSize(snapshot.contextSize);
        setInferenceSettings(snapshot.inferenceSettings);
        setReady(true);
        void refreshModels();
      } catch (error) { if (alive) setRuntimeError(String(error)); }
    }
    void loadCore();
    return () => { alive = false; };
  }, [loaded]);

  async function refreshModels() {
    setModelError("");
    try {
      const names = await core.models();
      setModels(names);
      if (!names.length) setModelError("Ollama chưa có model. Chạy ollama pull <tên-model>, rồi tải lại danh sách.");
    } catch (error) { setModelError(String(error)); }
  }
  async function chooseModel(model: string) {
    try {
      await core.setModel(model);
      setState((c) => ({ ...c, preferences: { ...c.preferences, provider: "ollama", model } }));
    } catch (error) { setRuntimeError(String(error)); }
  }
  // Context picked in Cookbook/Settings applies to the open session only.
  async function chooseContext(size: number) {
    const config = { ...effective, contextSize: size };
    try {
      if (native) await core.setSessionConfig(session.id, config);
      applySessionConfig(session.id, config);
    } catch (error) { setRuntimeError(String(error)); }
  }
  useEffect(() => { try { localStorage.setItem("lim-inference-open", String(inferenceOpen)); } catch { /* UI remains usable without persistence. */ } }, [inferenceOpen]);
  // Every parameter in the panel is saved on the open session only; other sessions keep theirs.
  async function saveInference(settings: InferenceSettings, size: number) {
    const config = { ...settings, contextSize: size };
    if (native) await core.setSessionConfig(session.id, config);
    applySessionConfig(session.id, config);
  }
  async function connectVault(path: string) {
    setMemoryBusy(true);
    try {
      const canonical = await core.setVault(path);
      setVaultPath(canonical); setVaultDraft(canonical); setMemoryHits([]);
      setMemoryStatus(canonical ? "Đã kết nối vault. Ghi chú liên quan sẽ được tìm khi gửi tin nhắn." : "Đã ngắt vault.");
    } catch (error) { setMemoryStatus(String(error)); }
    finally { setMemoryBusy(false); }
  }
  async function searchVault() {
    if (!memoryQuery.trim()) return;
    setMemoryBusy(true);
    try {
      const result = await core.searchMemory(memoryQuery);
      setMemoryHits(result.hits);
      setMemoryStatus(`${result.hits.length} kết quả · ${result.scannedFiles} tệp đã đọc${result.truncated ? " · đã đạt giới hạn tìm kiếm" : ""}`);
    } catch (error) { setMemoryStatus(String(error)); }
    finally { setMemoryBusy(false); }
  }

  useEffect(() => {
    document.documentElement.dataset.theme = state.preferences.theme;
    if (loaded.error) return;
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
      localStorage.setItem("lim-minimal-style-v1", "1");
      setStorageError("");
    } catch {
      setStorageError(
        "Không lưu được dữ liệu cục bộ. Hãy giải phóng dung lượng hoặc cho phép lưu trữ; giữ cửa sổ mở để tránh mất thay đổi.",
      );
    }
  }, [state, loaded.error]);
  useEffect(() => {
    messageEnd.current?.scrollIntoView({ behavior: "instant", block: "end" });
  }, [session.id, session.messages, page]);
  useEffect(() => {
    if (page === "chat") composer.current?.focus({ preventScroll: true });
  }, [page, session.id]);
  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(""), 4000);
    return () => clearTimeout(timer);
  }, [notice]);
  useEffect(() => {
    function shortcut(event: KeyboardEvent) {
      if (
        !(event.ctrlKey || event.metaKey) ||
        !["k", "n"].includes(event.key.toLowerCase())
      )
        return;
      event.preventDefault();
      if (noteDialog.current?.open || searchDialog.current?.open) return;
      if (event.key.toLowerCase() === "k") openSearch();
      else createSession();
    }
    window.addEventListener("keydown", shortcut);
    return () => window.removeEventListener("keydown", shortcut);
  }, [ready]);
  function openSearch() {
    searchDialog.current?.showModal();
    searchDialog.current
      ?.querySelector<HTMLInputElement>("#search-input")
      ?.focus();
  }
  async function createSession() {
    if (!ready) return;
    let next;
    try { next = native ? await core.createSession() : { ...newSession(), config: { ...defaultInference, contextSize: 4096 } }; }
    catch (error) { setRuntimeError(String(error)); return; }
    setState((c) => ({
      ...c,
      active: next.id,
      sessions: [next, ...c.sessions],
    }));
    setDraft("");
    setAttachment(null);
    setPage("chat");
    composer.current?.focus();
  }
  // Two-step delete: the first click arms the button, the second removes the session.
  async function deleteSession(id: string) {
    if (!ready) return;
    if (running[id]) { setRuntimeError("Phiên đang chạy; dừng trả lời trước khi xoá."); return; }
    if (pendingDelete !== id) { setPendingDelete(id); return; }
    setPendingDelete("");
    try { if (native) await core.deleteSession(id); }
    catch (error) { setRuntimeError(String(error)); return; }
    let replacement = null;
    if (state.sessions.length === 1) {
      try { replacement = native ? await core.createSession() : newSession(); }
      catch (error) { setRuntimeError(String(error)); }
    }
    setState((c) => {
      const rest = c.sessions.filter((s) => s.id !== id);
      const sessions = replacement ? [replacement, ...rest] : rest;
      if (!sessions.length) return c;
      return { ...c, sessions, active: c.active === id ? sessions[0].id : c.active };
    });
    if (state.active === id) { setDraft(""); setAttachment(null); }
  }
  function openSession(id: string) {
    setState((c) => ({ ...c, active: id }));
    setDraft("");
    setAttachment(null);
    setPage("chat");
  }
  async function send() {
    const text = [
      draft.trim(),
      attachment ? `Tệp: ${attachment.name}\n${attachment.text}` : "",
    ]
      .filter(Boolean)
      .join("\n\n");
    if (!text || !ready || runLock.current.has(session.id)) return;
    if (!native) { setRuntimeError("Mở ứng dụng desktop bằng npm run tauri dev để dùng Ollama và vault."); return; }
    if (!state.preferences.model) { setPage("settings"); setModelError("Chọn một model Ollama trước khi gửi."); return; }
    if (new TextEncoder().encode(text).length > 8000) { setRuntimeError("Tin nhắn và tệp đính kèm tối đa 8.000 byte UTF-8. Hãy rút gọn nội dung."); return; }
    const sessionId = session.id;
    const userId = crypto.randomUUID();
    const assistantId = crypto.randomUUID();
    runLock.current.add(sessionId);
    setRunning((c) => ({ ...c, [sessionId]: "starting" }));
    setRuntimeError("");
    setState((c) => ({
      ...c,
      sessions: c.sessions.map((s) =>
        s.id === sessionId
          ? {
              ...s,
              title: s.messages.length
                ? s.title
                : (draft.trim() || attachment?.name || "Hội thoại").slice(
                    0,
                    48,
                  ),
              messages: [
                ...s.messages,
                { id: userId, role: "user", text, source: "user" },
                {
                  id: assistantId,
                  role: "assistant",
                  text: "",
                  source: "model",
                  status: "running",
                },
              ],
            }
          : s,
      ),
    }));
    let sequence = -1;
    let ended = false;
    let currentAssistantId: string = assistantId;
    const finish = () => {
      ended = true;
      runLock.current.delete(sessionId);
      setRunning((c) => { const next = { ...c }; delete next[sessionId]; return next; });
      setStreamMeta((c) => { const next = { ...c }; delete next[sessionId]; return next; });
    };
    try {
      await core.start(sessionId, text, state.preferences.model, (event) => {
        if (ended || event.sessionId !== sessionId || event.sequence <= sequence) return;
        sequence = event.sequence;
        if (event.type === "started" && activeSession.current === sessionId) { setDraft(""); setAttachment(null); }
        if (event.type === "started") setStreamMeta((c) => ({ ...c, [sessionId]: { startedAt: Date.now(), chars: 0 } }));
        if (event.type === "usage" && event.usage) { const usage = event.usage; setStreamMeta((c) => ({ ...c, [sessionId]: { startedAt: c[sessionId]?.startedAt ?? Date.now(), chars: c[sessionId]?.chars ?? 0, usage } })); }
        if (event.type === "text_delta") setStreamMeta((c) => ({ ...c, [sessionId]: { startedAt: c[sessionId]?.startedAt ?? Date.now(), chars: (c[sessionId]?.chars ?? 0) + (event.delta ?? "").length, usage: c[sessionId]?.usage } }));
        if (event.type === "memory_sources" && (event.truncated || event.skippedFiles)) setMemoryStatus("Lượt tìm ghi chú có tệp bỏ qua hoặc đã chạm giới hạn; nguồn có thể chưa đầy đủ.");
        setRunning((c) => ({ ...c, [sessionId]: event.runId }));
        const previousId = currentAssistantId;
        if (event.message) currentAssistantId = event.message.id;
        setState((c) => ({ ...c, sessions: c.sessions.map((s) => s.id !== sessionId ? s : {
          ...s, messages: s.messages.map((m) => {
            if (m.id === userId && event.userMessage) return event.userMessage;
            if (m.id !== previousId) return m;
            const base = { ...m, runId: event.runId, ...(event.message ?? {}) };
            if (event.type === "text_delta") base.text = m.text + (event.delta ?? "");
            if (event.type === "thinking_delta") base.thinking = (m.thinking ?? "") + (event.delta ?? "");
            if (event.type === "metrics" && event.metrics) base.metrics = event.metrics;
            if (event.sources) base.sources = event.sources;
            if (["completed", "cancelled", "failed"].includes(event.type))
              base.status = event.type as "completed" | "cancelled" | "failed";
            if (event.type === "failed" && event.error) base.error = event.error;
            return base;
          }),
        }) }));
        if (event.error) setRuntimeError(event.error);
        if (["completed", "cancelled", "failed"].includes(event.type)) {
          ended = true;
          setStreamMeta((c) => { const next = { ...c }; delete next[sessionId]; return next; });
          // Native transcript is authoritative, including IDs and partial/error status.
          void core.snapshot().then((snapshot) => setState((c) => ({ ...c,
            sessions: c.sessions.map((s) => s.id === sessionId ? snapshot.sessions.find((n) => n.id === sessionId) ?? s : s),
          }))).catch((error) => setRuntimeError(String(error))).finally(finish);
        }
      });
    } catch (error) {
      finish(); setRuntimeError(String(error));
      try {
        const snapshot = await core.snapshot();
        setState((c) => ({ ...c, sessions: c.sessions.map((s) => s.id === sessionId ? snapshot.sessions.find((n) => n.id === sessionId) ?? s : s) }));
      } catch (loadError) { setRuntimeError(`${String(error)} · ${String(loadError)}`); }
    }
  }
  async function stop() {
    const runId = running[session.id];
    if (!runId || runId === "starting") return;
    try { await core.cancel(session.id, runId); }
    catch (error) { setRuntimeError(String(error)); }
  }
  function toggleModule(id: string) {
    setState((c) => ({
      ...c,
      preferences: {
        ...c.preferences,
        modules: c.preferences.modules.includes(id)
          ? c.preferences.modules.filter((m) => m !== id)
          : [...c.preferences.modules, id],
      },
    }));
  }
  function openNote(note?: Note) {
    setEditor(note ?? null);
    setNoteTitle(note?.title ?? "");
    setNoteContent(note?.content ?? "");
    noteDialog.current?.showModal();
    noteDialog.current?.querySelector<HTMLInputElement>("#note-title")?.focus();
  }
  function saveNote() {
    if (!noteTitle.trim() || !noteContent.trim()) return;
    const note = {
      id: editor?.id ?? crypto.randomUUID(),
      title: noteTitle.trim(),
      content: noteContent.trim(),
      updated: new Date().toISOString(),
    };
    setState((c) => ({
      ...c,
      notes: editor
        ? c.notes.map((n) => (n.id === editor.id ? note : n))
        : [note, ...c.notes],
    }));
    noteDialog.current?.close();
    setNotice("Đã lưu ghi chú cục bộ.");
  }
  async function attach(file?: File) {
    if (!file) return;
    if (!/\.(txt|md)$/i.test(file.name) || file.size > 1_000_000) {
      setNotice("Chọn tệp .txt hoặc .md, tối đa 1 MB.");
      return;
    }
    try {
      setAttachment({ name: file.name, text: await file.text() });
    } catch {
      setNotice("Không đọc được tệp. Hãy chọn lại.");
    }
  }
  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      setNotice("Đã sao chép.");
    } catch {
      setNotice(
        "Không có quyền clipboard. Bạn có thể chọn và sao chép văn bản trực tiếp.",
      );
    }
  }

  return (
    <>
      <a className="skip-link" href="#main">
        Đến nội dung chính
      </a>
      <div className={`app-shell ${collapsed ? "sidebar-collapsed" : ""}`}>
        <aside id="sidebar" className="sidebar" aria-label="Điều hướng">
          <div className="sidebar-header">
            <button
              className="brand"
              aria-label="Lim Harness — mở hội thoại"
              onClick={() => setPage("chat")}
            >
              <span className="brand-mark">
                <Logo />
              </span>
              <span className="brand-name">Lim Harness</span>
            </button>
            <button
              className="icon-button"
              aria-label={collapsed ? "Mở rộng sidebar" : "Thu gọn sidebar"}
              aria-expanded={!collapsed}
              aria-controls="sidebar"
              title={collapsed ? "Mở rộng sidebar" : "Thu gọn sidebar"}
              onClick={() => setCollapsed(!collapsed)}
            >
              {collapsed ? (
                <PanelLeftOpen size={18} />
              ) : (
                <PanelLeftClose size={18} />
              )}
            </button>
          </div>
          <div className="sidebar-actions">
            <button
              className="sidebar-action"
              onClick={createSession}
              aria-label="Phiên mới"
              title="Phiên mới (Ctrl N)"
            >
              <Plus size={18} />
              <span>Phiên mới</span>
            </button>
            <button
              className="sidebar-action"
              onClick={openSearch}
              aria-label="Tìm kiếm"
              title="Tìm kiếm (Ctrl K)"
            >
              <Search size={17} />
              <span>Tìm kiếm</span>
              <kbd>Ctrl K</kbd>
            </button>
          </div>
          <nav className="navigation">
            {navigation.map(({ id, label, icon: Icon }) => (
              <button
                key={id}
                className={`nav-item ${page === id ? "active" : ""}`}
                aria-current={page === id ? "page" : undefined}
                aria-label={label}
                title={label}
                data-window-opener={id}
                onClick={() => setPage(id)}
              >
                <Icon size={18} />
                <span>{label}</span>
              </button>
            ))}
          </nav>
          <div className="session-label">Hội thoại gần đây</div>
          <div className="session-list">
            {state.sessions.map((s) => (
              <div className={`session-row ${state.active === s.id && page === "chat" ? "selected" : ""}`} key={s.id} onMouseLeave={() => pendingDelete === s.id && setPendingDelete("")}>
                <button
                  className={`session-item ${state.active === s.id && page === "chat" ? "selected" : ""}`}
                  onClick={() => openSession(s.id)}
                  title={s.title}
                  aria-label={`Mở phiên: ${s.title}`}
                >
                  <MessageSquare size={14} />
                  <span>{s.title}</span>
                </button>
                <button
                  className={`session-delete ${pendingDelete === s.id ? "armed" : ""}`}
                  onClick={() => void deleteSession(s.id)}
                  onBlur={() => pendingDelete === s.id && setPendingDelete("")}
                  disabled={!ready || Boolean(running[s.id])}
                  title={running[s.id] ? "Đang chạy, chưa xoá được" : pendingDelete === s.id ? "Bấm lần nữa để xoá vĩnh viễn" : "Xoá hội thoại"}
                  aria-label={pendingDelete === s.id ? `Xác nhận xoá phiên: ${s.title}` : `Xoá phiên: ${s.title}`}
                >
                  {pendingDelete === s.id ? <span>Xoá?</span> : <Trash2 size={13} aria-hidden="true" />}
                </button>
              </div>
            ))}
          </div>
          {Object.values(windows).includes("minimized") && <nav className="window-dock" aria-label="Cửa sổ đã thu gọn"><span className="dock-label">Đã thu gọn</span>{(Object.keys(toolTitles) as ToolPage[]).filter((id) => windows[id] === "minimized").map((id) => { const Icon = navigation.find((item) => item.id === id)?.icon || Settings2; return <button key={id} onClick={() => setPage(id)} aria-label={`Khôi phục ${toolTitles[id]}`} title={`Khôi phục ${toolTitles[id]}`}><Icon size={14} aria-hidden="true" /><span>{toolTitles[id]}</span></button>; })}</nav>}
          <div className="sidebar-footer">
            <button
              className={`footer-action ${page === "settings" ? "active" : ""}`}
              aria-label="Cài đặt"
              title="Cài đặt"
              data-window-opener="settings"
              onClick={() => setPage("settings")}
            >
              <Settings2 size={18} />
              <span>Cài đặt</span>
            </button>
            <button
              className="footer-action"
              aria-label={
                state.preferences.theme === "light"
                  ? "Bật giao diện tối"
                  : "Bật giao diện sáng"
              }
              title="Đổi giao diện"
              onClick={() =>
                setState((c) => ({
                  ...c,
                  preferences: {
                    ...c.preferences,
                    theme: c.preferences.theme === "light" ? "dark" : "light",
                  },
                }))
              }
            >
              {state.preferences.theme === "light" ? (
                <Moon size={18} />
              ) : (
                <Sun size={18} />
              )}
              <span>
                {state.preferences.theme === "light"
                  ? "Giao diện tối"
                  : "Giao diện sáng"}
              </span>
            </button>
          </div>
        </aside>
        <div className="workspace-main">
          {(
            <header className="topbar">
              <span className="session-title">{session.title}</span>
              <div className="runtime-hud" aria-label="Cấu hình model hiện tại"><span className="hud-model" title={state.preferences.model}>{state.preferences.model || "Ollama · chưa chọn model"}</span><span className="hud-temperature">temp={effective.temperature.toFixed(2)}</span><ContextMeter usage={contextUsage(session.messages, draft, effective.contextSize, effective.numPredict + effective.thinkPredict, effective.systemPrompt)} memory={Boolean(vaultPath)} /></div>
              <button className={`icon-button inference-toggle ${inferenceDirty ? "has-draft" : ""}`} aria-label={inferenceDirty ? "Tham số chạy model · có thay đổi chưa áp dụng" : "Tham số chạy model"} title="Tham số chạy model" aria-expanded={inferenceOpen} aria-controls="inference-heading" onClick={() => setInferenceOpen((open) => !open)}><Settings2 size={17} /></button>
            </header>
          )}
          {storageError && (
            <div className="storage-error" role="alert">
              {storageError}
            </div>
          )}
          {runtimeError && <div className="storage-error" role="alert">{runtimeError}<button className="icon-button" aria-label="Đóng lỗi" onClick={() => setRuntimeError("")}><X size={15} /></button></div>}
          <div className="chat-layout"><main
            id="main"
            tabIndex={-1}
            className={`chat-main chat-column ${!session.messages.length ? "chat-empty" : ""}`}
          >
            <FloatingWindow {...windowProps("cookbook")} width={1100} height={780}>
              <Cookbook visible={windows.cookbook === "open"} selectedModel={state.preferences.model} contextSize={effective.contextSize} chatBusy={Object.keys(running).length > 0} onContext={(size) => void chooseContext(size)} onChat={() => setPage("chat")} onSelected={(model, size) => {
                setContextSize(size);
                setState((c) => ({ ...c, preferences: { ...c.preferences, provider: "ollama", model } }));
                void refreshModels();
                setNotice(`Đã nạp ${model} và chọn cho hội thoại.`);
                void core.snapshot().then((snapshot) => {
                  setContextSize(snapshot.contextSize);
                  setState((c) => ({ ...c, preferences: { ...c.preferences, provider: "ollama", model: snapshot.model } }));
                }).catch((error) => setRuntimeError(String(error)));
              }} />
            </FloatingWindow>
            {(
              <>
                <div
                  className={`conversation ${!session.messages.length ? "is-empty" : ""}`}
                >
                  {!session.messages.length ? (
                    <div className="welcome">
                      <div className="welcome-logo">
                        <span className="brand-mark">
                          <Logo />
                        </span>
                        <h1>Lim Harness</h1>
                      </div>
                      <p>Bắt đầu một cuộc hội thoại.</p>
                    </div>
                  ) : (
                    <div
                      className="messages"
                      role="log"
                      aria-label="Nội dung hội thoại"
                      aria-live="polite"
                      aria-relevant="additions"
                    >
                      {session.messages.map((m) => {
                        const isUser = m.role === "user";
                        const modelLabel = m.model || state.preferences.model || "Lim";
                        const parsed = isUser ? null : splitThink(m.text);
                        const thinking = isUser ? "" : (m.thinking || parsed?.thinking || "");
                        const answer = isUser ? m.text : (parsed?.answer ?? m.text);
                        const isRunning = m.status === "running";
                        const meta = streamMeta[session.id];
                        const liveSpeed = isRunning && !isUser && meta ? liveUsage(meta) : "";
                        return (
                        <article className={`message ${m.role}`} key={m.id}>
                          <span className="message-avatar" aria-hidden="true">
                            {isUser ? "B" : <Logo />}
                          </span>
                          <div className="message-body">
                            <div className="message-author">
                              {isUser ? "Bạn" : modelLabel}
                              {m.source === "ui_preview" && (
                                <span className="demo-label">UI thử</span>
                              )}
                            </div>
                            {!isUser && thinking && (
                              (() => {
                                const steps = thinkingSteps(thinking);
                                const live = isRunning && !answer.trim();
                                return <details className="message-thinking" open={live}>
                                  <summary>{live ? `Đang suy nghĩ · bước ${steps.length}` : "Quá trình suy nghĩ"}<span className="thinking-count">{steps.length} bước{m.metrics?.thinkingTokens ? ` · ~${m.metrics.thinkingTokens} token` : ""}{m.metrics?.thinkingCapped ? " · chạm giới hạn" : ""}</span></summary>
                                  <ThinkingSteps steps={steps} live={live} />
                                </details>;
                              })()
                            )}
                            <div className="message-bubble">
                              <p>{answer || (isRunning ? "Đang chờ model…" : "Không có nội dung trả lời.")}</p>
                            </div>
                            {m.source === "ui_preview" && <small className="message-status">Thông báo từ bản UI thử</small>}
                            {m.status && m.status !== "completed" && !isUser && <small className="message-status">{{ running: liveSpeed || "Đang trả lời", cancelled: "Đã dừng · câu trả lời chưa hoàn tất", failed: m.error ? `Lượt chạy bị lỗi: ${m.error}` : "Lượt chạy bị lỗi", interrupted: "Bị gián đoạn khi đóng ứng dụng" }[m.status]}</small>}
                            {!isUser && m.metrics && m.status === "completed" && formatMetrics(m.metrics) && (
                              <small className="message-metrics">{formatMetrics(m.metrics)}</small>
                            )}
                            {!!m.sources?.length && <details className="message-sources"><summary>Nguồn ghi chú ({m.sources.length})</summary>{m.sources.map((source, i) => <div key={source.path}><strong>[vault:{i + 1}] {source.title}</strong><small>{source.path}</small><p>{source.excerpt}</p></div>)}</details>}
                            <button
                              className="message-copy"
                              aria-label="Sao chép tin nhắn"
                              onClick={() => copy(answer)}
                            >
                              <Copy size={13} />
                              Sao chép
                            </button>
                          </div>
                        </article>
                        );
                      })}
                      <div ref={messageEnd} />
                    </div>
                  )}
                </div>
                <div className="composer-area">
                  <form
                    className="composer"
                    onSubmit={(e) => {
                      e.preventDefault();
                      send();
                    }}
                  >
                    {attachment && (
                      <div className="attachment-chip">
                        <FileText size={14} />
                        {attachment.name}
                        <button
                          type="button"
                          className="icon-button"
                          aria-label="Gỡ tệp đính kèm"
                          onClick={() => setAttachment(null)}
                        >
                          <X size={14} />
                        </button>
                      </div>
                    )}
                    <label className="sr-only" htmlFor="message-input">
                      Nội dung tin nhắn
                    </label>
                    <textarea
                      id="message-input"
                      ref={composer}
                      placeholder={state.preferences.model ? `Nhắn cho ${state.preferences.model}...` : "Nhắn cho Lim..."}
                      value={draft}
                      readOnly={busy}
                      maxLength={20000}
                      rows={1}
                      onChange={(e) => setDraft(e.target.value)}
                      onKeyDown={(e) => {
                        if (
                          e.key === "Enter" &&
                          !e.shiftKey &&
                          !e.nativeEvent.isComposing
                        ) {
                          e.preventDefault();
                          send();
                        }
                      }}
                    />
                    <div className="composer-bottom">
                      <div className="composer-options">
                        <label
                          className="icon-button attachment-button"
                          title="Thêm tệp .txt hoặc .md"
                        >
                          <Paperclip size={18} />
                          <span className="sr-only">Thêm tệp văn bản</span>
                          <input
                            type="file"
                            accept=".txt,.md"
                            onChange={(e) => {
                              void attach(e.target.files?.[0]);
                              e.target.value = "";
                            }}
                          />
                        </label>
                        <button
                          type="button"
                          className="model-button"
                          onClick={() => setPage("settings")}
                        >
                          <Cpu size={15} />
                          <span>{state.preferences.model || "Chọn model"}</span>
                          <ChevronDown size={13} />
                        </button>
                        <button
                          type="button"
                          className="module-button"
                          onClick={() => setPage("modules")}
                          aria-label={`Cấu hình module: ${selectedModules} đã chọn`}
                        >
                          <Layers3 size={15} />
                          <span>{selectedModules} module</span>
                        </button>
                      </div>
                      <button
                        className="send-button"
                        type={busy ? "button" : "submit"}
                        onClick={busy ? () => void stop() : undefined}
                        aria-label={busy ? "Dừng trả lời" : "Gửi tin nhắn"}
                        disabled={!ready || (busy ? running[session.id] === "starting" : !draft.trim() && !attachment)}
                      >
                        {busy ? <Square size={15} /> : <ArrowUp size={19} />}
                      </button>
                    </div>
                  </form>
                  <div className="composer-hint">
                    <span>{inferenceDirty ? "Chưa áp dụng tham số · gửi dùng cấu hình đã lưu" : !native ? "Bản xem trước · mở desktop để chạy model" : !ready ? "Đang mở dữ liệu…" : busy ? "Ollama đang chạy" : vaultPath ? "Ollama · Memory từ Obsidian" : "Ollama local"}</span>
                    <span>
                      <kbd>Enter</kbd> gửi · <kbd>Shift Enter</kbd> xuống dòng
                    </span>
                  </div>
                </div>
              </>
            )}
            <FloatingWindow {...windowProps("memory")} offset={20}>
              <>
                <PageHeading
                  title="Bộ nhớ"
                  description="Ghi chú cho công việc của bạn."
                  action={
                    <button
                      className="primary-button"
                      onClick={() => openNote()}
                    >
                      <Plus size={17} />
                      Ghi chú mới
                    </button>
                  }
                />
                <div className="memory-status">
                  <span className="status-dot" />
                  {vaultPath ? "Obsidian đã kết nối · chỉ đọc" : "Ghi chú cục bộ · chưa chọn vault"}
                </div>
                {vaultPath && <section className="settings-section vault-search"><form onSubmit={(e) => { e.preventDefault(); void searchVault(); }}><label htmlFor="vault-query">Tìm trong Obsidian</label><div className="vault-controls"><input id="vault-query" value={memoryQuery} onChange={(e) => setMemoryQuery(e.target.value)} maxLength={2000} placeholder="Từ khóa ghi chú" /><button className="secondary-button" disabled={memoryBusy || !memoryQuery.trim()}>Tìm</button></div></form><p role="status">{memoryStatus}</p>{memoryHits.map((hit, i) => <article className="vault-hit" key={hit.path}><h2>[vault:{i + 1}] {hit.title}</h2><small>{hit.path}</small><p>{hit.excerpt}</p></article>)}</section>}
                {state.notes.length ? (
                  <div className="note-grid">
                    {state.notes.map((n) => (
                      <button
                        className="note-card"
                        key={n.id}
                        onClick={() => openNote(n)}
                      >
                        <h2>{n.title}</h2>
                        <p>{n.content}</p>
                        <footer>{formatDate(n.updated)}</footer>
                      </button>
                    ))}
                  </div>
                ) : (
                  <div className="empty-state">
                    <BookOpen size={27} strokeWidth={1.5} />
                    <h2>Chưa có ghi chú</h2>
                    <p>Lưu một ý tưởng hoặc quyết định để tìm lại sau.</p>
                    <button
                      className="secondary-button"
                      onClick={() => openNote()}
                    >
                      <Plus size={16} />
                      Tạo ghi chú
                    </button>
                  </div>
                )}
              </>
            </FloatingWindow>
            <FloatingWindow {...windowProps("modules")} width={720} height={560} offset={40}>
              <>
                <PageHeading
                  title="Module"
                  description="Chọn khả năng cho hội thoại."
                />
                <div className="section-toolbar">
                  <span>Danh mục mẫu · Chưa có executor</span>
                  <span>{selectedModules} đã chọn</span>
                </div>
                <div className="module-list">
                  {moduleCatalog.map((m) => {
                    const Icon = moduleIcons[m.icon];
                    const enabled = state.preferences.modules.includes(m.id);
                    return (
                      <div className="module-row" key={m.id}>
                        <span className="module-icon">
                          <Icon size={21} strokeWidth={1.6} />
                        </span>
                        <div className="module-details">
                          <h2>{m.name}</h2>
                          <p>{m.description}</p>
                        </div>
                        <span className="module-tag">{m.tag}</span>
                        <button
                          className="toggle"
                          role="switch"
                          aria-label={`Chọn ${m.name}`}
                          aria-checked={enabled}
                          onClick={() => toggleModule(m.id)}
                        >
                          <span />
                        </button>
                      </div>
                    );
                  })}
                </div>
              </>
            </FloatingWindow>
            <FloatingWindow {...windowProps("settings")} width={650} height={660} offset={60}>
              <>
                <PageHeading
                  title="Cài đặt"
                  description="Cấu hình cho Lim Harness."
                />
                <div className="settings-form">
                  <section className="settings-section">
                    <h2>Model</h2>
                    <div className="field-row">
                      <label htmlFor="provider">Provider</label>
                      <select
                        id="provider"
                        value="ollama"
                        disabled
                      >
                        <option value="ollama">Ollama local</option>
                      </select>
                    </div>
                    <div className="field-row">
                      <label htmlFor="model">Tên model</label>
                      <select
                        id="model"
                        value={state.preferences.model}
                        disabled={!native || !ready}
                        onChange={(e) => void chooseModel(e.target.value)}
                      ><option value="">Chọn model đã tải</option>{state.preferences.model && !models.includes(state.preferences.model) && <option value={state.preferences.model}>{state.preferences.model} (chưa tìm thấy)</option>}{models.map((name) => <option key={name} value={name}>{name}</option>)}</select>
                    </div>
                    <button className="secondary-button" disabled={!native || !ready} onClick={() => void refreshModels()}>Tải lại danh sách</button>
                    <button className="secondary-button" onClick={() => setPage("cookbook")}>Mở Cookbook</button>
                    <p className="settings-status" role="status">{modelError || "Ollama tại 127.0.0.1:11434. Model và hội thoại được lưu bởi lõi desktop."}</p>
                    <div className="field-row"><label htmlFor="context-size">Context</label><select id="context-size" value={effective.contextSize} disabled={!native || !ready} onChange={(e) => void chooseContext(Number(e.target.value))}><option value={4096}>4K · tiết kiệm bộ nhớ</option><option value={8192}>8K</option><option value={16384}>16K · cần thêm RAM/VRAM</option></select></div>
                  </section>
                  <section className="settings-section">
                    <h2>Bộ nhớ Obsidian</h2>
                    <label htmlFor="vault-path">Thư mục vault</label>
                    <div className="vault-controls"><input id="vault-path" value={vaultDraft} onChange={(e) => setVaultDraft(e.target.value)} placeholder="C:\\Notes\\My Vault" maxLength={4096} /><button className="secondary-button" disabled={!native || !ready || memoryBusy || !vaultDraft.trim()} onClick={() => void connectVault(vaultDraft)}>Kết nối</button>{vaultPath && <button className="secondary-button" disabled={memoryBusy} onClick={() => void connectVault("")}>Ngắt</button>}</div>
                    <p className="settings-status" role="status">{memoryStatus || "Chỉ đọc Markdown và tìm ghi chú liên quan khi gửi tin. Không sửa vault. Ghi chú cục bộ phía trên vẫn được lưu riêng."}</p>
                  </section>
                </div>
              </>
            </FloatingWindow>
          </main>
          <InferencePanel key={session.id} sessionTitle={session.title} open={inferenceOpen} settings={panelSettings} contextSize={effective.contextSize} model={state.preferences.model} ready={ready} onDirty={setInferenceDirty} onSave={saveInference} onClose={() => setInferenceOpen(false)} />
          </div>
        </div>
      </div>
      <div
        className={`toast ${notice ? "visible" : ""}`}
        role="status"
        aria-live="polite"
      >
        {notice}
      </div>
      <dialog
        ref={searchDialog}
        className="search-dialog"
        aria-labelledby="search-title"
      >
        <div className="dialog-heading">
          <h2 id="search-title">Tìm kiếm</h2>
          <button
            className="icon-button"
            aria-label="Đóng tìm kiếm"
            onClick={() => searchDialog.current?.close()}
          >
            <X size={20} />
          </button>
        </div>
        <label className="sr-only" htmlFor="search-input">
          Tìm phiên và ghi chú
        </label>
        <div className="dialog-search">
          <Search size={18} />
          <input
            id="search-input"
            placeholder="Tìm phiên hoặc ghi chú..."
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
        <div className="search-results">
          {state.sessions
            .filter((s) => s.title.toLowerCase().includes(query.toLowerCase()))
            .map((s) => (
              <button
                key={s.id}
                onClick={() => {
                  openSession(s.id);
                  searchDialog.current?.close();
                }}
              >
                <MessageSquare size={17} />
                <span>
                  {s.title}
                  <small>Hội thoại</small>
                </span>
              </button>
            ))}
          {state.notes
            .filter((n) =>
              `${n.title} ${n.content}`
                .toLowerCase()
                .includes(query.toLowerCase()),
            )
            .map((n) => (
              <button
                key={n.id}
                onClick={() => {
                  searchDialog.current?.close();
                  setPage("memory");
                  openNote(n);
                }}
              >
                <FileText size={17} />
                <span>
                  {n.title}
                  <small>Ghi chú</small>
                </span>
              </button>
            ))}
          {!state.sessions.some((s) =>
            s.title.toLowerCase().includes(query.toLowerCase()),
          ) &&
            !state.notes.some((n) =>
              `${n.title} ${n.content}`
                .toLowerCase()
                .includes(query.toLowerCase()),
            ) && <p>Không tìm thấy kết quả.</p>}
        </div>
        <div className="dialog-hint">
          <kbd>Esc</kbd> để đóng
        </div>
      </dialog>
      <dialog
        ref={noteDialog}
        className="note-dialog"
        aria-labelledby="note-heading"
        aria-describedby="note-description"
      >
        <form
          onSubmit={(e) => {
            e.preventDefault();
            saveNote();
          }}
        >
          <div className="dialog-heading">
            <h2 id="note-heading">
              {editor ? "Chỉnh sửa ghi chú" : "Ghi chú mới"}
            </h2>
            <button
              type="button"
              className="icon-button"
              aria-label="Đóng ghi chú"
              onClick={() => noteDialog.current?.close()}
            >
              <X size={20} />
            </button>
          </div>
          <p id="note-description" className="dialog-description">
            Lưu cục bộ, chưa đồng bộ với Obsidian.
          </p>
          <label htmlFor="note-title">Tiêu đề</label>
          <input
            id="note-title"
            value={noteTitle}
            onChange={(e) => setNoteTitle(e.target.value)}
            maxLength={120}
            required
          />
          <label htmlFor="note-content">Nội dung</label>
          <textarea
            id="note-content"
            value={noteContent}
            onChange={(e) => setNoteContent(e.target.value)}
            maxLength={30000}
            required
            rows={8}
          />
          <div className="dialog-footer">
            <button
              type="button"
              className="secondary-button"
              onClick={() => noteDialog.current?.close()}
            >
              Hủy
            </button>
            <button
              type="submit"
              className="primary-button"
              disabled={!noteTitle.trim() || !noteContent.trim()}
            >
              <Check size={16} />
              Lưu ghi chú
            </button>
          </div>
        </form>
      </dialog>
    </>
  );
}

function PageHeading({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action?: React.ReactNode;
}) {
  return (
    <div className="page-heading">
      <div>
        <h1>{title}</h1>
        <p>{description}</p>
      </div>
      {action}
    </div>
  );
}
