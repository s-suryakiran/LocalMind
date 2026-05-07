export type AcceleratorType = "appleSilicon" | "nvidia" | "amd" | "intelArc" | "cpu";

export type Accelerator =
  | { type: "appleSilicon"; chip: string; unifiedMemoryGb: number }
  | { type: "nvidia"; name: string; vramGb: number; cudaVersion: string | null }
  | { type: "amd"; name: string; vramGb: number }
  | { type: "intelArc"; name: string }
  | { type: "cpu" };

export interface HardwareInfo {
  os: string;
  arch: string;
  cpuName: string;
  cpuCores: number;
  totalMemoryGb: number;
  accelerator: Accelerator;
  recommendedBackend: string;
  recommendedNGpuLayers: number;
}

export interface ModelFile {
  filename: string;
  sizeBytes: number;
  quantization: string;
  downloadUrl: string;
}

export interface ModelListing {
  id: string;
  name: string;
  author: string;
  downloads: number;
  likes: number;
  tags: string[];
  updated: string | null;
  description: string | null;
  files: ModelFile[];
}

export type ModelKind = "llm" | "vision" | "mmproj" | "embedding" | "whisper" | "sd";

export interface InstalledModel {
  id: string;
  filename: string;
  repo: string;
  sizeBytes: number;
  path: string;
  kind: ModelKind;
}

export type Role = "chat" | "embed" | "vision";

export interface SlotStatus {
  role: Role;
  running: boolean;
  port: number;
  modelId: string | null;
  mmprojId: string | null;
  pid: number | null;
}

export interface LlamaStatus {
  running: boolean;
  port: number;
  modelId: string | null;
  mmprojId: string | null;
  pid: number | null;
  embeddingRunning: boolean;
  embeddingPort: number;
  embeddingModelId: string | null;
  /** Canonical per-role view. Always exactly 3 entries: chat / embed / vision. */
  slots: SlotStatus[];
}

/** A single Synapse worker the host wants to pipeline-shard layers onto.
 *  Each worker has its own token (shown in the worker's Synapse UI) that
 *  the host's local proxy uses to authenticate. */
export interface SynapseWorker {
  endpoint: string;
  token: string;
  /** Phase 3 chunk G: relative compute weight (0–1). When any worker has a
   *  weight set, the host passes `--tensor-split` so layers split by ratio
   *  instead of evenly. Undefined keeps llama.cpp's default heuristic. */
  weight?: number;
  /** Phase 4 chunk N: SHA-256 hex fingerprint pinned for TLS verification.
   *  Captured from the worker's HMAC-verified beacon at pair time;
   *  undefined means TOFU (any cert accepted, token still gates). */
  certFingerprint?: string;
}

export interface LlamaSettings {
  modelId: string;
  contextSize?: number;
  nGpuLayers?: number;
  threads?: number;
  port?: number;
  mmprojId?: string;
  flashAttn?: boolean;
  /** Synapse workers to pipeline-shard layers across.
   *  Phase 3: each entry needs a token; the local proxy uses it for the
   *  handshake with the worker's auth proxy before any rpc bytes flow. */
  synapseWorkers?: SynapseWorker[];
  /** Phase 3 chunk G: host's relative weight in the layer split (0–1).
   *  Combined with each worker's `weight` to build `--tensor-split`. */
  hostWeight?: number;
}

export interface SynapseWorkerStatus {
  running: boolean;
  port: number;
  pid: number | null;
}

export interface SynapsePeer {
  id: string;
  hostname: string;
  address: string;
  port: number;
  /** `address:port` — paste-ready for the workers list. */
  endpoint: string;
  /** True iff the host has the worker's token AND the beacon's HMAC verifies.
   *  Hosts who haven't paired yet always see false; pair via the dialog and
   *  the next set_known_synapse_tokens call flips this on the cached entry. */
  verified: boolean;
  /** Phase 4 chunk J: hardware advertised in the beacon. Pre-Phase-4
   *  workers leave these undefined; treat that as "unknown, fall back to
   *  conservative defaults". */
  vramGb?: number;
  acceleratorKind?: "apple" | "nvidia" | "amd" | "intel-arc" | "cpu";
  acceleratorName?: string;
  /** Phase 4 chunk N: SHA-256 hex fingerprint of the worker's TLS cert.
   *  Rides inside the HMAC-signed beacon body. UI captures it on pair. */
  certFingerprint?: string;
}

/** synapse:metrics event payload — emitted by the chat llama-server's stderr
 *  parser whenever an `eval time` line lands. tokPerSec is the most recent
 *  generation throughput, host-side aggregate (all workers + host combined). */
export interface SynapseMetric {
  kind: "host-tok-s";
  tokPerSec: number;
  ts: number;
}

/** synapse:rtt event payload — emitted by each host_proxy's pinger every 5s. */
export interface SynapseRtt {
  endpoint: string;
  rttMs: number;
  ok: boolean;
  ts: number;
}

/** synapse:cluster-layout event payload — emitted once per model load,
 *  containing one entry per device that received layer tensors. Built
 *  from `load_tensors: <DEVICE> model buffer size = X MiB` lines. */
export interface SynapseClusterLayout {
  devices: { device: string; mb: number }[];
  ts: number;
}

export interface ModelDownloadProgress {
  id: string;
  downloaded: number;
  total: number;
  percent: number;
  stage: string;
}

export interface BinaryProgress {
  stage: string;
  downloaded: number;
  total: number;
  message: string;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  createdAt: number;
  pending?: boolean;
  images?: string[]; // base64 data URLs
  sources?: RetrievedChunk[];
}

export interface Conversation {
  id: string;
  title: string;
  messages: ChatMessage[];
  modelId: string | null;
  createdAt: number;
  updatedAt: number;
  ragDocIds: string[]; // documents enabled as context in this conversation
}

export interface RagDocument {
  id: string;
  name: string;
  sourcePath: string | null;
  createdAt: number;
  chunkCount: number;
  bytes: number;
}

export interface RagChunk {
  id: string;
  docId: string;
  docName: string;
  content: string;
  ordinal: number;
}

export interface RetrievedChunk {
  chunk: RagChunk;
  score: number;
}

export interface SdRequest {
  modelId: string;
  prompt: string;
  negativePrompt?: string;
  width?: number;
  height?: number;
  steps?: number;
  cfgScale?: number;
  seed?: number;
  sampler?: string;
}

export interface SdImage {
  id: string;
  path: string;
  prompt: string;
  modelId: string;
  width: number;
  height: number;
  seed: number;
  createdAt: number;
}

export interface SdProgress {
  id: string;
  stage: string;
  step: number;
  total: number;
  message: string;
}
