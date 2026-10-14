import { useEffect, useRef, useState } from "react";
import type { ContextUsage } from "./context-usage";

const k = (value: number) => value >= 1000 ? `${(value / 1000).toFixed(1).replace(/\.0$/, "")}k` : String(value);
const n = (value: number) => value.toLocaleString("vi-VN");
const pct = (part: number, whole: number) => whole > 0 ? Math.min(100, Math.max(0, part / whole * 100)) : 0;

/** Topbar meter: estimated next-request tokens against the context window, with a breakdown popover. */
export function ContextMeter({ usage, memory }: { usage: ContextUsage; memory: boolean }) {
  const [open, setOpen] = useState(false);
  const [details, setDetails] = useState(false);
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
  const inputRoom = Math.max(0, contextSize - reservedOutput);
  const level = !usage.draftFits || nextPromptTokens > inputRoom ? "full" : nextPromptTokens > inputRoom * 0.75 || usage.droppedMessages > 0 ? "warn" : "ok";
  const free = Math.max(0, contextSize - reservedOutput - nextPromptTokens);
  const share = Math.round(pct(nextPromptTokens, contextSize));
  // Beginner view: three plain amounts that add up to the window, plus one status sentence.
  const parts = [
    { key: "prompt", label: "Đã dùng", hint: "Hướng dẫn cho Lim, hội thoại và tin đang soạn", value: nextPromptTokens },
    { key: "output", label: "Giữ cho câu trả lời", hint: "Chỗ để Lim suy nghĩ và viết câu trả lời tiếp theo", value: reservedOutput },
    { key: "free", label: "Còn trống", hint: "Chỗ cho các tin nhắn tiếp theo", value: free },
  ];
  const status = !usage.draftFits
    ? "Tin đang soạn quá dài so với chỗ còn lại. Hãy rút gọn tin, hoặc tăng context trong Tham số chạy model."
    : usage.droppedMessages > 0
      ? `Đã đầy: Lim sẽ không còn thấy ${usage.droppedMessages} tin nhắn cũ nhất. Tin vẫn được lưu, chỉ là Lim không đọc lại.`
      : level === "warn" ? "Sắp đầy: khi hết chỗ, Lim sẽ quên dần các tin nhắn cũ nhất." : "Còn đủ chỗ: Lim vẫn nhớ toàn bộ cuộc trò chuyện này.";
  return <div className="context-meter" ref={root}>
    <button className={`context-meter-button ${level}`} aria-expanded={open} aria-haspopup="dialog"
      aria-label={`Context ước lượng ${n(nextPromptTokens)} trên ${n(contextSize)} token`} onClick={() => setOpen((value) => !value)}>
      <span className="context-meter-bar" aria-hidden="true"><span className="used" style={{ width: `${pct(nextPromptTokens, contextSize)}%` }} /><span className="reserved" style={{ width: `${Math.min(100 - pct(nextPromptTokens, contextSize), pct(reservedOutput, contextSize))}%` }} /></span>
      <span>{k(nextPromptTokens)} / {contextSize / 1024}K</span>
    </button>
    {open && <div className={`context-panel ${level}`} role="dialog" aria-label="Bộ nhớ hội thoại">
      <section>
        <div className="context-row"><strong>Bộ nhớ hội thoại</strong><span>{k(nextPromptTokens)} / {contextSize / 1024}K ({share}%)</span></div>
        <div className="context-stack" role="img" aria-label={parts.map((part) => `${part.label} ${n(part.value)} token`).join(", ")}>
          {parts.filter((part) => part.key !== "free").map((part) => <span key={part.key} className={`seg-${part.key}`} style={{ width: `${pct(part.value, contextSize)}%` }} />)}
        </div>
        <p className={`context-status ${level}`}>{status}</p>
      </section>
      <section>
        <dl className="context-parts">
          {parts.map((part) => <div key={part.key} title={part.hint}><dt><i className={`seg-${part.key}`} aria-hidden="true" /><span>{part.label}<small>{part.hint}</small></span></dt><dd>{n(part.value)}</dd></div>)}
        </dl>
        <small className="context-tip">Đơn vị là token, khoảng 3–4 ký tự mỗi token. Muốn Lim nhớ nhiều hơn: mở phiên mới hoặc tăng context.</small>
      </section>
      <section>
        <button className="context-details-button" aria-expanded={details} onClick={() => setDetails((value) => !value)}>{details ? "Ẩn chi tiết kỹ thuật" : "Chi tiết kỹ thuật"}</button>
        {details && <div className="context-details">
          <dl>
            <div><dt>Tin cũ gửi kèm / bị bỏ</dt><dd>{usage.includedMessages} / {usage.droppedMessages}</dd></div>
            <div><dt>Tin đang soạn</dt><dd>~{n(usage.draftTokens)}</dd></div>
            <div><dt>Prompt / trả lời lượt trước</dt><dd>{usage.lastPromptTokens != null ? `${n(usage.lastPromptTokens)} / ${n(usage.lastOutputTokens ?? 0)}` : "chưa có"}</dd></div>
            <div><dt>Ngân sách lịch sử</dt><dd>{n(usage.historyUsedBytes)} / {n(usage.historyBudgetBytes)} B</dd></div>
          </dl>
          <small>{usage.measured ? "\"Đã dùng\" lấy số token Ollama trả về ở lượt trước, cộng ước lượng ~3 byte/token cho tin mới." : "Chưa có số đo từ Ollama; ước lượng ~3 byte/token."} App cắt lịch sử theo byte (context − output − system prompt − 256){memory ? "; trích đoạn Obsidian dùng chung ngân sách, tối đa ~4.500 byte" : ""}.</small>
        </div>}
      </section>
    </div>}
  </div>;
}
