import { useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowUp, Square, Loader2, Boxes, ImagePlus, Mic, MicOff, Volume2, BookOpen, X, Menu,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark } from "react-syntax-highlighter/dist/esm/styles/prism";
import { useApp, registerChatRequest, unregisterChatRequest } from "../lib/store";
import { api, streamChat, type ChatTurn, type ChatContentPart } from "../lib/api";
import type { ChatMessage, RetrievedChunk } from "../lib/types";
import { cn, isTauri, uid } from "../lib/util";
import { createRecognition, speak, stopSpeaking, ttsSupported } from "../lib/voice";

export function Chat({ onOpenMenu }: { onOpenMenu?: () => void } = {}) {
  const {
    conversations, activeConvId, createConversation, updateConversation, renameConversation,
    activeModelId, setActiveModelId, installed, llama, setLlama, setView,
    ragDocs, setRagDocs, toggleRagDoc, synapse, online,
  } = useApp();
  const remote = !isTauri();
  // On the phone we don't pick a model — we use whatever the host has loaded.
  const effectiveModelId = remote ? (llama.modelId ?? null) : activeModelId;
  const canCompose = !!effectiveModelId && (!remote || online);

  const [input, setInput] = useState("");
  const [pendingImages, setPendingImages] = useState<string[]>([]);
  const [sending, setSending] = useState(false);
  const [recording, setRecording] = useState(false);
  const [showSources, setShowSources] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const abortRef = useRef<AbortController | null>(null);
  const recognitionRef = useRef<ReturnType<typeof createRecognition>>(null);

  const conv = useMemo(
    () => conversations.find((c) => c.id === activeConvId) ?? null,
    [conversations, activeConvId],
  );

  const activeModel = useMemo(
    () => installed.find((m) => m.id === effectiveModelId) ?? null,
    [installed, effectiveModelId],
  );
  // Remote/PWA = chat-only for now; image attach is desktop-side.
  const isVision = !remote && activeModel?.kind === "vision";

  useEffect(() => {
    if (!activeConvId && conversations.length === 0) {
      createConversation(effectiveModelId);
    } else if (!activeConvId && conversations[0]) {
      useApp.getState().setActiveConv(conversations[0].id);
    }
  }, [activeConvId, conversations, effectiveModelId, createConversation]);

  useEffect(() => {
    if (remote) return;
    api.ragList().then(setRagDocs).catch(() => {});
  }, [remote, setRagDocs]);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
  }, [conv?.messages.length, conv?.messages[conv.messages.length - 1]?.content]);

  async function ensureRunning(modelId: string) {
    // On the phone we can't spawn llama-server — surface a clear error if the
    // host hasn't loaded a model yet. Status is polled on a 5s tick.
    if (remote) {
      if (!llama.running) {
        throw new Error("No model is loaded on the host. Open LocalMind on your computer and start a model first.");
      }
      return;
    }
    const model = installed.find((m) => m.id === modelId);
    const wantMmproj = model?.kind === "vision" ? guessMmprojId(model, installed) : undefined;
    if (llama.running && llama.modelId === modelId && (llama.mmprojId ?? undefined) === wantMmproj) {
      return;
    }
    const synapseWorkers = synapse.workers.length > 0 ? synapse.workers : undefined;
    const status = await api.startLlama({
      modelId,
      mmprojId: wantMmproj,
      synapseWorkers,
      hostWeight: synapse.hostWeight,
    });
    setLlama(status);
  }

  function handleFiles(files: FileList | null) {
    if (!files) return;
    for (const f of Array.from(files)) {
      if (!f.type.startsWith("image/")) continue;
      downscaleImage(f, 1024, 0.85)
        .then((dataUrl) => setPendingImages((prev) => [...prev, dataUrl]))
        .catch((err) => console.error("image processing failed:", err));
    }
  }

  function startRecording() {
    if (recording) {
      recognitionRef.current?.stop();
      return;
    }
    const rec = createRecognition();
    if (!rec) {
      alert("Speech recognition isn't available in this browser. Try Chrome or Safari.");
      return;
    }
    recognitionRef.current = rec;
    setRecording(true);
    const start = input;
    rec.onResult((transcript, isFinal) => {
      setInput((start ? start + " " : "") + transcript);
      if (isFinal) {
        rec.stop();
      }
    });
    rec.onEnd(() => {
      setRecording(false);
      recognitionRef.current = null;
    });
    rec.onError(() => {
      setRecording(false);
      recognitionRef.current = null;
    });
    rec.start();
  }

  async function send() {
    if ((!input.trim() && pendingImages.length === 0) || sending) return;
    if (!effectiveModelId || !conv) return;

    if (!remote && pendingImages.length > 0 && isVision) {
      const hasMmproj = installed.some((m) => m.kind === "mmproj" || /mmproj/i.test(m.id));
      if (!hasMmproj) {
        alert(
          "This vision model needs a separate projector file (mmproj). Open the Marketplace, search for the same repo as your model, and download the file starting with \"mmproj-\".",
        );
        return;
      }
    }

    const userMsg: ChatMessage = {
      id: uid(),
      role: "user",
      content: input.trim(),
      createdAt: Date.now(),
      images: pendingImages.length ? [...pendingImages] : undefined,
    };
    const asstMsg: ChatMessage = {
      id: uid(),
      role: "assistant",
      content: "",
      createdAt: Date.now(),
      pending: true,
    };

    let messagesInProgress = [...conv.messages, userMsg, asstMsg];
    updateConversation(conv.id, { messages: messagesInProgress });

    if (conv.title === "New chat" && userMsg.content) {
      renameConversation(conv.id, userMsg.content.slice(0, 40));
    }

    const queryText = input.trim();
    setInput("");
    setPendingImages([]);
    setSending(true);

    try {
      let retrieved: RetrievedChunk[] = [];
      if (!remote && conv.ragDocIds.length > 0 && llama.embeddingRunning && queryText) {
        try {
          retrieved = await api.ragSearch(queryText, 5, conv.ragDocIds);
        } catch (e) {
          console.warn("RAG search failed:", e);
        }
      }

      await ensureRunning(effectiveModelId);
      const port = useApp.getState().llama.port;

      const systemPrompt = buildSystemPrompt(retrieved);
      const turns: ChatTurn[] = [];
      if (systemPrompt) {
        turns.push({ role: "system", content: systemPrompt });
      }
      for (const m of conv.messages) {
        turns.push({ role: m.role, content: m.content });
      }
      turns.push(formatUserTurn(userMsg, isVision));

      const controller = new AbortController();
      abortRef.current = controller;
      registerChatRequest(conv.id, controller);

      try {
        let acc = "";
        for await (const delta of streamChat(port, effectiveModelId, turns, { signal: controller.signal })) {
          if (controller.signal.aborted) break;
          acc += delta;
          messagesInProgress = messagesInProgress.map((m) =>
            m.id === asstMsg.id ? { ...m, content: acc } : m,
          );
          updateConversation(conv.id, { messages: messagesInProgress });
        }
        if (!controller.signal.aborted) {
          messagesInProgress = messagesInProgress.map((m) =>
            m.id === asstMsg.id
              ? { ...m, content: acc, pending: false, sources: retrieved.length ? retrieved : undefined }
              : m,
          );
          updateConversation(conv.id, { messages: messagesInProgress });
        }
      } finally {
        unregisterChatRequest(conv.id, controller);
      }
    } catch (e: any) {
      if (e?.name === "AbortError") return;
      const errText = e?.message ?? String(e);
      updateConversation(conv.id, {
        messages: messagesInProgress.map((m) =>
          m.id === asstMsg.id ? { ...m, content: `⚠️ ${errText}`, pending: false } : m,
        ),
      });
    } finally {
      setSending(false);
      abortRef.current = null;
    }
  }

  function stop() {
    abortRef.current?.abort();
  }

  const selectedDocCount = conv?.ragDocIds.length ?? 0;

  return (
    <div className="flex flex-col h-full">
      <header className="safe-top flex items-center justify-between px-5 py-3 border-b border-[var(--color-border-soft)] relative">
        <div className="flex items-center gap-2 min-w-0 flex-1">
          {onOpenMenu && (
            <button
              onClick={onOpenMenu}
              className="md:hidden text-[var(--color-text-muted)] hover:text-[var(--color-text)] -ml-1 p-1"
              aria-label="Open menu"
            >
              <Menu size={18} />
            </button>
          )}
          <h1 className="font-semibold text-[15px] truncate">{conv?.title ?? "Chat"}</h1>
        </div>
        <div className="flex items-center gap-2">
          {!remote && (
            <button
              onClick={() => setShowSources((v) => !v)}
              className={cn(
                "flex items-center gap-1.5 text-sm px-2.5 py-1.5 rounded-md border transition-colors",
                selectedDocCount > 0
                  ? "border-[var(--color-accent)]/60 bg-[var(--color-accent)]/10 text-[var(--color-text)]"
                  : "border-[var(--color-border)] bg-[var(--color-panel-2)] text-[var(--color-text-muted)] hover:text-[var(--color-text)]",
              )}
              title="Use documents as context (RAG)"
            >
              <BookOpen size={14} />
              {selectedDocCount > 0 ? `Sources · ${selectedDocCount}` : "Sources"}
            </button>
          )}
          {remote ? (
            <span className="text-xs text-[var(--color-text-muted)] bg-[var(--color-panel-2)] rounded-md px-2.5 py-1 truncate max-w-[200px]" title={llama.modelId ?? undefined}>
              {llama.modelId ? `via ${llama.modelId}` : "no model loaded on host"}
            </span>
          ) : (
            <ModelPicker
              installed={installed}
              activeModelId={activeModelId}
              onPick={setActiveModelId}
              onGoMarketplace={() => setView("marketplace")}
            />
          )}
        </div>
        {!remote && showSources && conv && (
          <SourcesPanel
            docs={ragDocs}
            selected={conv.ragDocIds}
            onToggle={(id) => toggleRagDoc(conv.id, id)}
            embeddingRunning={llama.embeddingRunning}
            onGoKnowledge={() => { setView("knowledge"); setShowSources(false); }}
            onClose={() => setShowSources(false)}
          />
        )}
      </header>

      <div ref={scrollRef} className="flex-1 overflow-y-auto">
        {(!conv || conv.messages.length === 0) ? (
          <EmptyState hasModel={!!effectiveModelId} onGoMarketplace={() => setView("marketplace")} remote={remote} />
        ) : (
          <div className="max-w-3xl mx-auto px-5 py-8 flex flex-col gap-6">
            {conv.messages.map((m) => (
              <Message key={m.id} msg={m} />
            ))}
          </div>
        )}
      </div>

      <footer className="safe-bottom px-5 py-4 border-t border-[var(--color-border-soft)]">
        <div className="max-w-3xl mx-auto">
          {pendingImages.length > 0 && (
            <div className="flex flex-wrap gap-2 mb-2">
              {pendingImages.map((src, i) => (
                <div key={i} className="relative w-16 h-16 rounded-md overflow-hidden border border-[var(--color-border)]">
                  <img src={src} alt="" className="w-full h-full object-cover" />
                  <button
                    onClick={() => setPendingImages((prev) => prev.filter((_, idx) => idx !== i))}
                    className="absolute top-0.5 right-0.5 w-4 h-4 rounded-full bg-black/60 text-white grid place-items-center"
                  >
                    <X size={10} />
                  </button>
                </div>
              ))}
            </div>
          )}
          <div className="relative flex items-end gap-1 rounded-2xl border border-[var(--color-border)] bg-[var(--color-panel)] focus-within:border-[var(--color-accent)]/60 transition-colors">
            <input
              type="file"
              accept="image/*"
              multiple
              hidden
              ref={fileInputRef}
              onChange={(e) => { handleFiles(e.target.files); e.target.value = ""; }}
            />
            {isVision && (
              <button
                onClick={() => fileInputRef.current?.click()}
                className="ml-1 mb-1.5 w-9 h-9 rounded-xl grid place-items-center text-[var(--color-text-muted)] hover:bg-[var(--color-panel-2)]"
                title="Attach image"
              >
                <ImagePlus size={16} />
              </button>
            )}
            <textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  send();
                }
              }}
              rows={1}
              placeholder={
                !effectiveModelId
                  ? remote ? "Load a model on the desktop first" : "Pick a model to start"
                  : remote && !online
                    ? "Host offline — reconnect to send"
                    : "Message your model…"
              }
              className="flex-1 resize-none bg-transparent px-3 py-3 outline-none text-sm max-h-48"
              disabled={!canCompose}
            />
            <button
              onClick={startRecording}
              className={cn(
                "mb-1.5 w-9 h-9 rounded-xl grid place-items-center hover:bg-[var(--color-panel-2)]",
                recording ? "text-[var(--color-danger)]" : "text-[var(--color-text-muted)]",
              )}
              title={recording ? "Stop recording" : "Voice input"}
            >
              {recording ? <MicOff size={16} /> : <Mic size={16} />}
            </button>
            <button
              onClick={sending ? stop : send}
              disabled={!canCompose || (!sending && !input.trim() && pendingImages.length === 0)}
              className={cn(
                "m-1.5 w-9 h-9 rounded-xl grid place-items-center transition-colors",
                sending
                  ? "bg-[var(--color-panel-2)] text-[var(--color-text)] hover:bg-[var(--color-border)]"
                  : "gradient-accent text-white disabled:opacity-40 disabled:cursor-not-allowed",
              )}
            >
              {sending ? <Square size={15} /> : <ArrowUp size={16} />}
            </button>
          </div>
          <p className="text-[11px] text-[var(--color-text-subtle)] mt-2 text-center">
            All conversations happen on your device. Nothing is sent to the cloud.
          </p>
        </div>
      </footer>
    </div>
  );
}

