import type { Message } from "./state";

const encoder = new TextEncoder();
export const utf8Bytes = (text: string) => encoder.encode(text).length;
// Mirrors storage.rs estimate_tokens: Ollama has no tokenizer API, so ~3 UTF-8 bytes per token
// plus a few tokens of chat framing per message.
export const estimateTokens = (text: string) => Math.ceil(utf8Bytes(text) / 3) + 4;

/** Mirrors runtime.rs: compaction starts at 75% of the input budget. */
export const COMPACT_AT = 0.75;
export const summaryBlock = (summary: string) => `Tóm tắt phần hội thoại trước (đã thu gọn để tiết kiệm context):\n${summary}`;

export type ContextUsage = {
  contextSize: number;
  /** Output limit kept free in the window (thinking + answer). */
  reservedOutput: number;
  systemTokens: number;
  summaryTokens: number;
  /** Conversation tokens that fit in the next request (newest first). */
  historyTokens: number;
  draftTokens: number;
  /** Tokens left for summary + conversation after system prompt, output and framing. */
  inputBudget: number;
  nextPromptTokens: number;
  includedMessages: number;
  droppedMessages: number;
  /** Messages folded into the summary so far. */
  summarizedMessages: number;
  draftFits: boolean;
  /** Summary + all eligible conversation, compared with the compaction threshold. */
  conversationTokens: number;
  compactAt: number;
  overThreshold: boolean;
  /** At least two older turns exist that compaction could fold. */
  canCompact: boolean;
  lastPromptTokens: number | null;
  lastOutputTokens: number | null;
};

const usable = (m?: Message) => m?.role === "assistant" && m.text.trim() !== "" && (m.status === "completed" || m.status === "truncated");

/** Same budgeting as runtime.rs generate(): system + summary + newest answered turns + draft. */
export function contextUsage(
  session: { messages: Message[]; summary?: string; summaryUntil?: string },
  draft: string,
  contextSize: number,
  numPredict: number,
  systemPrompt: string,
): ContextUsage {
  const systemTokens = systemPrompt ? estimateTokens(systemPrompt) : 0;
  const inputBudget = Math.max(0, contextSize - numPredict - 256 - estimateTokens(systemPrompt));
  const summaryTokens = session.summary?.trim() ? estimateTokens(summaryBlock(session.summary)) : 0;
  const cutIndex = session.summaryUntil ? session.messages.findIndex((m) => m.id === session.summaryUntil) : -1;
  const summarizedMessages = cutIndex + 1;
  const visible = session.messages.slice(cutIndex + 1).filter((m) => m.source !== "ui_preview");
  // Mirrors runtime.rs conversation(): answered turns only; the draft becomes the latest question.
  const eligible = visible.filter((m, i) => m.text && (m.role === "user" ? usable(visible[i + 1]) : usable(m) && visible[i - 1]?.role === "user"));
  const pending = draft.trim() ? [{ role: "user", text: draft.trim() } as Message] : [];
  const candidates = [...eligible, ...pending];
  let remaining = Math.max(0, inputBudget - summaryTokens);
  let included = 0;
  let used = 0;
  for (let i = candidates.length - 1; i >= 0; i--) {
    const cost = estimateTokens(candidates[i].text);
    if (cost > remaining) break;
    remaining -= cost; used += cost; included++;
  }
  const draftTokens = pending.length ? estimateTokens(pending[0].text) : 0;
  const draftFits = !pending.length || draftTokens <= Math.max(0, inputBudget - summaryTokens);
  const includedMessages = Math.max(0, included - (pending.length && draftFits ? 1 : 0));
  const conversationTokens = summaryTokens + candidates.reduce((sum, m) => sum + estimateTokens(m.text), 0);
  const compactAt = Math.floor(inputBudget * COMPACT_AT);
  const last = [...session.messages].reverse().find((m) => usable(m) && m.metrics?.promptTokens != null);
  return {
    contextSize, reservedOutput: numPredict, systemTokens, summaryTokens,
    historyTokens: Math.max(0, used - (pending.length && draftFits ? draftTokens : 0)), draftTokens, inputBudget,
    nextPromptTokens: systemTokens + summaryTokens + used,
    includedMessages, droppedMessages: eligible.length - includedMessages, summarizedMessages, draftFits,
    conversationTokens, compactAt, overThreshold: conversationTokens > compactAt,
    canCompact: eligible.length >= 4,
    lastPromptTokens: last?.metrics?.promptTokens ?? null, lastOutputTokens: last?.metrics?.outputTokens ?? null,
  };
}
