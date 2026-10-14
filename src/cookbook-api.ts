import { Channel, invoke } from "@tauri-apps/api/core";
import type { Hardware, CatalogSnapshot } from "./model-catalog";

export type InstalledModel = { name: string; sizeBytes: number; parameterSize?: string; quantizationLevel?: string; modifiedAt?: string };
export type RunningModel = { name: string; sizeBytes: number; sizeVramBytes: number; contextLength?: number; expiresAt?: string };
export type ModelObservation = { model: string; contextSize: number; tokensPerSecond: number; outputTokens: number; measuredAt: string };
export type CookbookSnapshot = { hardware: Hardware; ollamaAvailable: boolean; error?: string; version?: string | null; models: InstalledModel[]; running: RunningModel[]; measurements?: ModelObservation[] };
export type SetupEvent = {
  type: "started" | "progress" | "loading" | "ready" | "downloaded" | "failed" | "cancelled";
  operationId: string;
  sequence: number;
  model: string;
  status?: string;
  digest?: string;
  completed?: number;
  total?: number;
  error?: string;
};
export const cookbook = {
  snapshot: () => invoke<CookbookSnapshot>("cookbook_snapshot"),
  catalog: (refresh = false) => invoke<CatalogSnapshot>("get_model_catalog", { refresh }),
  search: (query: string, provider: "all" | "ollama" | "huggingface") => invoke<CatalogSnapshot>("search_model_catalog", { query, provider }),
  setup: (model: string, contextSize: number, receive: (event: SetupEvent) => void) => {
    const onEvent = new Channel<SetupEvent>();
    onEvent.onmessage = receive;
    return invoke<string>("start_model_setup", { model, contextSize, onEvent });
  },
  download: (model: string, receive: (event: SetupEvent) => void) => {
    const onEvent = new Channel<SetupEvent>();
    onEvent.onmessage = receive;
    return invoke<string>("start_model_setup", { model, contextSize: 4096, downloadOnly: true, onEvent });
  },
  cancel: (operationId: string) => invoke<boolean>("cancel_model_setup", { operationId }),
  unload: (model: string) => invoke<void>("unload_model", { model }),
};