function SourcesPanel({
  docs, selected, onToggle, embeddingRunning, onGoKnowledge, onClose,
}: {
  docs: { id: string; name: string; chunkCount: number }[];
  selected: string[];
  onToggle: (id: string) => void;
  embeddingRunning: boolean;
  onGoKnowledge: () => void;
  onClose: () => void;
}) {
  return (
    <>
      <div className="fixed inset-0 z-40" onClick={onClose} />
      <div className="absolute right-5 top-14 z-50 w-80 max-h-[60vh] overflow-y-auto rounded-xl border border-[var(--color-border)] bg-[var(--color-panel)] shadow-xl p-3">
        <div className="flex items-center justify-between mb-2">
          <span className="text-xs text-[var(--color-text-muted)] uppercase tracking-wide">Context sources</span>
          <button onClick={onGoKnowledge} className="text-[11px] text-[var(--color-accent)]">manage</button>
        </div>
        {!embeddingRunning && (
          <div className="text-xs text-[var(--color-text-muted)] bg-[var(--color-panel-2)] rounded p-2 mb-2">
            Start the embedding model in Knowledge to enable RAG.
          </div>
        )}
        {docs.length === 0 ? (
          <div className="text-sm text-[var(--color-text-subtle)] py-4 text-center">No documents uploaded yet.</div>
        ) : (
          docs.map((d) => {
            const on = selected.includes(d.id);
            return (
              <label key={d.id} className="flex items-start gap-2 py-1.5 cursor-pointer">
                <input
                  type="checkbox"
                  checked={on}
                  onChange={() => onToggle(d.id)}
                  className="mt-0.5 accent-[var(--color-accent)]"
                />
                <div className="min-w-0">
                  <div className="text-sm truncate">{d.name}</div>
                  <div className="text-[11px] text-[var(--color-text-subtle)]">{d.chunkCount} chunks</div>
                </div>
              </label>
            );
          })
        )}
      </div>
    </>
  );
}

