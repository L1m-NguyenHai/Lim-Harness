import { useState } from "react";
import { Check, FileText, FolderOpen, Globe, PenLine, Search, ShieldAlert, SquareTerminal, X } from "lucide-react";
import type { AgentStep } from "./state";

const icons: Record<string, typeof Search> = { web_search: Search, fetch_url: Globe, list_dir: FolderOpen, read_file: FileText, write_file: PenLine, run_command: SquareTerminal };
const statusLabels: Record<AgentStep["status"], string> = { waiting: "Chờ cho phép", running: "Đang chạy", done: "Xong", denied: "Đã từ chối", error: "Lỗi" };

/** Tool steps of an agent answer; risky steps show Allow/Deny until the user decides. */
export function AgentSteps({ steps, onResolve }: { steps: AgentStep[]; onResolve?: (approvalId: string, allow: boolean) => Promise<void> }) {
  const [busy, setBusy] = useState("");
  async function resolve(approvalId: string, allow: boolean) {
    if (!onResolve || busy) return;
    setBusy(approvalId);
    try { await onResolve(approvalId, allow); } finally { setBusy(""); }
  }
  return <ol className="agent-steps" aria-label="Các bước Agent">
    {steps.map((step) => {
      const Icon = icons[step.tool] ?? SquareTerminal;
      const detail = step.tool === "run_command" ? step.args?.command : step.tool === "write_file" ? step.args?.content : undefined;
      return <li key={step.id} className={`agent-step ${step.status}`}>
        <span className="agent-step-icon" aria-hidden="true"><Icon size={13} /></span>
        <div className="agent-step-body">
          <div className="agent-step-head"><span className="agent-step-title">{step.title}</span><span className="agent-step-status">{statusLabels[step.status] ?? step.status}</span></div>
          {step.status === "waiting" && step.approvalId && <div className="agent-approval" role="group" aria-label="Xin phép thao tác">
            <p><ShieldAlert size={13} aria-hidden="true" />Lim muốn {step.tool === "run_command" ? "chạy lệnh này trên máy của bạn" : step.tool === "write_file" ? "ghi tệp này" : "đọc tệp này"}{step.tool === "run_command" && step.args?.cwd ? ` (thư mục ${step.args.cwd})` : ""}.</p>
            {detail && <pre>{String(detail).slice(0, 2000)}</pre>}
            <div className="agent-approval-actions">
              <button className="primary-button" disabled={Boolean(busy) || !onResolve} onClick={() => void resolve(step.approvalId!, true)}><Check size={13} aria-hidden="true" />Cho phép</button>
              <button className="secondary-button" disabled={Boolean(busy) || !onResolve} onClick={() => void resolve(step.approvalId!, false)}><X size={13} aria-hidden="true" />Từ chối</button>
            </div>
          </div>}
          {step.output && step.status !== "waiting" && <details className="agent-step-output"><summary>Kết quả</summary><pre>{step.output}</pre></details>}
        </div>
      </li>;
    })}
  </ol>;
}
