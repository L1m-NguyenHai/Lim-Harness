import { useEffect, useRef, useState } from "react";
import { RotateCcw, SlidersHorizontal, X } from "lucide-react";
import { defaultInference, native } from "./core";
import type { InferenceSettings } from "./core";
import "./inference.css";

type Props = { open: boolean; settings: InferenceSettings; contextSize: number; ready: boolean; model: string; onClose: () => void; onDirty: (dirty: boolean) => void; onSave: (settings: InferenceSettings, contextSize: number) => Promise<void> };
export function InferencePanel({ open, settings, contextSize, ready, model, onClose, onDirty, onSave }: Props) {
  const [draft, setDraft] = useState(settings);
  const [draftContext, setDraftContext] = useState(contextSize);
  const previousContext = useRef(contextSize);
  const pendingApplyContext = useRef<number | null>(null);
  const [contextNotice, setContextNotice] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { setDraft(settings); }, [settings]);
  const invalidNumbers = [draft.temperature, draft.topP, draft.topK, draft.repeatPenalty, draft.numPredict].some((value) => !Number.isFinite(value));
  const dirty = JSON.stringify(draft) !== JSON.stringify(settings) || draftContext !== contextSize;
  useEffect(() => { onDirty(dirty); }, [dirty, onDirty]);
  useEffect(() => {
    const previous = previousContext.current;
    previousContext.current = contextSize;
    setDraftContext((current) => current === previous ? contextSize : current);
    if (previous !== contextSize && pendingApplyContext.current !== contextSize && draftContext !== previous) setContextNotice("Context đã cập nhật từ Cookbook. Áp dụng sẽ dùng giá trị đang hiện trong panel.");
    if (pendingApplyContext.current === contextSize) pendingApplyContext.current = null;
  }, [contextSize]);
  const promptBytes = new TextEncoder().encode(draft.systemPrompt).length;
  const budget = draftContext - draft.numPredict - promptBytes - 256;
  const controls: { key: "temperature" | "topP" | "topK" | "repeatPenalty" | "numPredict"; label: string; min: number; max: number; step: number; hint: string }[] = [
    { key: "temperature", label: "Temperature", min: 0, max: 2, step: 0.05, hint: "Thấp: tập trung · Cao: đa dạng" },
    { key: "topP", label: "Top P", min: 0, max: 1, step: 0.05, hint: "Ngưỡng xác suất lấy mẫu" },
    { key: "topK", label: "Top K", min: 0, max: 100, step: 1, hint: "Số ứng viên token · 0 để tắt" },
    { key: "repeatPenalty", label: "Repeat penalty", min: 0, max: 2, step: 0.05, hint: "1: không phạt lặp" },
    { key: "numPredict", label: "Giới hạn output", min: 128, max: 8192, step: 1, hint: "Token tối đa, gồm cả reasoning" },
  ];
  async function save(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault(); setError(""); setSaving(true);
    pendingApplyContext.current = draftContext;
    try { await onSave(draft, draftContext); setContextNotice(""); } catch (reason) { pendingApplyContext.current = null; setError(String(reason)); }
    finally { setSaving(false); }
  }
  return <aside hidden={!open} className="inference-panel" aria-labelledby="inference-heading">
    <header className="inference-heading"><div><SlidersHorizontal size={16} aria-hidden="true" /><h2 id="inference-heading">Tham số chạy model</h2></div><button className="icon-button" aria-label="Ẩn tham số chạy model" onClick={onClose}><X size={16} /></button></header>
    <div className="inference-model"><span>OLLAMA LOCAL</span><strong title={model}>{model || "Chưa chọn model"}</strong></div>
    <form onSubmit={(event) => void save(event)}>
      <fieldset className="inference-fields" disabled={!ready || saving}>
        <section className="inference-section"><h3>Inference</h3>
          <label className="inference-control" htmlFor="inference-context"><span>Context window</span><select id="inference-context" value={draftContext} onChange={(event) => setDraftContext(Number(event.target.value))}><option value={4096}>4 096 token</option><option value={8192}>8 192 token</option><option value={16384}>16 384 token</option></select><small>Lưu cùng Áp dụng · tăng sẽ dùng thêm bộ nhớ</small></label>
          {controls.map(({ key, label, min, max, step, hint }) => <div className="inference-control" key={key}><div className="inference-control-top"><label htmlFor={`inference-${key}`}>{label}</label><input id={`inference-${key}`} type="number" min={min} max={max} step={key === "topK" || key === "numPredict" ? 1 : "any"} required value={Number.isFinite(draft[key]) ? draft[key] : ""} onChange={(event) => setDraft({ ...draft, [key]: event.target.valueAsNumber })} /></div>{key !== "numPredict" && <input type="range" aria-label={`${label} slider`} min={min} max={max} step={step} value={draft[key]} onChange={(event) => setDraft({ ...draft, [key]: Number(event.target.value) })} />}<small>{hint}</small></div>)}
          <label className="inference-control" htmlFor="inference-seed"><span>Seed</span><input id="inference-seed" type="number" min={0} max={2147483647} step={1} placeholder="Mặc định của model" value={draft.seed ?? ""} onChange={(event) => setDraft({ ...draft, seed: event.target.value === "" ? null : event.target.valueAsNumber })} /><small>Để trống để dùng mặc định của model</small></label>
        </section>
        <section className="inference-section"><h3>System prompt</h3><label className="sr-only" htmlFor="inference-system">Chỉ dẫn hệ thống cho model</label><textarea id="inference-system" rows={7} value={draft.systemPrompt} onChange={(event) => setDraft({ ...draft, systemPrompt: event.target.value })} placeholder="Chỉ dẫn về vai trò, cách trả lời…" /><small>Áp dụng cho mọi hội thoại từ lượt gửi tiếp theo.</small></section>
        {promptBytes > 12000 && <p className="inference-error" role="alert">System prompt tối đa 12 000 byte UTF-8.</p>}
        {budget < 64 && <p className="inference-error" role="alert">Context chưa đủ cho prompt và output. Tăng context hoặc rút gọn cấu hình.</p>}
      </fieldset>
        <footer className="inference-footer"><button type="button" className="secondary-button" disabled={!ready || saving} onClick={() => { setDraft({ ...defaultInference }); setDraftContext(4096); }}><RotateCcw size={14} /> Mặc định</button><button className="primary-button" disabled={!ready || saving || !dirty || invalidNumbers || promptBytes > 12000 || budget < 64}>{saving ? "Đang lưu…" : "Áp dụng"}</button><span role="status">{dirty ? "Có thay đổi chưa áp dụng" : !native ? "Bản xem trước · đã lưu trên trình duyệt" : "Đã lưu · áp dụng từ lượt tiếp theo"}</span>{contextNotice && <span className="inference-context-notice">{contextNotice}</span>}{error && <span role="alert">{error}</span>}</footer>
    </form>
  </aside>;
}