function Message({ msg }: { msg: ChatMessage }) {
  const isUser = msg.role === "user";
  const [speaking, setSpeaking] = useState(false);

  function toggleSpeak() {
    if (speaking) {
      stopSpeaking();
      setSpeaking(false);
      return;
    }
    speak(msg.content);
    setSpeaking(true);
    const timer = setInterval(() => {
      if (!window.speechSynthesis.speaking) {
        setSpeaking(false);
        clearInterval(timer);
      }
    }, 300);
  }

  return (
    <div className={cn("flex gap-3 group", isUser && "flex-row-reverse")}>
      <div
        className={cn(
          "w-7 h-7 rounded-full shrink-0 grid place-items-center text-[11px] font-semibold",
          isUser ? "bg-[var(--color-panel-2)] text-[var(--color-text-muted)]" : "gradient-accent text-white",
        )}
      >
        {isUser ? "You" : "AI"}
      </div>
      <div className={cn("max-w-[85%] min-w-0", isUser && "text-right")}>
        {msg.images && msg.images.length > 0 && (
          <div className="flex flex-wrap gap-2 mb-2 justify-end">
            {msg.images.map((src, i) => (
              <img key={i} src={src} alt="" className="max-w-[220px] rounded-lg border border-[var(--color-border)]" />
            ))}
          </div>
        )}
        <div
          className={cn(
            "rounded-2xl px-4 py-3 text-[14.5px] leading-relaxed prose-msg inline-block text-left",
            isUser
              ? "bg-[var(--color-panel-2)] text-[var(--color-text)]"
              : "bg-transparent text-[var(--color-text)]",
          )}
        >
          {msg.pending && !msg.content && (
            <span className="inline-flex items-center gap-2 text-[var(--color-text-muted)]">
              <Loader2 size={14} className="animate-spin" /> thinking…
            </span>
          )}
          <ReactMarkdown
            remarkPlugins={[remarkGfm]}
            components={{
              code({ className, children, ...props }) {
                const match = /language-(\w+)/.exec(className || "");
                const content = String(children).replace(/\n$/, "");
                const inline = !match && !content.includes("\n");
                return !inline && match ? (
                  <SyntaxHighlighter style={oneDark as any} language={match[1]} PreTag="div" customStyle={{ borderRadius: 8, fontSize: 13 }}>
                    {content}
                  </SyntaxHighlighter>
                ) : (
                  <code className={className} {...props}>{children}</code>
                );
              },
            }}
          >
            {msg.content}
          </ReactMarkdown>
        </div>
        {!isUser && msg.content && !msg.pending && (
          <div className="flex items-center gap-2 mt-1.5 opacity-0 group-hover:opacity-100 transition-opacity">
            {ttsSupported() && (
              <button
                onClick={toggleSpeak}
                className={cn(
                  "flex items-center gap-1 text-[11px] text-[var(--color-text-subtle)] hover:text-[var(--color-text)]",
                  speaking && "text-[var(--color-accent)]",
                )}
              >
                <Volume2 size={12} /> {speaking ? "stop" : "read"}
              </button>
            )}
          </div>
        )}
        {!isUser && msg.sources && msg.sources.length > 0 && (
          <details className="mt-2 text-xs text-[var(--color-text-muted)]">
            <summary className="cursor-pointer hover:text-[var(--color-text)]">
              {msg.sources.length} source{msg.sources.length === 1 ? "" : "s"} used
            </summary>
            <div className="mt-2 flex flex-col gap-1.5">
              {msg.sources.map((s, i) => (
                <div key={i} className="rounded border border-[var(--color-border-soft)] bg-[var(--color-panel)] p-2">
                  <div className="text-[11px] text-[var(--color-text-subtle)] mb-1">
                    {s.chunk.docName} · #{s.chunk.ordinal} · score {s.score.toFixed(2)}
                  </div>
                  <div className="line-clamp-3 text-[var(--color-text-muted)]">{s.chunk.content}</div>
                </div>
              ))}
            </div>
          </details>
        )}
      </div>
    </div>
  );
}

