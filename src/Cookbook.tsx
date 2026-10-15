import { useEffect, useRef, useState } from "react";
import { ArrowDown, ArrowRight, ArrowUp, Box, Check, Cpu, Download, ExternalLink, Flame, Globe, HardDrive, RefreshCw, Search, Settings, Square, Unplug, X } from "lucide-react";
import { native } from "./core";
import { cookbook } from "./cookbook-api";
import type { CookbookSnapshot, InstalledModel, SetupEvent } from "./cookbook-api";
import { BUNDLED_CATALOG, getModelFit, estimateMemoryBytes } from "./model-catalog";
import type { AssessmentMode, CatalogModel, CatalogSnapshot } from "./model-catalog";
import "./cookbook.css";

const GiB = 1024 ** 3;
const memory = (bytes: number | null | undefined) => bytes == null ? "—" : `${(bytes / GiB).toFixed(1)} GiB`;
const fitLabels = { recommended: "Phù hợp", caution: "Cân nhắc", limited: "Giới hạn", unknown: "Chưa rõ" };
const stepLabels = { started: "Chuẩn bị", progress: "Đang tải", loading: "Đang nạp", downloaded: "Đã tải", ready: "Đã nạp", failed: "Bị lỗi", cancelled: "Đã ngắt" };
const tabs = [
  { id: "active", label: "Active", icon: Cpu },
  { id: "launch", label: "Launch", icon: Flame },
  { id: "download", label: "Download", icon: Download },
  { id: "dependencies", label: "Dependencies", icon: Box },
  { id: "settings", label: "Settings", icon: Settings },
] as const;
type Tab = typeof tabs[number]["id"];
type Sort = "fit" | "model" | "memory" | "params" | "quant" | "context" | "score" | "speed" | "source";
type Row = { id: string; catalog?: CatalogModel; local?: InstalledModel };
type Provider = "all" | "ollama" | "huggingface";
const providerOf = (id: string, entry?: CatalogModel) => entry?.provider ?? (id.toLowerCase().startsWith("hf.co/") ? "huggingface" : "ollama");
const providerLabels = { ollama: "Ollama", huggingface: "Hugging Face" };
const compact = (value: number) => new Intl.NumberFormat("vi-VN", { notation: "compact", maximumFractionDigits: 1 }).format(value);

