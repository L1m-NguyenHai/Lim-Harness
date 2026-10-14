import type { Message } from "./state";

const encoder = new TextEncoder();
export const utf8Bytes = (text: string) => encoder.encode(text).length;
// Rough token estimate for text Ollama has not counted yet. Vietnamese UTF-8 runs ~3 bytes/token.
export const estimateTokens = (text: string) => Math.ceil(utf8Bytes(text) / 3);

export type ContextUsage = {
  contextSize: number;
  /** Generation budget (thinking + answer) reserved in the same window. */
  reservedOutput: number;
  /** Prompt tokens Ollama counted on the latest completed turn, when reported. */
  lastPromptTokens: number | null;
  lastOutputTokens: number | null;
  /** Estimated tokens of the next request: last real count (or text estimate) + draft. */
  nextPromptTokens: number;
  measured: boolean;
  draftTokens: number;
  /** Mirrors runtime.rs input_budget/bounded_history, which budget in UTF-8 bytes. */
  historyBudgetBytes: number;
  historyUsedBytes: number;
  includedMessages: number;
  droppedMessages: number;
  draftFits: boolean;
};

export function contextUsage(messages: Message[], draft: string, contextSize: number, numPredict: number, systemPrompt: string): ContextUsage {
  const historyBudgetBytes = Math.max(0, contextSize - (numPredict + utf8Bytes(systemPrompt) + 256));
  // Mirrors runtime.rs conversation(): answered turns only; a question without a usable answer
  // is dropped (the draft becomes the latest question).
  const usable = (m?: Message) => m?.role === "assistant" && m.text.trim() !== "" && (m.status === "completed" || m.status === "truncated");
  const visible = messages.filter((m) => m.source !== "ui_preview");
  const eligible = visible.filter((m, i) => m.text && (m.role === "user" ? usable(visible[i + 1]) : usable(m) && visible[i - 1]?.role === "user"));
  const pending = draft.trim() ? [{ role: "user", text: draft.trim() } as Message] : [];
  const candidates = [...eligible, ...pending];
  let remaining = historyBudgetBytes;
  let included = 0;
  let used = 0;
  for (let i = candidates.length - 1; i >= 0; i--) {
    const cost = utf8Bytes(candidates[i].text) + 32;
    if (cost > remaining) break;
    remaining -= cost; used += cost; included++;
  }
  const draftFits = !pending.length || utf8Bytes(pending[0].text) + 32 <= historyBudgetBytes;
  const last = [...messages].reverse().find((m) => m.role === "assistant" && (m.status === "completed" || m.status === "truncated") && m.metrics?.promptTokens != null);
  const lastPromptTokens = last?.metrics?.promptTokens ?? null;
  const lastOutputTokens = last?.metrics?.outputTokens ?? null;
  const draftTokens = pending.length ? estimateTokens(pending[0].text) : 0;
  const base = lastPromptTokens != null
    ? lastPromptTokens + (lastOutputTokens ?? 0)
    : estimateTokens(systemPrompt) + eligible.reduce((sum, m) => sum + estimateTokens(m.text) + 4, 0);
  return {
    contextSize, reservedOutput: numPredict, lastPromptTokens, lastOutputTokens,
    nextPromptTokens: base + draftTokens, measured: lastPromptTokens != null, draftTokens,
    historyBudgetBytes, historyUsedBytes: used,
    includedMessages: Math.max(0, included - pending.length), droppedMessages: eligible.length - Math.max(0, included - pending.length),
    draftFits,
  };
}
