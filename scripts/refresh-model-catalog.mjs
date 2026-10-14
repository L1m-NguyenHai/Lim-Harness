import { mkdir, writeFile, rename } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";

const output = fileURLToPath(new URL("../src/data/model-catalog.json", import.meta.url));
const MAX_MODELS = 80;
const MAX_CONFIG = 64 * 1024;
const warnings = [];
const families = ["qwen3", "gemma3", "qwen2.5", "qwen2.5-coder", "deepseek-r1", "llama3.2", "llama3.1", "mistral", "phi4", "phi3", "smollm2", "granite3.3", "llava", "minicpm-v", "nomic-embed-text", "mxbai-embed-large", "bge-m3", "qwen3-embedding", "embeddinggemma", "gpt-oss"];

async function request(url, limit, redirects = 0, configDigest = null) {
  const parsed = new URL(url);
  const hash = configDigest?.slice(7);
  const configCdn = hash && parsed.hostname === "dd20bb891979d25aebc8bec07b2b3bbc.r2.cloudflarestorage.com" && parsed.pathname === `/ollama/docker/registry/v2/blobs/sha256/${hash.slice(0, 2)}/${hash}/data` && limit <= MAX_CONFIG;
  if (parsed.protocol !== "https:" || !["ollama.com", "registry.ollama.ai"].includes(parsed.hostname) && !configCdn) throw new Error("Untrusted metadata URL");
  const response = await fetch(parsed, { redirect: "manual", signal: AbortSignal.timeout(15000), headers: { "User-Agent": "Lim-Harness-Catalog/1.0" } });
  if (response.status >= 300 && response.status < 400) {
    if (redirects >= 3 || !response.headers.get("location")) throw new Error("Metadata redirect limit");
    return request(new URL(response.headers.get("location"), parsed), limit, redirects + 1, configDigest);
  }
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  const length = Number(response.headers.get("content-length"));
  if (length > limit) { await response.body?.cancel(); throw new Error("Metadata exceeds size limit"); }
  const chunks = [];
  let size = 0;
  const reader = response.body.getReader();
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > limit) throw new Error("Metadata exceeds size limit");
      chunks.push(Buffer.from(value));
    }
  } finally { await reader.cancel(); }
  return Buffer.concat(chunks);
}

const htmlText = (html) => html.replace(/<[^>]*>/g, " ").replace(/&amp;/g, "&").replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/\s+/g, " ").trim();
function links(html) {
  return [...html.matchAll(/href=["']([^"']+)["']/g)].map((match) => match[1].replace(/&amp;/g, "&"));
}
async function concurrent(items, callback) {
  let index = 0;
  await Promise.all(Array.from({ length: Math.min(4, items.length) }, async () => {
    while (index < items.length) await callback(items[index++]);
  }));
}

