import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import type { Message, Session } from "./state";

export const native = isTauri();
export type VaultHit = { path: string; title: string; excerpt: string; memoryId?: string; revision?: string };
export type MemorySettings = { enabled: boolean; useMemory: boolean; autoMemory: boolean };
export type MemorySource = { sessionId: string; messageId: string; quote: string };
export type MemoryRecord = {
  id: string; path: string; revision: string; title: string; category: string;
  status: "active" | "retracted"; pinned: boolean; createdAt: string; updatedAt: string;
  content: string; sources: MemorySource[]; history: MemoryHistory[];
};
export type MemoryHistory = { title: string; category: string; content: string; status: string; pinned: boolean; updatedAt: string; sources: MemorySource[] };
export type MemoryJob = { id: string; sessionId: string; model: string; status: string; error: string | null; changedCount: number; createdAt: string };
export type MemorySnapshot = { settings: MemorySettings; records: MemoryRecord[]; diagnostics: string[]; truncated: boolean; jobs: MemoryJob[] };
export type VaultSearchResult = {
  hits: VaultHit[];
  scannedFiles: number;
  skippedFiles: number;
  truncated: boolean;
};
export type InferenceSettings = { systemPrompt: string; temperature: number; topP: number; topK: number; repeatPenalty: number; numPredict: number; seed: number | null; autoCompact: boolean };
export type RunUsage = { thinkingTokens: number; answerTokens: number; budget: number };
export const defaultInference: InferenceSettings = {
  systemPrompt: "You are a helpful assistant.",
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
  initializeMemoryVault: () => invoke<string>("initialize_memory_vault"),
  openMemoryObsidian: () => invoke<string>("open_memory_obsidian"),
  searchMemory: (query: string) => invoke<VaultSearchResult>("search_memory", { query }),
  memorySnapshot: () => invoke<MemorySnapshot>("get_memory_snapshot"),
  configureMemory: (settings: MemorySettings) => invoke<MemorySettings>("configure_memory", { settings }),
  createMemory: (input: Pick<MemoryRecord, "title" | "category" | "content">) => invoke<MemoryRecord>("create_memory", { input }),
  updateMemory: (input: Pick<MemoryRecord, "id" | "revision" | "title" | "category" | "content" | "pinned" | "status">) => invoke<MemoryRecord>("update_memory", { input }),
  retryMemoryJob: (id: string) => invoke<void>("retry_memory_job", { id }),
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
