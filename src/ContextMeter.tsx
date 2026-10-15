import { useEffect, useRef, useState } from "react";
import type { ContextUsage } from "./context-usage";

const k = (value: number) => value >= 1000 ? `${(value / 1000).toFixed(1).replace(/\.0$/, "")}k` : String(value);
const n = (value: number) => value.toLocaleString("vi-VN");
const pct = (part: number, whole: number) => whole > 0 ? Math.min(100, Math.max(0, part / whole * 100)) : 0;
const windowLabel = (size: number) => size >= 1024 ? `${Math.round(size / 1024)}K` : String(size);

type Props = {
  usage: ContextUsage;
  memory: boolean;
  autoCompact: boolean;
  /** Undefined when compaction is not possible right now (preview, running, no model). */
  onCompact?: () => Promise<void>;
};

/** Topbar meter: what the next request carries against the window, like a harness /context view. */
export function ContextMeter({ usage, memory, autoCompact, onCompact }: Props) {
  const [open, setOpen] = useState(false);
  const [details, setDetails] = useState(false);
  const [compacting, setCompacting] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (event: PointerEvent | KeyboardEvent) => {
      if (event instanceof KeyboardEvent ? event.key === "Escape" : !root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", close);
    document.addEventListener("keydown", close);
    return () => { document.removeEventListener("pointerdown", close); document.removeEventListener("keydown", close); };
  }, [open]);

  const { contextSize, reservedOutput, nextPromptTokens } = usage;
  const level = !usage.draftFits ? "full" : usage.droppedMessages > 0 || usage.overThreshold ? "warn" : "ok";
  const free = Math.max(0, contextSize - reservedOutput - nextPromptTokens);
  const share = Math.round(pct(nextPromptTokens, contextSize));
  const parts = [
    { key: "system", label: "Hướng dẫn hệ thống", hint: "System prompt của phiên", value: usage.systemTokens },
    { key: "summary", label: "Tóm tắt", hint: usage.summarizedMessages ? `${usage.summarizedMessages} tin cũ đã được thu gọn` : "Chưa thu gọn", value: usage.summaryTokens },
    { key: "prompt", label: "Hội thoại", hint: `${usage.includedMessages} tin gần nhất gửi kèm`, value: usage.historyTokens },
    { key: "draft", label: "Tin đang soạn", hint: "Ước lượng", value: usage.draftTokens },
    { key: "output", label: "Giữ cho output", hint: "Suy nghĩ và trả lời của Lim", value: reservedOutput },
    { key: "free", label: "Còn trống", hint: "Chỗ cho các lượt tiếp theo", value: free },
  ];
  const status = !usage.draftFits
    ? "Tin đang soạn quá dài so với context. Rút gọn tin, hoặc tăng Context window trong Tham số chạy model."
    : usage.overThreshold && autoCompact
      ? "Sắp đầy: lượt gửi tới Lim sẽ tự tóm tắt các tin cũ để giữ chỗ cho hội thoại."
      : usage.droppedMessages > 0
        ? `Đã đầy: Lim không còn thấy ${usage.droppedMessages} tin cũ nhất. Bấm Thu gọn ngay để giữ ý chính của chúng.`
        : usage.summarizedMessages
          ? `Còn đủ chỗ. ${usage.summarizedMessages} tin cũ đang được nhớ qua bản tóm tắt.`
          : "Còn đủ chỗ: Lim vẫn nhớ toàn bộ cuộc trò chuyện này.";

  async function compact() {
    if (!onCompact || compacting) return;
    setCompacting(true);
    try { await onCompact(); } finally { setCompacting(false); }
  }

  return <div className="context-meter" ref={root}>
    <button className={`context-meter-button ${level}`} aria-expanded={open} aria-haspopup="dialog"
      aria-label={`Context ước lượng ${n(nextPromptTokens)} trên ${n(contextSize)} token`} onClick={() => setOpen((value) => !value)}>
      <span className="context-meter-bar" aria-hidden="true"><span className="used" style={{ width: `${pct(nextPromptTokens, contextSize)}%` }} /><span className="reserved" style={{ width: `${Math.min(100 - pct(nextPromptTokens, contextSize), pct(reservedOutput, contextSize))}%` }} /></span>
      <span>{k(nextPromptTokens)} / {windowLabel(contextSize)}</span>
    </button>
    {open && <div className={`context-panel ${level}`} role="dialog" aria-label="Bộ nhớ hội thoại">
      <section>
        <div className="context-row"><strong>Bộ nhớ hội thoại</strong><span>{k(nextPromptTokens)} / {windowLabel(contextSize)} ({share}%)</span></div>
        <div className="context-stack" role="img" aria-label={parts.map((part) => `${part.label} ${n(part.value)} token`).join(", ")}>
          {parts.filter((part) => part.key !== "free").map((part) => <span key={part.key} className={`seg-${part.key}`} style={{ width: `${pct(part.value, contextSize)}%` }} />)}
        </div>
        <p className={`context-status ${level}`}>{status}</p>
      </section>
      <section>
        <dl className="context-parts">
          {parts.filter((part) => part.value > 0 || ["prompt", "output", "free"].includes(part.key)).map((part) => <div key={part.key} title={part.hint}><dt><i className={`seg-${part.key}`} aria-hidden="true" /><span>{part.label}<small>{part.hint}</small></span></dt><dd>{n(part.value)}</dd></div>)}
        </dl>
        <div className="context-actions">
          <button className="secondary-button" disabled={!onCompact || !usage.canCompact || compacting} title={usage.canCompact ? "Tóm tắt các lượt cũ bằng model hiện tại, giữ nguyên các lượt gần nhất" : "Cần ít nhất 2 lượt hỏi–đáp cũ để thu gọn"} onClick={() => void compact()}>{compacting ? "Đang thu gọn…" : "Thu gọn ngay"}</button>
          <small>{autoCompact ? `Tự thu gọn khi hội thoại vượt ~${n(usage.compactAt)} token (75%).` : "Tự thu gọn đang tắt; khi đầy, Lim bỏ các tin cũ nhất."}</small>
        </div>
      </section>
      <section>
        <button className="context-details-button" aria-expanded={details} onClick={() => setDetails((value) => !value)}>{details ? "Ẩn chi tiết kỹ thuật" : "Chi tiết kỹ thuật"}</button>
        {details && <div className="context-details">
          <dl>
            <div><dt>Ngân sách đầu vào</dt><dd>{n(usage.inputBudget)}</dd></div>
            <div><dt>Hội thoại chưa thu gọn</dt><dd>~{n(usage.conversationTokens)}</dd></div>
            <div><dt>Tin gửi kèm / bị bỏ</dt><dd>{usage.includedMessages} / {usage.droppedMessages}</dd></div>
            <div><dt>Prompt / output lượt trước</dt><dd>{usage.lastPromptTokens != null ? `${n(usage.lastPromptTokens)} / ${n(usage.lastOutputTokens ?? 0)}` : "chưa có"}</dd></div>
          </dl>
          <small>Ước lượng ~3 byte/token (Ollama không có API đếm token); "Prompt lượt trước" là số Ollama thực đếm. Ngân sách đầu vào = context − output − system prompt − 256{memory ? "; ghi chú Obsidian dùng chung ngân sách này" : ""}. Khi vượt 75%, các lượt cũ được model tóm tắt; nếu vẫn thiếu chỗ, tin cũ nhất bị bỏ.</small>
        </div>}
      </section>
    </div>}
  </div>;
}
