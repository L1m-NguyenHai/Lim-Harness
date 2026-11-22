import { useCallback, useEffect, useRef, useState } from "react";
import { agentSetup, type AgentSetupStatus } from "./core";
import { cookbook, type SetupEvent } from "./cookbook-api";

type Props = { available: boolean; notify: (message: string) => void; onModelsChanged: () => void };

/** One-click install of what Agent mode needs besides the chat model: the decision model
 *  (pulled through Ollama like Cookbook downloads) and the local SearXNG (scripts/setup-searxng.ps1). */
export function AgentSetup({ available, notify, onModelsChanged }: Props) {
  const [status, setStatus] = useState<AgentSetupStatus | null>(null);
  const [model, setModel] = useState<{ busy: boolean; percent?: number; text: string }>({ busy: false, text: "" });
  const [search, setSearch] = useState<{ busy: boolean; text: string }>({ busy: false, text: "" });
  const mounted = useRef(true);

  const refresh = useCallback(async () => {
    if (!available) return;
    try { const next = await agentSetup.status(); if (mounted.current) setStatus(next); }
    catch (error) { if (mounted.current) setSearch((s) => ({ ...s, text: String(error) })); }
  }, [available]);
  useEffect(() => { mounted.current = true; void refresh(); return () => { mounted.current = false; }; }, [refresh]);

  async function installModel() {
    if (!status) return;
    setModel({ busy: true, text: "Đang bắt đầu tải…" });
    const receive = (event: SetupEvent) => {
      if (!mounted.current) return;
      if (event.type === "progress") {
        const percent = event.total ? Math.round(((event.completed || 0) / event.total) * 100) : undefined;
        setModel({ busy: true, percent, text: event.status || "Đang tải" });
      } else if (event.type === "downloaded" || event.type === "ready") {
        setModel({ busy: false, text: `Đã cài ${event.model}.` });
        notify(`Đã cài model quyết định ${event.model}.`);
        onModelsChanged();
        void refresh();
      } else if (event.type === "failed" || event.type === "cancelled") {
        setModel({ busy: false, text: event.error || "Đã dừng tải." });
      } else if (event.status) {
        setModel((m) => ({ ...m, text: event.status || m.text }));
      }
    };
    try { await cookbook.download(status.decisionModel, receive); }
    catch (error) { setModel({ busy: false, text: String(error) }); }
  }

  async function installSearch() {
    setSearch({ busy: true, text: "Đang chuẩn bị…" });
    try {
      await agentSetup.installSearxng((event) => {
        if (!mounted.current) return;
        if (event.type === "log" && event.line) setSearch({ busy: true, text: event.line });
        else if (event.type === "failed") setSearch({ busy: false, text: event.error || "Cài đặt thất bại." });
        else if (event.type === "done") {
          setSearch({ busy: false, text: "Đã cài SearXNG, đang khởi động…" });
          notify("Đã cài SearXNG cho Agent.");
          // SearXNG needs a few seconds before it answers on 127.0.0.1:8888.
          window.setTimeout(() => void refresh(), 5000);
          void refresh();
        }
      });
    } catch (error) { setSearch({ busy: false, text: String(error) }); }
  }

  const searx = status?.searxng;
  const modelState = !status ? "…" : status.decisionInstalled === null ? "Không kết nối được Ollama" : status.decisionInstalled ? "Đã cài" : "Chưa cài";
  const searchState = !searx ? "…" : searx.running ? "Đang chạy" : searx.installed ? "Đã cài, chưa chạy" : "Chưa cài";
  const missingTools = searx && !searx.installed ? [!searx.git && "Git for Windows", !searx.python && "Python 3.10+ (python.org)"].filter(Boolean) : [];

  return (
    <section className="settings-section agent-setup">
      <h2>Agent: tra web và ra quyết định</h2>
      <p>Ngoài model chat, Agent dùng thêm một model nhỏ để hiểu yêu cầu và một máy tìm kiếm chạy trên máy. Thiếu chúng Agent vẫn chạy (model chat tự quyết, tìm qua DuckDuckGo/Bing) nhưng kém ổn định hơn.</p>
      <div className="setup-row">
        <div><strong>Model ra quyết định</strong><small>{status?.decisionModel || "granite4:3b"} · khoảng 2 GB · tải qua Ollama, chạy trên CPU</small></div>
        <span className={`setup-state ${status?.decisionInstalled ? "ok" : ""}`}>{modelState}</span>
        <button className="secondary-button" disabled={!available || !status || !status.ollama || !!status.decisionInstalled || model.busy} onClick={() => void installModel()}>{model.busy ? "Đang tải…" : status?.decisionInstalled ? "Đã cài" : "Cài"}</button>
      </div>
      {(model.busy || model.text) && <div className="setup-detail" role="status">{model.percent !== undefined && model.busy && <progress max={100} value={model.percent} />}<small>{model.text}{model.percent !== undefined && model.busy ? ` · ${model.percent}%` : ""}</small></div>}
      <div className="setup-row">
        <div><strong>SearXNG (tìm web cục bộ)</strong><small>127.0.0.1:8888 · cần Git và Python · cài vào thư mục .local của dự án</small></div>
        <span className={`setup-state ${searx?.running ? "ok" : ""}`}>{searchState}</span>
        <button className="secondary-button" disabled={!available || !searx || searx.installed || search.busy || searx.installing || missingTools.length > 0} onClick={() => void installSearch()}>{search.busy || searx?.installing ? "Đang cài…" : searx?.installed ? "Đã cài" : "Cài"}</button>
      </div>
      {missingTools.length > 0 && <div className="setup-detail"><small>Cần cài trước: {missingTools.join(", ")}. Sau đó bấm Kiểm tra lại.</small></div>}
      {search.text && <div className="setup-detail" role="status"><small>{search.text}</small></div>}
      <button className="secondary-button" disabled={!available || model.busy || search.busy} onClick={() => void refresh()}>Kiểm tra lại</button>
    </section>
  );
}

/** True when something Agent mode uses is missing (for the hint when Agent is turned on). */
export async function agentSetupMissing(): Promise<boolean> {
  try {
    const status = await agentSetup.status();
    return status.decisionInstalled === false || !status.searxng.installed;
  } catch { return false; }
}
