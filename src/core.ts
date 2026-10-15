import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import type { Message, Session } from "./state";

export const native = isTauri();
export type VaultHit = { path: string; title: string; excerpt: string };
export type VaultSearchResult = {
  hits: VaultHit[];
  scannedFiles: number;
  skippedFiles: number;
  truncated: boolean;
};
export type InferenceSettings = { systemPrompt: string; temperature: number; topP: number; topK: number; repeatPenalty: number; numPredict: number; seed: number | null; autoCompact: boolean };
export type RunUsage = { thinkingTokens: number; answerTokens: number; budget: number };
export const defaultInference: InferenceSettings = {
  systemPrompt: "Bạn là Lim, trợ lý AI chạy trên máy của người dùng. Mặc định trả lời bằng tiếng Việt, tự nhiên và ngắn gọn. Bạn chỉ trò chuyện bằng văn bản, không chạy lệnh và không đọc hay sửa tệp.",
  temperature: 0.7, topP: 0.9, topK: 40, repeatPenalty: 1.1, numPredict: 2048, seed: null, autoCompact: true,
};
export type Snapshot = { sessions: Session[]; model: string; vaultPath: string; contextSize: number; inferenceSettings: InferenceSettings };
export type RunEvent = {
  type: "started" | "text_delta" | "thinking_delta" | "metrics" | "memory_sources" | "usage" | "compacting" | "compacted" | "notice" | "completed" | "truncated" | "cancelled" | "failed";
  sessionId: string;
  runId: string;
  sequence: number;
  delta?: string;
  error?: string;
  message?: Message;
  userMessage?: Message;
  sources?: VaultHit[];
  metrics?: import("./state").ChatMetrics;
  usage?: RunUsage;
  notice?: string;
  count?: number;
  truncated?: boolean;
  skippedFiles?: number;
};
export const core = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  importSessions: (sessions: Session[]) => invoke<void>("import_sessions", { sessions }),
  createSession: () => invoke<Session>("create_session"),
  deleteSession: (sessionId: string) => invoke<void>("delete_session", { sessionId }),
  /** Summarize older turns now; returns how many messages were folded. */
  compactSession: (sessionId: string, model: string) => invoke<number>("compact_session", { sessionId, model }),
  /** The model's trained context length from Ollama metadata, if known. */
  modelContext: (model: string) => invoke<number | null>("model_context", { model }),
  setSessionConfig: (sessionId: string, config: import("./state").SessionConfig) => invoke<void>("set_session_config", { sessionId, config }),
  models: () => invoke<string[]>("list_models"),
  setModel: (model: string) => invoke<void>("set_model", { model }),
  setContext: (contextSize: number) => invoke<void>("set_context_size", { contextSize }),
  setInference: (settings: InferenceSettings, contextSize: number) => invoke<void>("set_inference_settings", { settings, contextSize }),
  setVault: (path: string) => invoke<string>("set_vault", { path }),
  searchMemory: (query: string) => invoke<VaultSearchResult>("search_memory", { query }),
  start: (sessionId: string, text: string, model: string, receive: (event: RunEvent) => void) => {
    const onEvent = new Channel<RunEvent>();
    onEvent.onmessage = receive;
    return invoke<string>("start_turn", { sessionId, text, model, onEvent });
  },
  /** Continues an answer that stopped at the output limit, in a new run with a fresh budget. */
  continueTurn: (sessionId: string, messageId: string, model: string, receive: (event: RunEvent) => void) => {
    const onEvent = new Channel<RunEvent>();
    onEvent.onmessage = receive;
    return invoke<string>("continue_turn", { sessionId, messageId, model, onEvent });
  },
  cancel: (sessionId: string, runId: string) => invoke<boolean>("cancel_turn", { sessionId, runId }),
};
