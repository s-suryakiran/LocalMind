import { isTauri } from "./util";
import { useApp } from "./store";
import type {
  HardwareInfo,
  InstalledModel,
  LlamaSettings,
  LlamaStatus,
  ModelListing,
  RagDocument,
  RetrievedChunk,
  SdImage,
  SdRequest,
  SynapsePeer,
  SynapseWorkerStatus,
} from "./types";

function connection() {
  return useApp.getState().connection;
}

function authHeaders(): Record<string, string> {
  const c = connection();
  return c ? { Authorization: `Bearer ${c.token}` } : {};
}

export async function pairWithServer(url: string, pin: string): Promise<{ token: string }> {
  const base = url.replace(/\/+$/, "");
  const res = await fetch(`${base}/api/pair`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ pin }),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(text || `pair failed: ${res.status}`);
  }
  return res.json();
}

export async function probeServer(url: string): Promise<boolean> {
  try {
    const base = url.replace(/\/+$/, "");
    const res = await fetch(`${base}/health`, { method: "GET" });
    return res.ok;
  } catch {
    return false;
  }
}

/** Sentinel error class so callers can branch on "stale token, re-pair" vs. transient network failure. */
export class UnauthorizedError extends Error {
  constructor() {
    super("unauthorized");
    this.name = "UnauthorizedError";
  }
}

export async function remoteStatus(): Promise<LlamaStatus> {
  const c = connection();
  if (!c) throw new Error("not connected");
  const res = await fetch(`${c.url.replace(/\/+$/, "")}/api/status`, {
    headers: authHeaders(),
  });
  if (res.status === 401) throw new UnauthorizedError();
  if (!res.ok) throw new Error(`status failed: ${res.status}`);
  return res.json();
}

/// Phase 4 chunk P: read-only Synapse fetchers for the phone PWA. Always
/// hit the paired desktop's LAN API — there's no desktop equivalent
/// because desktop already has Tauri-IPC versions on `api`. Returning
/// nullable so a transient LAN hiccup is visible (state goes empty)
/// rather than throwing into an unhandled-promise hole.
async function lanGet<T>(path: string): Promise<T> {
  const c = connection();
  if (!c) throw new Error("not connected");
  const res = await fetch(`${c.url.replace(/\/+$/, "")}${path}`, {
    headers: authHeaders(),
  });
  if (!res.ok) throw new Error(`${path}: ${res.status}`);
  return res.json();
}

