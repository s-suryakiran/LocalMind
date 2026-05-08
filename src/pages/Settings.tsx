import { useEffect, useState } from "react";
import { Cpu, Wifi, Smartphone, Copy, Check, KeyRound, LogOut, Mic } from "lucide-react";
import QRCode from "qrcode";
import { useApp } from "../lib/store";
import { api } from "../lib/api";
import type { BinaryProgress, VoiceEngineKind } from "../lib/types";
import { listen } from "../lib/api";
import { isTauri } from "../lib/util";

export function Settings() {
  const { hardware, lanUrl, connection, setConnection } = useApp();
  const remote = !isTauri();
  const [engineStatus, setEngineStatus] = useState<string>("not checked");
  const [engineProgress, setEngineProgress] = useState<BinaryProgress | null>(null);
  const [copied, setCopied] = useState<"url" | "pin" | null>(null);
  const [pin, setPin] = useState<string | null>(null);
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);

  useEffect(() => {
    if (remote) return;
    listen<BinaryProgress>("binary:progress", (p) => setEngineProgress(p));
    api.getLanPin().then(setPin).catch(() => {});
  }, [remote]);

  // Render a QR encoding {url, pin} JSON so a future scan-to-pair flow can
  // populate both fields in one tap.
  useEffect(() => {
    if (!lanUrl || !pin) return;
    const payload = JSON.stringify({ url: lanUrl, pin });
    QRCode.toDataURL(payload, { width: 192, margin: 1, color: { dark: "#0a0a0b", light: "#ffffff" } })
      .then(setQrDataUrl)
      .catch(() => setQrDataUrl(null));
  }, [lanUrl, pin]);

  async function installEngine() {
    setEngineStatus("installing");
    try {
      await api.ensureEngine();
      setEngineStatus("ready");
    } catch (e: any) {
      setEngineStatus("error: " + (e?.message ?? (typeof e === "string" ? e : JSON.stringify(e))));
    }
  }

  function copy(text: string, key: "url" | "pin") {
    navigator.clipboard.writeText(text);
    setCopied(key);
    setTimeout(() => setCopied(null), 1500);
  }

  return (
    <div className="flex flex-col h-full">
      <header className="px-5 py-4 border-b border-[var(--color-border-soft)]">
        <h1 className="font-semibold text-[17px]">Settings</h1>
        <p className="text-[var(--color-text-muted)] text-sm">
          {remote ? "Connection details and pairing." : "Hardware, engine, and network access."}
        </p>
      </header>

      <div className="flex-1 overflow-y-auto px-5 py-5 max-w-3xl">
        {remote && connection && (
          <Section title="Connection" icon={<Wifi size={15} />}>
            <Row k="Server" v={connection.url} />
            <Row k="Token" v={`${connection.token.slice(0, 8)}…`} />
            <button
              onClick={() => setConnection(null)}
              className="mt-3 flex items-center gap-2 text-sm px-3 py-1.5 rounded-md bg-[var(--color-panel-2)] border border-[var(--color-border)] hover:border-[var(--color-danger)]/60 hover:text-[var(--color-danger)]"
            >
              <LogOut size={13} /> Disconnect
            </button>
          </Section>
        )}

        {!remote && (
          <Section title="Hardware" icon={<Cpu size={15} />}>
            {hardware ? (
              <div className="grid grid-cols-2 gap-y-2 text-sm">
                <Row k="Operating system" v={`${hardware.os} (${hardware.arch})`} />
                <Row k="CPU" v={`${hardware.cpuName.trim()} · ${hardware.cpuCores} cores`} />
                <Row k="Memory" v={`${hardware.totalMemoryGb.toFixed(1)} GB`} />
                <Row k="Accelerator" v={describeAcc(hardware.accelerator)} />
                <Row k="Recommended backend" v={hardware.recommendedBackend} />
                <Row k="GPU layers" v={hardware.recommendedNGpuLayers === -1 ? "all (offload)" : String(hardware.recommendedNGpuLayers)} />
              </div>
            ) : (
              <p className="text-[var(--color-text-muted)] text-sm">Detecting…</p>
            )}
          </Section>
        )}

        {!remote && (
          <Section title="Inference engine (llama.cpp)" icon={<Cpu size={15} />}>
            <p className="text-sm text-[var(--color-text-muted)] mb-3">
              LocalMind downloads a prebuilt llama.cpp for your hardware. This happens automatically the first time you load a model.
            </p>
            <button
              onClick={installEngine}
              className="text-sm px-3 py-1.5 rounded-md bg-[var(--color-panel-2)] border border-[var(--color-border)] hover:border-[var(--color-accent)]/60"
            >
              Reinstall engine
            </button>
            {engineProgress && (
              <div className="mt-3 text-xs text-[var(--color-text-muted)]">
                {engineProgress.message} {engineProgress.total > 0 && `· ${Math.floor((engineProgress.downloaded / engineProgress.total) * 100)}%`}
              </div>
            )}
            <div className="text-xs text-[var(--color-text-subtle)] mt-2">Status: {engineStatus}</div>
          </Section>
        )}

        {!remote && (
          <Section title="Pair a phone or tablet" icon={<Smartphone size={15} />}>
            <p className="text-sm text-[var(--color-text-muted)] mb-3">
              Open the URL below on your phone (same Wi-Fi), then enter the PIN to pair. The PIN is reset every time the desktop app starts.
            </p>
            <div className="flex flex-col gap-2 sm:flex-row sm:items-start">
              <div className="flex-1 flex flex-col gap-2">
                {lanUrl ? (
                  <div className="flex items-center gap-2 rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2">
                    <Wifi size={14} className="text-[var(--color-text-muted)]" />
                    <code className="flex-1 text-sm font-mono truncate">{lanUrl}</code>
                    <button
                      onClick={() => copy(lanUrl, "url")}
                      className="text-[var(--color-text-muted)] hover:text-[var(--color-text)]"
                    >
                      {copied === "url" ? <Check size={14} className="text-[var(--color-success)]" /> : <Copy size={14} />}
                    </button>
                  </div>
                ) : (
                  <p className="text-sm text-[var(--color-text-muted)]">Starting LAN server…</p>
                )}
                <div className="flex items-center gap-2 rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2">
                  <KeyRound size={14} className="text-[var(--color-text-muted)]" />
                  <code className="flex-1 text-base font-mono tracking-[0.4em]">{pin ?? "------"}</code>
                  {pin && (
                    <button
                      onClick={() => copy(pin, "pin")}
                      className="text-[var(--color-text-muted)] hover:text-[var(--color-text)]"
                    >
                      {copied === "pin" ? <Check size={14} className="text-[var(--color-success)]" /> : <Copy size={14} />}
                    </button>
                  )}
                </div>
              </div>
              {qrDataUrl && (
                <img
                  src={qrDataUrl}
                  alt="Scan from phone to copy URL + PIN"
                  className="w-[160px] h-[160px] rounded-md border border-[var(--color-border)] bg-white"
                />
              )}
            </div>
          </Section>
        )}

        <VoiceSection />

      </div>
    </div>
  );
}