function downscaleImage(file: File, maxEdge: number, quality: number): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const img = new Image();
      img.onload = () => {
        const scale = Math.min(1, maxEdge / Math.max(img.width, img.height));
        const w = Math.round(img.width * scale);
        const h = Math.round(img.height * scale);
        const canvas = document.createElement("canvas");
        canvas.width = w;
        canvas.height = h;
        const ctx = canvas.getContext("2d");
        if (!ctx) { reject(new Error("no 2d context")); return; }
        ctx.drawImage(img, 0, 0, w, h);
        resolve(canvas.toDataURL("image/jpeg", quality));
      };
      img.onerror = () => reject(new Error("image decode failed"));
      img.src = reader.result as string;
    };
    reader.onerror = () => reject(reader.error ?? new Error("file read failed"));
    reader.readAsDataURL(file);
  });
}

function formatUserTurn(msg: ChatMessage, supportsVision: boolean): ChatTurn {
  if (!supportsVision || !msg.images || msg.images.length === 0) {
    return { role: "user", content: msg.content };
  }
  const parts: ChatContentPart[] = msg.images.map((url) => ({ type: "image_url", image_url: { url } }));
  if (msg.content) parts.push({ type: "text", text: msg.content });
  return { role: "user", content: parts };
}

function buildSystemPrompt(retrieved: RetrievedChunk[]): string {
  if (retrieved.length === 0) return "";
  const ctx = retrieved
    .map((r, i) => `[${i + 1}] ${r.chunk.docName} (chunk #${r.chunk.ordinal})\n${r.chunk.content}`)
    .join("\n\n---\n\n");
  return `You are a helpful assistant. Use the following context to answer the user's question. If the context does not contain the answer, say so briefly and answer from general knowledge. Cite relevant sources as [1], [2] etc.

Context:
${ctx}`;
}

