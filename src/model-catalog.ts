import bundledCatalog from "./data/model-catalog.json";

export type Hardware = {
  cpu: string | null;
  ramTotalBytes: number | null;
  ramAvailableBytes: number | null;
  gpuName: string | null;
  gpuTotalBytes: number | null;
  gpuFreeBytes: number | null;
  diskAvailableBytes: number | null;
  modelDirectory: string | null;
  modelDirectorySource: "configured" | "default-estimate" | null;
  notes: string[];
};

export type CatalogModel = {
  id: string;
  name: string;
  downloadBytes: number;
  quantization: string;
  capabilities: string[];
  description: string;
  source: string;
  verifiedAt: string;
  weightBytes?: number;
  parameterSize?: string;
  parameterCount?: number;
  maxContext?: number;
  digest?: string;
  metadataStatus?: "verified" | "partial";
  provider?: "ollama" | "huggingface";
  downloads?: number;
};

export type CatalogSnapshot = {
  models: CatalogModel[];
  updatedAt: string;
  source: "online" | "cache" | "bundled" | "search";
  warnings: string[];
};

export const BUNDLED_CATALOG = bundledCatalog as CatalogSnapshot;


export type ModelFit = {
  status: "recommended" | "caution" | "limited" | "unknown";
  reason: string;
  contextTokens: 4096 | 8192;
  estimatedMemoryGiB: number;
  mode: "gpu" | "ram" | "unknown";
  score: number | null;
  assumptions: string;
};
export type AssessmentMode = "auto" | "gpu" | "ram";

const GIB = 1024 ** 3;
function knownBytes(value: number | null): number | null {
  return value !== null && Number.isFinite(value) && value >= 0
    ? value
    : null;
}

export function estimateMemoryBytes(model: CatalogModel, context: number): number {
  // ponytail: KV estimate from total parameters, not architecture-derived or measured.
  // MoE and hybrid architectures need real layer/head/cache metadata for tighter estimates.
  const parametersB = model.parameterCount ? model.parameterCount / 1e9 : null;
  const embedding = model.capabilities.some((capability) => capability.toLowerCase() === "embedding");
  const cacheGiBPer4k = parametersB == null ? 2 : Math.max(0.25, Math.min(8, parametersB * 0.15));
  const cache = embedding ? 0.25 * Math.max(1, context / 4096) : cacheGiBPer4k * context / 4096;
  return (model.weightBytes ?? model.downloadBytes) * 1.1 + GIB * (0.75 + cache);
}

export function getModelFit(
  model: CatalogModel,
  hardware: Hardware,
  contextSize = 4096,
  assessment: AssessmentMode = "auto",
): ModelFit {
  const context = Number.isFinite(contextSize) && contextSize > 0
    ? Math.ceil(contextSize)
    : 4096;
  const estimated = estimateMemoryBytes(model, context);
  const gpuFree = knownBytes(hardware.gpuFreeBytes);
  const ramFree = knownBytes(hardware.ramAvailableBytes);
  // Keep GPU and host RAM separate; never add the pools to claim a fit.
  const gpuBudget = gpuFree === null ? null : Math.max(0, gpuFree - Math.max(GIB, gpuFree * 0.2));
  const ramBudget = ramFree === null ? null : Math.max(0, ramFree - Math.max(2 * GIB, ramFree * 0.2));
  const gpuCanFit = gpuBudget !== null && estimated <= gpuBudget;
  const ramCanFit = ramBudget !== null && estimated <= ramBudget;
  const gpuFits = assessment !== "ram" && gpuCanFit;
  const ramFits = assessment !== "gpu" && ramCanFit;
  const suggestedBudget = gpuFits ? gpuBudget : ramFits ? ramBudget : null;
  const contextTokens: 4096 | 8192 = suggestedBudget !== null && estimateMemoryBytes(model, 8192) <= suggestedBudget ? 8192 : 4096;
  const base = {
    contextTokens,
    estimatedMemoryGiB: Math.ceil(estimated / GIB * 10) / 10,
    assumptions: "Weights × 1,10 + 0,75 GiB runtime + KV ước lượng theo params/context; dự phòng ≥1 GiB/20% GPU hoặc ≥2 GiB/20% RAM. Không phải benchmark.",
  };
  const budget = assessment === "gpu" ? gpuBudget : assessment === "ram" ? ramBudget : gpuFits ? gpuBudget : ramFits ? ramBudget : Math.max(gpuBudget ?? 0, ramBudget ?? 0) || null;
  // SCORE is machine-fit only: 50 at the memory limit, 100 with >=2x headroom.
  const score = budget == null ? null : Math.round(Math.max(0, Math.min(100, 50 * budget / estimated)));
  const contextLabel = context.toLocaleString("vi-VN");
  if (model.maxContext && context > model.maxContext) {
    return { ...base, status: "limited", mode: "unknown", score: 0, reason: `Context ${contextLabel} vượt mức ${model.maxContext.toLocaleString("vi-VN")} được công bố cho tag này.` };
  }
  if (gpuFits && ramCanFit) {
    return {
      ...base, score, status: model.metadataStatus === "partial" ? "caution" : "recommended", mode: "gpu",
      reason: `Ước lượng vừa VRAM trống ở context ${contextLabel}, đã chừa bộ nhớ dự phòng; RAM trống cũng đủ theo ước lượng. Ollama vẫn cần xác nhận GPU khi nạp.`,
    };
  }
  if (gpuFits) {
    return {
      ...base, score, status: "caution", mode: "gpu",
      reason: `VRAM có vẻ đủ ở context ${contextLabel}, nhưng ${ramBudget === null ? "chưa đo được RAM trống" : "RAM trống thấp theo ước lượng"}. Chưa xác nhận nạp được model.`,
    };
  }
  if (ramFits) {
    return {
      ...base, score, status: assessment === "ram" && model.metadataStatus !== "partial" ? "recommended" : "caution", mode: "ram",
      reason: assessment === "ram" ? `Ước lượng vừa RAM trống ở context ${contextLabel}, đã chừa dự phòng. Chưa xác nhận tốc độ CPU/offload.` : `Có thể dùng RAM ở context ${contextLabel} theo ước lượng; ${gpuBudget === null ? "chưa biết VRAM trống" : "VRAM trống chưa đủ cho toàn bộ model"}. Chưa xác nhận CPU/offload, tốc độ hoặc hỗ trợ GPU.`,
    };
  }
  if (assessment === "gpu" ? gpuBudget === null : assessment === "ram" ? ramBudget === null : gpuBudget === null || ramBudget === null) {
    return {
      ...base, score: null, status: "unknown", mode: "unknown",
      reason: "Thiếu số đo RAM hoặc VRAM trống để kết luận. Không dùng bộ nhớ tổng thay cho bộ nhớ đang khả dụng.",
    };
  }
  return {
    ...base, score, status: "limited", mode: "unknown",
    reason: `${assessment === "gpu" ? "VRAM" : assessment === "ram" ? "RAM" : "RAM và VRAM"} trống thấp hơn ước lượng ở context ${contextLabel} sau khi chừa dự phòng. Thử context 4.096 hoặc model nhỏ hơn; đây chưa phải kết quả chạy thử.`,
  };
}