function VoiceSection() {
  const { voiceEngineOverride, setVoiceEngineOverride } = useApp();
  return (
    <Section title="Voice input" icon={<Mic size={14} />}>
      <p className="text-xs text-[var(--color-text-muted)] mb-3">
        Speech-to-text engine. <strong>Local pipeline</strong> uses sherpa-onnx with Whisper-tiny —
        all processing on your device. <strong>Browser</strong> uses your OS's Web Speech API
        (less private — Chrome streams audio to Google's servers).
      </p>
      <div className="flex gap-2">
        {(["sherpa", "web-speech"] as VoiceEngineKind[]).map((kind) => (
          <button
            key={kind}
            onClick={() => setVoiceEngineOverride(voiceEngineOverride === kind ? null : kind)}
            className={
              "px-3 py-1.5 rounded-md text-xs border " +
              (voiceEngineOverride === kind
                ? "bg-[var(--color-accent)]/20 border-[var(--color-accent)]"
                : "border-[var(--color-border)] hover:border-[var(--color-accent)]/40")
            }
          >
            {kind === "sherpa" ? "Local pipeline" : "Browser (Web Speech)"}
            {voiceEngineOverride === kind ? " · forced" : ""}
          </button>
        ))}
      </div>
    </Section>
  );
}

function Section({ title, icon, children }: { title: string; icon: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="mb-6 rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] p-4">
      <div className="flex items-center gap-2 mb-3">
        <span className="text-[var(--color-text-muted)]">{icon}</span>
        <h2 className="font-semibold text-sm">{title}</h2>
      </div>
      {children}
    </div>
  );
}

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="grid grid-cols-2 gap-y-2 text-sm">
      <div className="text-[var(--color-text-muted)]">{k}</div>
      <div className="text-right break-all">{v}</div>
    </div>
  );
}

function describeAcc(a: any): string {
  switch (a.type) {
    case "appleSilicon": return `${a.chip} (Apple Silicon, Metal)`;
    case "nvidia": return `${a.name} · ${a.vramGb.toFixed(1)}GB VRAM (CUDA)`;
    case "amd": return `${a.name} (AMD, Vulkan)`;
    case "intelArc": return `${a.name} (Intel Arc)`;
    case "cpu": return "CPU only";
    default: return "Unknown";
  }
}