export default function Cookbook({ visible, selectedModel, contextSize, chatBusy, onSelected, onContext, onChat }: {
  visible: boolean; selectedModel: string; contextSize: number; chatBusy: boolean;
  onSelected: (model: string, context: number) => void; onContext: (context: number) => void; onChat: () => void;
}) {
  const [snapshot, setSnapshot] = useState<CookbookSnapshot | null>(null);
  const [catalog, setCatalog] = useState<CatalogSnapshot>(BUNDLED_CATALOG);
  const [catalogBusy, setCatalogBusy] = useState(false);
  const [catalogError, setCatalogError] = useState("");
  const [assessment, setAssessment] = useState<AssessmentMode>("auto");
  const [fitOnly, setFitOnly] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [tab, setTab] = useState<Tab>("download");
  const [query, setQuery] = useState("");
  const [directTag, setDirectTag] = useState("");
  const [filter, setFilter] = useState("all");
  const [provider, setProvider] = useState<Provider>("all");
  const [found, setFound] = useState<CatalogModel[]>([]);
  const [foundQuery, setFoundQuery] = useState("");
  const [searching, setSearching] = useState(false);
  const searchSeq = useRef(0);
  const [sort, setSort] = useState<Sort>("fit");
  const [descending, setDescending] = useState(false);
  const [detailId, setDetailId] = useState("");
  const [operation, setOperation] = useState<SetupEvent | null>(null);
  const [working, setWorking] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const actionLock = useRef(false);
  const refreshLock = useRef(false);
  const catalogLock = useRef(false);
  const alive = useRef(true);
  const selectedCallback = useRef(onSelected);
  selectedCallback.current = onSelected;

  async function refresh() {
    if (!native || refreshLock.current) return;
    refreshLock.current = true;
    setLoading(true);
    try {
      const result = await cookbook.snapshot();
      if (alive.current) { setSnapshot(result); setError(result.error ?? ""); }
    } catch (failure) {
      if (alive.current) setError(String(failure));
    } finally {
      refreshLock.current = false;
      if (alive.current) setLoading(false);
    }
  }
  async function refreshCatalog(force = false) {
    if (!native || catalogLock.current) return;
    catalogLock.current = true; setCatalogBusy(true); setCatalogError("");
    try {
      const result = await cookbook.catalog(force);
      if (alive.current) {
        if (result.models.length) setCatalog(result);
        setCatalogError(result.warnings.join(" · "));
      }
    } catch (failure) { if (alive.current) setCatalogError(String(failure)); }
    finally { catalogLock.current = false; if (alive.current) setCatalogBusy(false); }
  }
  async function searchOnline() {
    const term = query.trim();
    if (!native || !term) return;
    const sequence = ++searchSeq.current;
    setSearching(true); setCatalogError("");
    try {
      const result = await cookbook.search(term, provider);
      if (!alive.current || sequence !== searchSeq.current) return;
      setFound(result.models); setFoundQuery(term);
      setCatalogError([`Tìm online "${term}": ${result.models.length} kết quả`, ...result.warnings].join(" · "));
    } catch (failure) { if (alive.current && sequence === searchSeq.current) setCatalogError(String(failure)); }
    finally { if (alive.current && sequence === searchSeq.current) setSearching(false); }
  }
  function clearSearch() { searchSeq.current++; setFound([]); setFoundQuery(""); setSearching(false); setCatalogError(""); }
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  useEffect(() => {
    if (!visible || !native) return;
    void refresh();
    void refreshCatalog();
    const timer = setInterval(() => { if (!actionLock.current) void refresh(); }, 20000);
    return () => clearInterval(timer);
  }, [visible]);

  async function setup(model: string, downloadOnly = false) {
    if (actionLock.current || chatBusy || !native || !snapshot?.ollamaAvailable) return;
    actionLock.current = true;
    setWorking(true);
    setError("");
    setCancelling(false);
    setOperation({ type: "started", operationId: "", sequence: -1, model });
    let sequence = -1;
    let ended = false;
    const chosenContext = contextSize;
    const finish = () => {
      actionLock.current = false;
      if (alive.current) { setWorking(false); setCancelling(false); }
    };
    const receive = (event: SetupEvent) => {
      if (ended || event.sequence <= sequence) return;
      sequence = event.sequence;
      if (alive.current) setOperation(event);
      if (["ready", "downloaded", "failed", "cancelled"].includes(event.type)) {
        ended = true;
        if (event.type === "ready") selectedCallback.current(event.model, chosenContext);
        if (event.error && alive.current) setError(event.error);
        void refresh().finally(finish);
      }
    };
    try {
      if (downloadOnly) await cookbook.download(model, receive);
      else await cookbook.setup(model, chosenContext, receive);
    } catch (failure) {
      ended = true;
      if (alive.current) {
        setError(String(failure));
        setOperation({ type: "failed", operationId: "", sequence, model, error: String(failure) });
      }
      void refresh().finally(finish);
    }
  }
  async function cancel() {
    if (!operation?.operationId || cancelling) return;
    setCancelling(true);
    try {
      const accepted = await cookbook.cancel(operation.operationId);
      if (!accepted) { setCancelling(false); void refresh(); }
    } catch (failure) { setError(String(failure)); setCancelling(false); }
  }
  async function unload(model: string) {
    if (actionLock.current || chatBusy) return;
    actionLock.current = true;
    setWorking(true);
    setError("");
    setOperation(null);
    try { await cookbook.unload(model); await refresh(); }
    catch (failure) { setError(String(failure)); }
    finally { actionLock.current = false; setWorking(false); }
  }

  const hw = snapshot?.hardware;
  const installed = snapshot?.models ?? [];
  const running = snapshot?.running ?? [];
  const disabled = !native || !snapshot?.ollamaAvailable || working || chatBusy;
  const sameTag = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
  // Live search results first, then the cached/online catalog; ids stay unique across both.
  const entries = [...found, ...catalog.models.filter((entry) => !found.some((hit) => sameTag(hit.id, entry.id)))];
  const catalogRows: Row[] = entries.map((entry) => ({ id: entry.id, catalog: entry, local: installed.find((item) => sameTag(item.name, entry.id)) }));
  const rows: Row[] = [...catalogRows, ...installed.filter((item) => !entries.some((entry) => sameTag(entry.id, item.name))).map((local) => ({ id: local.name, local }))];
  const isFound = (row: Row) => found.some((hit) => sameTag(hit.id, row.id));
  const fitFor = (row: Row) => hw && row.catalog ? getModelFit(row.local ? { ...row.catalog, downloadBytes: row.local.sizeBytes, weightBytes: row.local.sizeBytes } : row.catalog, hw, contextSize, assessment) : null;
  const rowMemory = (row: Row) => fitFor(row)?.estimatedMemoryGiB ?? (row.catalog ? estimateMemoryBytes(row.catalog, contextSize) / GiB : row.local ? row.local.sizeBytes / GiB : null);
  const rowParams = (row: Row) => row.local?.parameterSize ?? row.catalog?.parameterSize ?? "—";
  const observation = (row: Row) => snapshot?.measurements?.find((entry) => sameTag(entry.model, row.id) && entry.contextSize === contextSize);
  const embedding = (row: Row) => row.catalog?.capabilities.some((capability) => capability.toLowerCase() === "embedding");
  const parameterValue = (row: Row) => {
    const label = rowParams(row).toUpperCase();
    return (parseFloat(label) || 0) / (label.endsWith("M") ? 1000 : 1);
  };
  const rowQuant = (row: Row) => row.local?.quantizationLevel ?? row.catalog?.quantization ?? "—";
  const fitOrder = { recommended: 0, caution: 1, unknown: 2, limited: 3 };
  const shownRows = rows.filter((row) => {
    if (tab === "active" && !running.some((model) => model.name === row.id)) return false;
    if (tab === "launch" && !row.local) return false;
    if (filter === "installed" && !row.local) return false;
    if (provider !== "all" && providerOf(row.id, row.catalog) !== provider) return false;
    if (["tools", "vision", "embedding", "thinking"].includes(filter) && !row.catalog?.capabilities.some((capability) => capability.toLowerCase() === filter)) return false;
    if (fitOnly && !["recommended", "caution"].includes(fitFor(row)?.status ?? "unknown")) return false;
    // Online hits already matched the search term on the server (e.g. "llama" → Meta-Llama repos).
    if (foundQuery && query.trim() === foundQuery && isFound(row)) return true;
    return `${row.id} ${row.catalog?.name ?? ""}`.toLowerCase().includes(query.toLowerCase());
  }).sort((a, b) => {
    let order = 0;
    if (sort === "model") order = a.id.localeCompare(b.id);
    if (sort === "source") order = providerOf(a.id, a.catalog).localeCompare(providerOf(b.id, b.catalog)) || (b.catalog?.downloads ?? 0) - (a.catalog?.downloads ?? 0);
    if (sort === "fit") order = fitOrder[fitFor(a)?.status ?? "unknown"] - fitOrder[fitFor(b)?.status ?? "unknown"];
    if (sort === "score") order = (fitFor(a)?.score ?? -1) - (fitFor(b)?.score ?? -1);
    if (sort === "speed") order = (observation(a)?.tokensPerSecond ?? -1) - (observation(b)?.tokensPerSecond ?? -1);
    if (sort === "memory") {
      const amount = (row: Row) => tab === "active" ? running.find((model) => model.name === row.id)?.sizeVramBytes : rowMemory(row);
      order = (amount(a) ?? Infinity) - (amount(b) ?? Infinity);
    }
    if (sort === "params") order = parameterValue(a) - parameterValue(b);
    if (sort === "quant") order = rowQuant(a).localeCompare(rowQuant(b));
    if (sort === "context") order = (running.find((model) => model.name === a.id)?.contextLength ?? contextSize) - (running.find((model) => model.name === b.id)?.contextLength ?? contextSize);
    return (descending ? -order : order) || a.id.localeCompare(b.id);
  });
  const detail = rows.find((row) => row.id === detailId);
  const detailFit = detail ? fitFor(detail) : null;
  const liveDetail = running.find((model) => model.name === detailId);
  const hasProgress = operation?.total != null && operation.total > 0 && operation.completed != null;
  const percent = hasProgress ? Math.min(100, Math.round(100 * operation!.completed! / operation!.total!)) : undefined;
  const selectedReady = running.some((model) => model.name === selectedModel && model.contextLength === contextSize);

  function sortColumn(column: Sort, label: string) {
    return <th scope="col" aria-sort={sort === column ? descending ? "descending" : "ascending" : "none"}>
      <button onClick={() => { setDescending(sort === column ? !descending : false); setSort(column); }}>
        {label}{sort === column && (descending ? <ArrowDown size={11} /> : <ArrowUp size={11} />)}
      </button>
    </th>;
  }

  return <section className="cookbook" aria-label="Cookbook model">
    <div className="cookbook-tabs" role="tablist" aria-label="Cookbook">
      {tabs.map(({ id, label, icon: Icon }, index) => <button
        key={id} id={`cookbook-tab-${id}`} role="tab" aria-selected={tab === id}
        aria-controls="cookbook-panel" tabIndex={tab === id ? 0 : -1}
        onClick={() => { setTab(id); setFilter("all"); setQuery(""); }}
        onKeyDown={(event) => {
          if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
          event.preventDefault();
          const next = event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : (index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
          setTab(tabs[next].id); setFilter("all"); setQuery("");
          document.getElementById(`cookbook-tab-${tabs[next].id}`)?.focus();
        }}
      ><Icon size={15} aria-hidden="true" />{label}{id === "active" && running.length > 0 && <span>{running.length}</span>}</button>)}
    </div>

    <div className="cookbook-panel" id="cookbook-panel" role="tabpanel" aria-labelledby={`cookbook-tab-${tab}`}>
      {!native && <p className="cookbook-callout">Mở ứng dụng desktop để đọc cấu hình máy, tải và nạp model.</p>}
      {error && <p className="cookbook-callout cookbook-error" role="alert">{error}</p>}
      {catalogBusy && <p className="cookbook-callout" role="status">Đang tải danh mục online từ Ollama và Hugging Face (có thể mất khoảng 1 phút); bảng tạm hiện {catalog.source === "bundled" ? "snapshot kèm app, chỉ có Ollama" : "dữ liệu trước đó"}.</p>}
      {catalogError && <p className="cookbook-callout" role="status">Catalog: {catalogError}</p>}
      {chatBusy && <p className="cookbook-callout">Đợi hội thoại hoàn tất hoặc dừng trả lời để nạp/giải phóng model.</p>}
      {operation && <section className="setup-progress" aria-label="Tiến độ model">
        <div><strong>{stepLabels[operation.type]} · {operation.model}</strong>
          {working && operation.operationId && <button className="secondary-button" disabled={cancelling} onClick={() => void cancel()}><Square size={12} />{cancelling ? "Đang ngắt…" : "Ngắt"}</button>}
          {operation.type === "ready" && <button className="secondary-button" onClick={onChat}>Hội thoại <ArrowRight size={12} /></button>}
          {operation.type === "downloaded" && <button className="secondary-button" onClick={() => setTab("launch")}>Launch <ArrowRight size={12} /></button>}
        </div>
        <p role="status">{operation.status || (operation.type === "loading" ? "Đang nạp và xác nhận model sẵn sàng." : operation.type === "ready" ? "Model đã được chọn cho hội thoại." : operation.type === "downloaded" ? "Đã tải model. Mở Launch để nạp." : "")}</p>
        {operation.type === "progress" && <><progress max={100} value={percent} aria-label="Tiến độ layer hiện tại" /><small>{hasProgress ? `${percent}% layer · ${memory(operation.completed)} / ${memory(operation.total)}` : "Đang nhận thông tin tải…"}{operation.digest ? ` · ${operation.digest.slice(0, 16)}…` : ""}</small></>}
        {operation.type === "cancelled" && <small>App đã ngắt yêu cầu; Ollama có thể vẫn xử lý phía server. Cache được giữ lại.</small>}
      </section>}

      {tab === "download" && <details className="cookbook-direct">
        <summary><h2>Direct Download</h2><Download size={13} aria-hidden="true" /></summary>
        <p>Nhập tag Ollama hoặc repo GGUF Hugging Face (hf.co/user/repo:Q4_K_M, có thể dán link huggingface.co). Mở Launch khi model đã tải xong.</p>
        <form onSubmit={(event) => {
          event.preventDefault();
          // A pasted Hugging Face page URL becomes Ollama's hf.co name.
          const tag = directTag.trim().replace(/^https:\/\/(?:www\.)?huggingface\.co\//i, "hf.co/").replace(/\/$/, "");
          if (!/^(?:hf\.co\/[a-zA-Z0-9_][a-zA-Z0-9._-]*\/[a-zA-Z0-9_][a-zA-Z0-9._-]*|[a-zA-Z0-9_][a-zA-Z0-9._-]*(?:\/[a-zA-Z0-9_][a-zA-Z0-9._-]*)?)(?::[a-zA-Z0-9._-]+)?$/.test(tag)) { setError("Nhập tag Ollama (qwen3:4b) hoặc repo GGUF Hugging Face (hf.co/user/repo:Q4_K_M)."); return; }
          void setup(tag, true);
        }}>
          <label className="sr-only" htmlFor="cookbook-direct-tag">Tên model Ollama</label>
          <input id="cookbook-direct-tag" value={directTag} onChange={(event) => setDirectTag(event.target.value)} placeholder="qwen3:4b hoặc hf.co/user/repo:Q4_K_M" maxLength={200} autoComplete="off" disabled={working} />
          <button className="primary-button" disabled={disabled || !directTag.trim()}><Download size={14} />Download</button>
        </form>
      </details>}

      {["download", "launch", "active"].includes(tab) && <section className="cookbook-scan">
        <div className="cookbook-section-title"><h2>{tab === "download" ? "Scan / Download" : tab === "launch" ? "Launch model" : "Active models"}</h2><span>{tab === "active" ? `${running.length} đang nạp` : `${installed.length} đã tải`}</span></div>
        <p className="cookbook-section-help">{tab === "download" ? "Đánh giá bộ nhớ theo cấu hình hiện tại. Quét lại khi cần cập nhật RAM/VRAM trống." : tab === "launch" ? "Các model đã tải trên Ollama. Nạp model để dùng trong hội thoại." : "Trạng thái nạp thực tế từ Ollama; giải phóng model để trả lại bộ nhớ."}</p>
        <div className="cookbook-toolbar">
          <span className="cookbook-local"><span className={`cookbook-dot ${snapshot?.ollamaAvailable ? "connected" : ""}`} />Local · Ollama</span>
          <form className="cookbook-search" role="search" onSubmit={(event) => { event.preventDefault(); if (tab === "download") void searchOnline(); }}>
            <Search size={14} aria-hidden="true" /><label className="sr-only" htmlFor="cookbook-query">Tìm model</label>
            <input id="cookbook-query" value={query} onChange={(event) => setQuery(event.target.value)} placeholder={tab === "download" ? "Tìm model · Enter để tìm online" : "Tìm model…"} maxLength={80} />
            {foundQuery && <button type="button" className="icon-button" aria-label="Bỏ kết quả tìm online" onClick={clearSearch}><X size={13} /></button>}
          </form>
          {tab === "download" && <button className="secondary-button" disabled={!native || searching || !query.trim()} onClick={() => void searchOnline()}><Globe size={14} />{searching ? "Đang tìm…" : "Tìm online"}</button>}
          <button className="secondary-button" disabled={!native || loading || working} onClick={() => void refresh()}><RefreshCw size={14} />{loading ? "Đang quét…" : "Scan"}</button>
        </div>
        <div className="cookbook-controls">
          <label><span className="sr-only">Lọc model</span><select value={filter} onChange={(event) => setFilter(event.target.value)}><option value="all">Tất cả model</option><option value="installed">Đã tải</option><option value="tools">Tools</option><option value="vision">Vision</option><option value="embedding">Embedding</option><option value="thinking">Thinking</option></select></label>
          <label><span className="sr-only">Nguồn model</span><select value={provider} onChange={(event) => setProvider(event.target.value as Provider)}><option value="all">Mọi nguồn</option><option value="ollama">Ollama</option><option value="huggingface">Hugging Face</option></select></label>
          <div className="assessment-toggle" aria-label="Bộ nhớ để đánh giá">{(["auto", "ram", "gpu"] as const).map((mode) => <button key={mode} aria-pressed={assessment === mode} onClick={() => setAssessment(mode)}>{mode === "auto" ? "Auto" : mode.toUpperCase()}</button>)}</div>
          <label className="cookbook-context">Context<select aria-label="Context cho model" value={contextSize} disabled={working || chatBusy || !native} onChange={(event) => onContext(Number(event.target.value))}><option value={4096}>4K</option><option value={8192}>8K</option><option value={16384}>16K</option></select></label>
          <span className="cookbook-table-count">{shownRows.length} model · estimate</span>
        </div>
        <div className="catalog-toolbar"><label><input type="checkbox" checked={fitOnly} disabled={!hw} onChange={(event) => setFitOnly(event.target.checked)} />Chỉ model có khả năng chạy</label><span>{catalog.source === "online" ? "Ollama + Hugging Face online" : catalog.source === "cache" ? "Cache online" : "Snapshot kèm app"} · {catalog.models.length} mục · {catalog.updatedAt ? new Date(catalog.updatedAt).toLocaleDateString("vi-VN", { timeZone: "Asia/Bangkok" }) : "chưa có ngày"}</span><button className="secondary-button" disabled={!native || catalogBusy || working} onClick={() => void refreshCatalog(true)}><RefreshCw size={13} />{catalogBusy ? "Đang cập nhật…" : "Cập nhật catalog"}</button></div>
        <div className="cookbook-hardware-chips" aria-label="Cấu hình máy">
          <span><Cpu size={12} />{hw?.gpuName ?? "GPU chưa xác định"}</span>
          <span>VRAM {memory(hw?.gpuFreeBytes)} / {memory(hw?.gpuTotalBytes)}</span>
          <span>RAM {memory(hw?.ramAvailableBytes)} / {memory(hw?.ramTotalBytes)}</span>
          <span><HardDrive size={12} />{memory(hw?.diskAvailableBytes)} trống</span>
        </div>
        <div className="cookbook-table-wrap">
          <table className="cookbook-table"><caption className="sr-only">Model Ollama/Hugging Face và đánh giá bộ nhớ</caption>
            <thead><tr>{sortColumn("fit", "FIT")}{sortColumn("model", "MODEL")}{sortColumn("memory", tab === "active" ? "VRAM" : "EST. GiB")}{sortColumn("params", "PARAMS")}{sortColumn("quant", "QUANT")}{sortColumn("context", "CTX")}{sortColumn("speed", "SPEED†")}{sortColumn("score", "FIT SCORE")}{sortColumn("source", "NGUỒN")}<th scope="col"><span className="sr-only">Thao tác</span></th></tr></thead>
            <tbody>{shownRows.map((row) => {
              const fit = fitFor(row);
              const live = running.find((model) => model.name === row.id);
              return <tr key={row.id} className={detailId === row.id ? "is-selected" : ""}>
                <td><span className={`cookbook-fit ${fit?.status ?? "unknown"}`}>{fit ? fitLabels[fit.status] : "Chưa rõ"}</span></td>
                <td><button className="cookbook-model-name" aria-expanded={detailId === row.id} onClick={() => setDetailId(detailId === row.id ? "" : row.id)}><Cpu size={13} /><span>{row.id}</span>{selectedModel === row.id ? <Check size={12} aria-label="Đang chọn" /> : row.local ? <span className="cookbook-installed-dot" aria-label="Đã tải" /> : null}</button></td>
                <td>{tab === "active" ? memory(live?.sizeVramBytes) : rowMemory(row) == null ? "—" : `${rowMemory(row)!.toFixed(1)}${row.catalog ? "*" : "†"}`}</td>
                <td>{rowParams(row)}</td><td>{rowQuant(row)}</td><td title={`Context đang chọn ${contextSize}; tối đa công bố ${row.catalog?.maxContext ?? "chưa rõ"}`}>{live?.contextLength ? `${live.contextLength / 1024}K` : `${Math.min(contextSize, row.catalog?.maxContext ?? contextSize) / 1024}K*`}</td><td title={observation(row) ? `Lượt chat ${observation(row)!.measuredAt}, context ${contextSize}; tốc độ eval Ollama` : "Chưa có lượt chat được ghi nhận ở context này"}>{observation(row)?.tokensPerSecond.toFixed(1) ?? "—"}{observation(row) ? " t/s" : ""}</td><td title="Điểm phù hợp bộ nhớ, không phải chất lượng model">{fit?.score ?? "—"}</td><td title={row.catalog?.downloads != null ? `${row.catalog.downloads.toLocaleString("vi-VN")} lượt tải trên Hugging Face; chạy qua Ollama` : "Thư viện Ollama"}>{providerOf(row.id, row.catalog) === "huggingface" ? `HF${row.catalog?.downloads != null ? ` · ${compact(row.catalog.downloads)}` : ""}` : "Ollama"}</td>
                <td className="cookbook-row-actions">{tab === "active" ? <button className="secondary-button" aria-label={`Giải phóng ${row.id}`} disabled={disabled} onClick={() => void unload(row.id)}><Unplug size={13} />Unload</button> : tab === "launch" ? <button className="secondary-button" disabled={disabled || embedding(row)} title={embedding(row) ? "Embedding dùng cho memory; chưa có luồng chat cho model này" : "Nạp và chọn cho hội thoại"} onClick={() => void setup(row.id)}>Run <ArrowRight size={12} /></button> : <button className="secondary-button" disabled={disabled || Boolean(row.local)} onClick={() => void setup(row.id, true)}>{row.local ? <Check size={12} /> : <Download size={12} />}{row.local ? "Đã tải" : "Download"}</button>}</td>
              </tr>;
            })}</tbody>
          </table>
          {shownRows.length === 0 && <p className="cookbook-empty">{tab === "active" ? "Chưa có model đang nạp." : tab === "launch" ? "Chưa có model đã tải. Chọn model trong Download." : "Không có model khớp bộ lọc."}</p>}
        </div>
        <p className="cookbook-table-note">* Bộ nhớ/context ước lượng · † SPEED từ lượt chat thực tế, chưa đo hiển thị —. FIT SCORE chỉ đánh giá bộ nhớ: 50 × ngân sách / yêu cầu, giới hạn 0–100; không phải điểm chất lượng.</p>
        {detail && <section className="cookbook-model-detail">
          <div><strong>{detail.id}</strong><button aria-label="Đóng chi tiết model" onClick={() => setDetailId("")}>Đóng</button></div>
          <p>{detailFit?.reason ?? "Model này chưa có đánh giá phần cứng trong catalog."}</p>
          {detail.catalog && <><p>{detail.catalog.description} Capabilities: {detail.catalog.capabilities.join(" · ")}. Khả năng model không đồng nghĩa app đã có executor/đầu vào ảnh.</p><p>Nguồn {providerLabels[providerOf(detail.id, detail.catalog)]} · tải xấp xỉ {(detail.catalog.downloadBytes / 1e9).toFixed(1)} GB · kiểm tra {detail.catalog.verifiedAt} <a href={detail.catalog.source} target="_blank" rel="noreferrer">Nguồn <ExternalLink size={11} /></a></p></>}
          {detailFit && <p>Context gợi ý: {detailFit.contextTokens / 1024}K. Đây là ước lượng, chưa phải benchmark.</p>}
          {detailFit && <p>{detailFit.assumptions}</p>}
          {detail.catalog && <p>Metadata: {detail.catalog.metadataStatus ?? "snapshot"} · params {detail.catalog.parameterSize ?? "chưa rõ"} · context tối đa {detail.catalog.maxContext?.toLocaleString("vi-VN") ?? "chưa rõ"} · digest {detail.catalog.digest?.slice(0, 24) ?? "chưa rõ"}.</p>}
          {embedding(detail) && <p>Model embedding phục vụ truy xuất memory; hiện chỉ cho tải, không chọn làm model chat.</p>}
          {liveDetail && <p>Đã nạp: {memory(liveDetail.sizeVramBytes)} VRAM · context {liveDetail.contextLength ?? "chưa xác định"}.</p>}
        </section>}
      </section>}

      {tab === "dependencies" && <section className="cookbook-config-section">
        <div className="cookbook-section-title"><h2>Dependencies</h2><button className="secondary-button" disabled={!native || loading || working} onClick={() => void refresh()}><RefreshCw size={13} />Kiểm tra</button></div>
        <div className="cookbook-dependency-row"><Box size={18} /><div><strong>Ollama</strong><small>127.0.0.1:11434</small></div><span>{snapshot?.ollamaAvailable ? "Đã kết nối" : "Chưa kết nối"}</span></div>
        <p className="cookbook-section-help">{snapshot?.version ? `Phiên bản ${snapshot.version}` : "Phiên bản chưa xác định."}</p>
        <p className="cookbook-section-help">Cookbook hiện dùng Ollama đã cài trên máy. Cài hoặc mở Ollama, rồi kiểm tra lại để tải và nạp model.</p>
        <a className="cookbook-external" href="https://ollama.com/download" target="_blank" rel="noreferrer">Tải Ollama <ExternalLink size={12} /></a>
      </section>}
      {tab === "settings" && <section className="cookbook-config-section">
        <h2>Settings</h2>
        <div className="cookbook-setting-row"><div><strong>Ngân sách context</strong><p>Lịch sử, ghi chú Obsidian và phần trả lời dùng chung ngân sách này.</p></div><select aria-label="Ngân sách context" value={contextSize} disabled={working || chatBusy || !native} onChange={(event) => onContext(Number(event.target.value))}><option value={4096}>4K tokens</option><option value={8192}>8K tokens</option><option value={16384}>16K tokens</option></select></div>
        <div className="cookbook-setting-row"><div><strong>Thư mục model</strong><p>{hw?.modelDirectory ?? "Chưa xác định"}{hw?.modelDirectorySource === "default-estimate" ? " · đường dẫn mặc định ước lượng" : ""}</p></div></div>
        <div className="cookbook-setting-row"><div><strong>Cấu hình phát hiện</strong><p>{hw?.cpu ?? "CPU chưa xác định"}</p>{hw?.notes.map((note) => <p key={note}>{note}</p>)}</div></div>
      </section>}
    </div>
    <footer className="cookbook-statusbar"><span><span className={`cookbook-dot ${snapshot?.ollamaAvailable ? "connected" : ""}`} />{snapshot?.ollamaAvailable ? "Ollama connected" : "Ollama chưa kết nối"}</span><button onClick={onChat}>{selectedModel || "Chưa chọn model"}{selectedModel && <span> · {selectedReady ? "ready" : "chưa nạp"}</span>}<ArrowRight size={11} /></button></footer>
  </section>;
}