function guessMmprojId(
  model: { id: string; repo: string },
  all: { id: string; repo: string; kind: string }[],
): string | undefined {
  const mmprojs = all.filter(
    (m) => m.kind === "mmproj" || /mmproj/i.test(m.id),
  );
  const sameRepo = mmprojs.find((m) => m.repo === model.repo);
  if (sameRepo) return sameRepo.id;
  return mmprojs[0]?.id;
}

function ModelPicker({
  installed, activeModelId, onPick, onGoMarketplace,
}: {
  installed: { id: string; filename: string; kind: string }[];
  activeModelId: string | null;
  onPick: (id: string) => void;
  onGoMarketplace: () => void;
}) {
  const chatModels = installed.filter((m) => m.kind === "llm" || m.kind === "vision");
  if (chatModels.length === 0) {
    return (
      <button onClick={onGoMarketplace} className="flex items-center gap-2 text-sm px-3 py-1.5 rounded-md bg-[var(--color-panel-2)] border border-[var(--color-border)] hover:border-[var(--color-accent)]/60">
        <Boxes size={14} /> Get a model
      </button>
    );
  }
  return (
    <select
      value={activeModelId ?? ""}
      onChange={(e) => onPick(e.target.value)}
      className="text-sm px-3 py-1.5 rounded-md bg-[var(--color-panel-2)] border border-[var(--color-border)] hover:border-[var(--color-accent)]/60 outline-none max-w-[200px]"
    >
      <option value="" disabled>Select model…</option>
      {chatModels.map((m) => (
        <option key={m.id} value={m.id}>{m.filename}</option>
      ))}
    </select>
  );
}

function EmptyState({
  hasModel, onGoMarketplace, remote,
}: { hasModel: boolean; onGoMarketplace: () => void; remote: boolean }) {
  return (
    <div className="h-full flex flex-col items-center justify-center text-center px-6">
      <div className="w-14 h-14 rounded-2xl gradient-accent grid place-items-center mb-4">
        <span className="text-white font-bold text-xl">L</span>
      </div>
      <h2 className="text-xl font-semibold mb-1">Welcome to LocalMind</h2>
      <p className="text-[var(--color-text-muted)] max-w-md mb-5 text-sm">
        {remote
          ? "You're connected. Chat will use the model loaded on your computer."
          : "Run open-source language models entirely on your device. Private, offline, fast."}
      </p>
      {!hasModel && !remote && (
        <button
          onClick={onGoMarketplace}
          className="px-4 py-2 rounded-md gradient-accent text-white text-sm font-medium"
        >
          Browse models →
        </button>
      )}
      {!hasModel && remote && (
        <p className="text-sm text-[var(--color-text-muted)]">Open LocalMind on your computer and start a model.</p>
      )}
      {hasModel && <p className="text-sm text-[var(--color-text-muted)]">Ask anything to get started.</p>}
    </div>
  );
}