async function collectTags(family) {
  const html = (await request(`https://ollama.com/library/${family}/tags`, 2 * 1024 * 1024)).toString("utf8");
  const tags = [...new Set(links(html).filter((link) => link.startsWith(`/library/${family}:`)).map((link) => decodeURIComponent(link.slice(`/library/${family}:`.length).split(/[?#]/)[0])))];
  // Prefer real, plainly named size tags. Never synthesize tags from size chips.
  const simple = tags.filter((tag) => /^\d+(?:\.\d+)?[bm]$/.test(tag));
  const q4 = tags.filter((tag) => !/cloud/i.test(tag) && /q4_k_m/i.test(tag));
  const others = tags.filter((tag) => !/cloud/i.test(tag) && ["latest", "mini", "small", "medium"].includes(tag));
  const rowInfo = new Map();
  for (const match of html.matchAll(/<a[^>]*href="([^"?#]+)"[^>]*>([\s\S]*?)<\/a>/g)) {
    const tag = match[1].startsWith(`/library/${family}:`) ? decodeURIComponent(match[1].slice(`/library/${family}:`.length)) : "";
    if (!tag || rowInfo.has(tag)) continue;
    const text = htmlText(match[2]);
    const context = text.match(/(\d+(?:\.\d+)?)\s*([km])?\s+context\s+window/i);
    if (context || /(?:Text|Image) input/i.test(text)) rowInfo.set(tag, {
      maxContext: context ? Math.round(Number(context[1]) * (context[2]?.toLowerCase() === "k" ? 1024 : context[2]?.toLowerCase() === "m" ? 1024 * 1024 : 1)) : undefined,
      imageInput: /(?:Text,?\s*)?Image input/i.test(text),
    });
  }
  return [...simple, ...q4, ...others].slice(0, 6).map((tag, slot) => ({ family, tag, slot, ...rowInfo.get(tag) }));
}

async function modelMetadata({ family, tag, maxContext, imageInput }) {
  const registryRoot = `https://registry.ollama.ai/v2/library/${family}`;
  const manifestBytes = await request(`${registryRoot}/manifests/${encodeURIComponent(tag)}`, MAX_CONFIG);
  const manifest = JSON.parse(manifestBytes.toString("utf8"));
  if (!manifest.config || manifest.config.size > MAX_CONFIG || !/^sha256:[a-f0-9]{64}$/.test(manifest.config.digest)) throw new Error("Invalid small config descriptor");
  const configBytes = await request(`${registryRoot}/blobs/${manifest.config.digest}`, MAX_CONFIG, 0, manifest.config.digest);
  if (`sha256:${createHash("sha256").update(configBytes).digest("hex")}` !== manifest.config.digest) throw new Error("Config digest mismatch");
  const config = JSON.parse(configBytes.toString("utf8"));
  const layers = Array.isArray(manifest.layers) ? manifest.layers : [];
  if (!layers.every((layer) => Number.isSafeInteger(layer.size) && layer.size >= 0)) throw new Error("Invalid layer sizes");
  const weights = layers.filter((layer) => ["application/vnd.ollama.image.model", "application/vnd.ollama.image.projector"].includes(layer.mediaType));
  if (!weights.length) throw new Error("No weights descriptor; cloud or unsupported model");
  const source = `https://ollama.com/library/${family}:${tag}`;
  let capabilities = [], description = "";
  try {
    const html = (await request(source, 2 * 1024 * 1024)).toString("utf8");
    capabilities = ["tools", "thinking", "vision", "embedding"].filter((capability) => new RegExp(`>\\s*${capability}\\s*<`, "i").test(html));
    if (imageInput === false) capabilities = capabilities.filter((capability) => capability !== "vision");
    const meta = html.match(/<meta\s+name="description"\s+content="([^"]*)"/i);
    description = meta ? htmlText(meta[1]).slice(0, 240) : "";
  } catch (error) { warnings.push(`${family}:${tag}: public page unavailable (${error.message})`); }
  const parameterSize = typeof config.model_type === "string" ? config.model_type : undefined;
  const parameterMatch = parameterSize?.match(/^(\d+(?:\.\d+)?)\s*([BM])$/i);
  const parameterCount = parameterMatch ? Number(parameterMatch[1]) * (parameterMatch[2].toUpperCase() === "B" ? 1e9 : 1e6) : undefined;
  return {
    id: `${family}:${tag}`, name: `${family}:${tag}`,
    downloadBytes: manifest.config.size + layers.reduce((total, layer) => total + layer.size, 0),
    weightBytes: weights.reduce((total, layer) => total + layer.size, 0),
    quantization: typeof config.file_type === "string" ? config.file_type : "—",
    parameterSize, parameterCount, maxContext, capabilities,
    description, source, verifiedAt: new Date().toISOString().slice(0, 10),
    digest: `sha256:${createHash("sha256").update(manifestBytes).digest("hex")}`,
    metadataStatus: config.file_type && parameterSize ? "verified" : "partial",
    _weightsKey: weights.map((layer) => layer.digest).sort().join(","),
  };
}

async function main() {
  const library = (await request("https://ollama.com/library", 4 * 1024 * 1024)).toString("utf8");
  const listed = new Set(links(library).filter((link) => /^\/library\/[a-zA-Z0-9._-]+$/.test(link)).map((link) => link.split("/").pop()));
  // A family not on the listing is verified via its public family page, not assumed.
  const candidates = [];
  await concurrent(families, async (family) => {
    try {
      if (!listed.has(family)) await request(`https://ollama.com/library/${family}`, 2 * 1024 * 1024);
      candidates.push(...await collectTags(family));
    } catch (error) { warnings.push(`${family}: ${error.message}`); }
  });
  // Interleave families so the cap never silently removes embedding/vision families.
  candidates.sort((a, b) => a.slot - b.slot || families.indexOf(a.family) - families.indexOf(b.family));
  const found = [];
  await concurrent(candidates.slice(0, MAX_MODELS), async (candidate) => {
    try { found.push(await modelMetadata(candidate)); }
    catch (error) { warnings.push(`${candidate.family}:${candidate.tag}: ${error.message}`); }
  });
  found.sort((a, b) => families.indexOf(a.id.split(":")[0]) - families.indexOf(b.id.split(":")[0]) || (a.parameterCount ?? 0) - (b.parameterCount ?? 0) || a.id.localeCompare(b.id));
  const seen = new Set();
  const models = found.filter((model) => {
    if (seen.has(model._weightsKey)) return false;
    seen.add(model._weightsKey); return true;
  }).map(({ _weightsKey, ...model }) => model);
  if (models.length < 10) throw new Error(`Only ${models.length} models verified; preserving existing catalog. ${warnings.join("; ")}`);
  const snapshot = { models, updatedAt: new Date().toISOString(), source: "bundled", warnings };
  await mkdir(fileURLToPath(new URL("../src/data/", import.meta.url)), { recursive: true });
  await writeFile(`${output}.tmp`, `${JSON.stringify(snapshot, null, 2)}\n`, "utf8");
  await rename(`${output}.tmp`, output);
  console.log(JSON.stringify({ count: models.length, families: new Set(models.map((model) => model.id.split(":")[0])).size, warnings, output }, null, 2));
}

main().catch((error) => { console.error(error.message); process.exitCode = 1; });