export const remoteSynapse = {
  status: () => lanGet<{ running: boolean; port: number; pid: number | null }>("/api/synapse/status"),
  peers: () => lanGet<import("./types").SynapsePeer[]>("/api/synapse/peers"),
  sessions: () => lanGet<{ count: number }>("/api/synapse/sessions"),
};

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri()) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<T>(cmd, args);
  }
  const res = await fetch(`/api/cmd/${cmd}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(args ?? {}),
  });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

export async function listen<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
  if (isTauri()) {
    const { listen } = await import("@tauri-apps/api/event");
    const unlisten = await listen<T>(event, (e) => handler(e.payload));
    return unlisten;
  }
  const es = new EventSource(`/api/events?topic=${encodeURIComponent(event)}`);
  es.onmessage = (e) => {
    try { handler(JSON.parse(e.data) as T); } catch { /* ignore */ }
  };
  return () => es.close();
}

export const api = {
  detectHardware: () => invoke<HardwareInfo>("detect_hardware"),
  searchModels: (query: string, limit = 20) => invoke<ModelListing[]>("search_models", { query, limit }),
  downloadModel: (repo: string, filename: string, kind = "llm") =>
    invoke<InstalledModel>("download_model", { repo, filename, kind }),
  listInstalledModels: () => invoke<InstalledModel[]>("list_installed_models"),
  deleteModel: (id: string) => invoke<void>("delete_model", { id }),
  startLlama: (settings: LlamaSettings) => invoke<LlamaStatus>("start_llama", { settings }),
  stopLlama: () => invoke<void>("stop_llama"),
  llamaStatus: () => invoke<LlamaStatus>("llama_status"),
  startEmbeddingServer: (modelId: string) => invoke<LlamaStatus>("start_embedding_server", { modelId }),
  stopEmbeddingServer: () => invoke<void>("stop_embedding_server"),
  getLanUrl: () => invoke<string | null>("get_lan_url"),
  ensureEngine: () => invoke<string>("ensure_engine"),
  ragList: () => invoke<RagDocument[]>("rag_list"),
  ragDelete: (id: string) => invoke<void>("rag_delete", { id }),
  ragIngest: (path: string) => invoke<RagDocument>("rag_ingest", { path }),
  ragSearch: (query: string, topK = 5, docIds?: string[]) =>
    invoke<RetrievedChunk[]>("rag_search", { query, topK, docIds }),
  sdGenerate: (request: SdRequest) => invoke<SdImage>("sd_generate", { request }),
  sdBusy: () => invoke<boolean>("sd_busy"),
  ensureSd: () => invoke<string>("ensure_sd"),
  getLanPin: () => invoke<string>("get_lan_pin"),
  startSynapseWorker: (port?: number) =>
    invoke<SynapseWorkerStatus>("start_synapse_worker", { port }),
  stopSynapseWorker: () => invoke<void>("stop_synapse_worker"),
  synapseWorkerStatus: () => invoke<SynapseWorkerStatus>("synapse_worker_status"),
  restartSynapseWorker: (port?: number) =>
    invoke<SynapseWorkerStatus>("restart_synapse_worker", { port }),
  synapseListPeers: () => invoke<SynapsePeer[]>("synapse_list_peers"),
  getSynapseToken: () => invoke<string>("get_synapse_token"),
  rotateSynapseToken: () => invoke<string>("rotate_synapse_token"),
  setKnownSynapseTokens: (tokens: Record<string, string>) =>
    invoke<void>("set_known_synapse_tokens", { tokens }),
  synapseActiveSessions: () => invoke<number>("synapse_active_sessions"),
};

export type ChatContentPart =
  | { type: "text"; text: string }
  | { type: "image_url"; image_url: { url: string } };

export interface ChatTurn {
  role: "user" | "assistant" | "system";
  content: string | ChatContentPart[];
}

export async function* streamChat(
  port: number,
  modelId: string,
  messages: ChatTurn[],
  opts: { temperature?: number; maxTokens?: number; signal?: AbortSignal } = {},
): AsyncGenerator<string> {
  const conn = connection();
  const base = conn
    ? conn.url.replace(/\/+$/, "")
    : isTauri()
    ? `http://127.0.0.1:${port}`
    : "";
  const res = await fetch(`${base}/v1/chat/completions`, {
    method: "POST",
    headers: { "content-type": "application/json", ...authHeaders() },
    signal: opts.signal,
    body: JSON.stringify({
      model: modelId,
      messages,
      stream: true,
      temperature: opts.temperature ?? 0.7,
      top_p: 0.9,
      max_tokens: opts.maxTokens ?? 1024,
      repeat_penalty: 1.1,
      frequency_penalty: 0.3,
      presence_penalty: 0.0,
    }),
  });
  if (!res.ok || !res.body) {
    throw new Error(`chat request failed: ${res.status} ${await res.text().catch(() => "")}`);
  }

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    buf += decoder.decode(value, { stream: true });
    const lines = buf.split("\n");
    buf = lines.pop() ?? "";
    for (const raw of lines) {
      const line = raw.trim();
      if (!line || !line.startsWith("data:")) continue;
      const data = line.slice(5).trim();
      if (data === "[DONE]") return;
      try {
        const json = JSON.parse(data);
        const delta = json.choices?.[0]?.delta?.content ?? "";
        if (delta) yield delta;
      } catch { /* keep reading */ }
    }
  }
}
